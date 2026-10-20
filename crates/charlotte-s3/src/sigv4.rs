//! AWS Signature Version 4 request signing.

use alloc::{
    format,
    string::String,
    vec::Vec,
};

use charlotte_launch::sha256::{
    digest,
    hmac_sha256,
};
use zeroize::Zeroizing;

/// SHA-256 of an empty payload, used by GET, HEAD, and DELETE requests.
pub const EMPTY_PAYLOAD_SHA256: &str =
    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidTimestamp,
    InvalidHeader,
    InvalidPath,
    TooLarge,
}

#[derive(Clone, Copy)]
pub struct Credentials<'a> {
    pub access_key: &'a str,
    pub secret_key: &'a [u8],
}

/// UTC fields used by SigV4. S3 signatures have whole-second precision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timestamp {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Timestamp {
    pub fn date(self) -> Result<String, Error> {
        self.validate()?;
        Ok(format!("{:04}{:02}{:02}", self.year, self.month, self.day))
    }

    pub fn amz_date(self) -> Result<String, Error> {
        self.validate()?;
        Ok(format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        ))
    }

    fn validate(self) -> Result<(), Error> {
        if !(1970..=9999).contains(&self.year)
            || !(1..=12).contains(&self.month)
            || !(1..=31).contains(&self.day)
            || self.day > days_in_month(self.year, self.month)
            || self.hour > 23
            || self.minute > 59
            || self.second > 59
        {
            return Err(Error::InvalidTimestamp);
        }
        Ok(())
    }
}

const fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

#[derive(Clone, Copy)]
pub struct Header<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

#[derive(Clone, Copy)]
pub struct Query<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

pub struct Request<'a> {
    pub method: &'a str,
    /// Unescaped absolute path, normally `/bucket/key` for path-style S3.
    pub path: &'a str,
    pub query: &'a [Query<'a>],
    /// Headers to sign. Names may use any ASCII case; values are
    /// whitespace-folded.
    /// `host`, `x-amz-content-sha256`, and `x-amz-date` must be present.
    pub headers: &'a [Header<'a>],
    pub payload_sha256: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    pub timestamp: Timestamp,
}

/// Signing output and canonical metadata retain erasing owners. Caller inputs
/// keep their separate lifetimes. The aggregate has no debug formatter.
///
/// ```compile_fail
/// fn requires_debug<T: core::fmt::Debug>() {}
/// requires_debug::<charlotte_s3::sigv4::Signature>();
/// ```
pub struct Signature {
    pub authorization: Zeroizing<String>,
    pub canonical_request: Zeroizing<String>,
    pub string_to_sign: Zeroizing<String>,
    pub signed_headers: Zeroizing<String>,
    pub signature: Zeroizing<String>,
}

pub fn sign(request: &Request<'_>, credentials: Credentials<'_>) -> Result<Signature, Error> {
    if request.method.is_empty()
        || request.region.is_empty()
        || request.service.is_empty()
        || request.headers.is_empty()
    {
        return Err(Error::InvalidHeader);
    }
    let date = request.timestamp.date()?;
    let amz_date = request.timestamp.amz_date()?;
    let canonical_uri = canonical_uri(request.path)?;
    let canonical_query = canonical_query(request.query)?;
    let (canonical_headers, signed_headers) = canonical_headers(request.headers)?;
    let canonical_request = concat_owned(&[
        request.method,
        "\n",
        &canonical_uri,
        "\n",
        &canonical_query,
        "\n",
        &canonical_headers,
        "\n",
        &signed_headers,
        "\n",
        request.payload_sha256,
    ])?;
    let scope = concat_owned(&[&date, "/", request.region, "/", request.service, "/aws4_request"])?;
    let canonical_digest = Zeroizing::new(digest(canonical_request.as_bytes()));
    let canonical_hash = hex_digest(&canonical_digest);
    let string_to_sign =
        concat_owned(&["AWS4-HMAC-SHA256\n", &amz_date, "\n", &scope, "\n", &canonical_hash])?;

    let mut first_key = Zeroizing::new(Vec::with_capacity(4 + credentials.secret_key.len()));
    first_key.extend_from_slice(b"AWS4");
    first_key.extend_from_slice(credentials.secret_key);
    let date_key = hmac_sha256(&first_key, date.as_bytes());
    let region_key = hmac_sha256(&*date_key, request.region.as_bytes());
    let service_key = hmac_sha256(&*region_key, request.service.as_bytes());
    let signing_key = hmac_sha256(&*service_key, b"aws4_request");
    let signature = hex_digest(&hmac_sha256(&*signing_key, string_to_sign.as_bytes()));
    let authorization = authorization(credentials.access_key, &scope, &signed_headers, &signature)?;
    Ok(Signature {
        authorization,
        canonical_request,
        string_to_sign,
        signed_headers,
        signature,
    })
}

fn hex_digest(bytes: &[u8; 32]) -> Zeroizing<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = Zeroizing::new(String::with_capacity(64));
    for byte in bytes {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 0x0f) as usize] as char);
    }
    text
}

fn authorization(
    access_key: &str,
    scope: &str,
    signed_headers: &str,
    signature: &str,
) -> Result<Zeroizing<String>, Error> {
    let parts = [
        "AWS4-HMAC-SHA256 Credential=",
        access_key,
        "/",
        scope,
        ", SignedHeaders=",
        signed_headers,
        ", Signature=",
        signature,
    ];
    concat_owned(&parts)
}

fn concat_owned(parts: &[&str]) -> Result<Zeroizing<String>, Error> {
    let len = parts
        .iter()
        .try_fold(0usize, |len, part| len.checked_add(part.len()))
        .ok_or(Error::TooLarge)?;
    let mut text = Zeroizing::new(String::with_capacity(len));
    for part in parts {
        text.push_str(part);
    }
    Ok(text)
}

/// S3 URI encoding: retain `/`, encode bytes outside the unreserved set, and
/// use uppercase hex. Input paths must be absolute.
pub fn canonical_uri(path: &str) -> Result<Zeroizing<String>, Error> {
    if !path.starts_with('/') {
        return Err(Error::InvalidPath);
    }
    uri_encode(path.as_bytes(), false)
}

pub fn canonical_query(query: &[Query<'_>]) -> Result<Zeroizing<String>, Error> {
    let mut encoded = Vec::with_capacity(query.len());
    for item in query {
        encoded.push((
            uri_encode(item.name.as_bytes(), true)?,
            uri_encode(item.value.as_bytes(), true)?,
        ));
    }
    encoded.sort_unstable_by(|left, right| {
        left.0.as_str().cmp(right.0.as_str()).then_with(|| left.1.as_str().cmp(right.1.as_str()))
    });
    let len = encoded
        .iter()
        .try_fold(0usize, |len, (name, value)| {
            len.checked_add(name.len())?.checked_add(1)?.checked_add(value.len())
        })
        .and_then(|len| len.checked_add(encoded.len().saturating_sub(1)))
        .ok_or(Error::TooLarge)?;
    let mut result = Zeroizing::new(String::with_capacity(len));
    for (index, (name, value)) in encoded.iter().enumerate() {
        if index != 0 {
            result.push('&');
        }
        result.push_str(name);
        result.push('=');
        result.push_str(value);
    }
    Ok(result)
}

type CanonicalHeaders = (Zeroizing<String>, Zeroizing<String>);

fn canonical_headers(headers: &[Header<'_>]) -> Result<CanonicalHeaders, Error> {
    // Reject invalid shapes before copying any header values. Normalized owners
    // still cover every later construction error and ordinary return.
    for (index, header) in headers.iter().enumerate() {
        if header.name.is_empty()
            || header.name.bytes().any(|byte| !byte.is_ascii_alphanumeric() && byte != b'-')
            || headers[..index].iter().any(|prior| prior.name.eq_ignore_ascii_case(header.name))
        {
            return Err(Error::InvalidHeader);
        }
    }
    for required in ["host", "x-amz-content-sha256", "x-amz-date"] {
        if !headers.iter().any(|header| header.name.eq_ignore_ascii_case(required)) {
            return Err(Error::InvalidHeader);
        }
    }
    let mut normalized = Vec::with_capacity(headers.len());
    for header in headers {
        let mut name = Zeroizing::new(String::with_capacity(header.name.len()));
        for byte in header.name.bytes() {
            name.push(byte.to_ascii_lowercase() as char);
        }
        normalized.push((name, fold_whitespace(header.value)?));
    }
    normalized.sort_unstable_by(|left, right| left.0.as_str().cmp(right.0.as_str()));
    let canonical_len = normalized
        .iter()
        .try_fold(0usize, |len, (name, value)| {
            len.checked_add(name.len())?.checked_add(1)?.checked_add(value.len())?.checked_add(1)
        })
        .ok_or(Error::TooLarge)?;
    let names_len = normalized
        .iter()
        .try_fold(0usize, |len, (name, _)| len.checked_add(name.len()))
        .and_then(|len| len.checked_add(normalized.len().saturating_sub(1)))
        .ok_or(Error::TooLarge)?;
    let mut canonical = Zeroizing::new(String::with_capacity(canonical_len));
    let mut names = Zeroizing::new(String::with_capacity(names_len));
    for (index, (name, value)) in normalized.iter().enumerate() {
        if index != 0 {
            names.push(';');
        }
        names.push_str(name);
        canonical.push_str(name);
        canonical.push(':');
        canonical.push_str(value);
        canonical.push('\n');
    }
    Ok((canonical, names))
}

fn fold_whitespace(value: &str) -> Result<Zeroizing<String>, Error> {
    let mut len = 0usize;
    for (index, part) in value.split_ascii_whitespace().enumerate() {
        len = len
            .checked_add(usize::from(index != 0))
            .and_then(|len| len.checked_add(part.len()))
            .ok_or(Error::TooLarge)?;
    }
    let mut result = Zeroizing::new(String::with_capacity(len));
    for (index, part) in value.split_ascii_whitespace().enumerate() {
        if index != 0 {
            result.push(' ');
        }
        result.push_str(part);
    }
    Ok(result)
}

fn uri_encode(bytes: &[u8], encode_slash: bool) -> Result<Zeroizing<String>, Error> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let unescaped = |byte: u8| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
            || (byte == b'/' && !encode_slash)
    };
    let len = bytes
        .iter()
        .try_fold(0usize, |len, byte| {
            len.checked_add(
                if unescaped(*byte) {
                    1
                } else {
                    3
                },
            )
        })
        .ok_or(Error::TooLarge)?;
    let mut result = Zeroizing::new(String::with_capacity(len));
    for byte in bytes {
        if unescaped(*byte) {
            result.push(*byte as char);
        } else {
            result.push('%');
            result.push(HEX[(byte >> 4) as usize] as char);
            result.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    Ok(result)
}

pub fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0x0f) as usize] as char);
    }
    result
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    fn hex_array(hex: &str) -> Vec<u8> {
        hex.as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| {
                let digit = |byte| match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    _ => panic!("bad hex"),
                };
                digit(pair[0]) << 4 | digit(pair[1])
            })
            .collect()
    }

    #[test]
    fn hmac_matches_rfc_4231_case_one() {
        let key = [0x0bu8; 20];
        assert_eq!(
            hmac_sha256(&key, b"Hi There").as_slice(),
            hex_array("b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7")
        );
    }

    #[test]
    fn s3_encoding_preserves_path_separators() {
        assert_eq!(canonical_uri("/my bucket/a+b/%").unwrap().as_str(), "/my%20bucket/a%2Bb/%25");
        assert_eq!(
            canonical_query(&[
                Query {
                    name: "prefix",
                    value: "a/b c"
                },
                Query {
                    name: "list-type",
                    value: "2"
                },
            ])
            .unwrap()
            .as_str(),
            "list-type=2&prefix=a%2Fb%20c"
        );
    }

    #[test]
    fn signs_a_deterministic_s3_get() {
        let headers = [
            Header {
                name: "host",
                value: "rustfs.test:9000",
            },
            Header {
                name: "x-amz-content-sha256",
                value: EMPTY_PAYLOAD_SHA256,
            },
            Header {
                name: "x-amz-date",
                value: "20260826T120102Z",
            },
        ];
        let signature = sign(
            &Request {
                method: "GET",
                path: "/test-bucket/reports/hello world.txt",
                query: &[],
                headers: &headers,
                payload_sha256: EMPTY_PAYLOAD_SHA256,
                region: "us-east-1",
                service: "s3",
                timestamp: Timestamp {
                    year: 2026,
                    month: 8,
                    day: 26,
                    hour: 12,
                    minute: 1,
                    second: 2,
                },
            },
            Credentials {
                access_key: "CHARLOTTE",
                secret_key: b"correct horse battery staple", // https://xkcd.com/936/ :)
            },
        )
        .unwrap();
        assert_eq!(
            signature.signature.as_str(),
            "34e97fe1de869a653fb4f8170b09370b65eda55bc5c5ba8ffd8b47100c677f68"
        );
        assert_eq!(signature.signed_headers.as_str(), "host;x-amz-content-sha256;x-amz-date");
    }

    #[test]
    fn canonicalizes_header_names_and_rejects_invalid_dates() {
        let headers = [
            Header {
                name: "Host",
                value: " example.test ",
            },
            Header {
                name: "X-Amz-Content-Sha256",
                value: EMPTY_PAYLOAD_SHA256,
            },
            Header {
                name: "X-Amz-Date",
                value: "20260228T120000Z",
            },
        ];
        let (canonical, names) = canonical_headers(&headers).unwrap();
        assert!(canonical.starts_with("host:example.test\n"));
        assert_eq!(names.as_str(), "host;x-amz-content-sha256;x-amz-date");
        assert_eq!(
            Timestamp {
                year: 2026,
                month: 2,
                day: 29,
                hour: 12,
                minute: 0,
                second: 0,
            }
            .amz_date(),
            Err(Error::InvalidTimestamp)
        );
    }
}
