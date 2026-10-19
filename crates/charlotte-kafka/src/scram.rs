//! Bounded SCRAM-SHA-256 client state for Kafka SASL authentication.

use alloc::{
    string::String,
    vec,
    vec::Vec,
};

use charlotte_launch::sha256::{
    digest,
    hmac_sha256,
};
use zeroize::Zeroizing;

pub const MECHANISM: &[u8] = b"SCRAM-SHA-256";
pub const MIN_ITERATIONS: u32 = 4_096;
/// Upper bound on the server-advertised PBKDF2 iteration count. A malicious
/// broker could otherwise force a million synchronous SHA-256 rounds per
/// authentication attempt. This stays above the counts used by common Kafka
/// and MongoDB deployments.
pub const MAX_ITERATIONS: u32 = 200_000;
const MAX_SERVER_MESSAGE_BYTES: usize = 4_096;
const MAX_SALT_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidCredential,
    InvalidNonce,
    InvalidServerMessage,
    UnsupportedExtension,
    IterationLimit,
    ServerRejected,
    SignatureMismatch,
}

/// Initial SCRAM phase. Consuming the server challenge releases the password
/// copy on success or rejection. The returned phase owns only the proof and
/// expected server signature.
///
/// A rejected challenge consumes this phase too:
/// ```compile_fail,E0382
/// use charlotte_kafka::scram::Client;
/// let client = Client::new("user", b"public-fixture", "nonce").unwrap();
/// let _ = client.receive_server_first(b"malformed");
/// let _ = client.client_first(); // The initial phase was consumed.
/// ```
pub struct Client {
    password: Zeroizing<Vec<u8>>,
    nonce: String,
    client_first_bare: Zeroizing<Vec<u8>>,
}

/// A prepared proof and one server-final verification. This owner contains no
/// password or initial transcript and cannot begin another exchange.
///
/// Success and every rejection consume the verifier:
/// ```compile_fail,E0382
/// use charlotte_kafka::scram::Client;
/// let initial = Client::new("user", b"public-fixture", "client").unwrap();
/// let final_phase = initial
///     .receive_server_first(b"r=client-server,s=c2FsdA==,i=4096").unwrap();
/// let _ = final_phase.receive_server_final(b"malformed");
/// let _ = final_phase.receive_server_final(b"v=another"); // Already consumed.
/// ```
pub struct ClientFinal {
    message: Zeroizing<Vec<u8>>,
    expected_server_signature: Zeroizing<[u8; 32]>,
}

impl Client {
    pub fn new(username: &str, password: &[u8], nonce: &str) -> Result<Self, Error> {
        if username.is_empty()
            || password.is_empty()
            || nonce.is_empty()
            || !nonce.bytes().all(|byte| (0x21..=0x7e).contains(&byte) && byte != b',')
        {
            return Err(Error::InvalidCredential);
        }
        let escaped = escape_username(username);
        let mut client_first_bare =
            Zeroizing::new(Vec::with_capacity(2 + escaped.len() + 3 + nonce.len()));
        client_first_bare.extend_from_slice(b"n=");
        client_first_bare.extend_from_slice(escaped.as_bytes());
        client_first_bare.extend_from_slice(b",r=");
        client_first_bare.extend_from_slice(nonce.as_bytes());
        Ok(Self {
            password: Zeroizing::new(password.to_vec()),
            nonce: String::from(nonce),
            client_first_bare,
        })
    }

    pub fn client_first(&self) -> Zeroizing<Vec<u8>> {
        let mut message = Zeroizing::new(Vec::with_capacity(3 + self.client_first_bare.len()));
        message.extend_from_slice(b"n,,");
        message.extend_from_slice(&self.client_first_bare);
        message
    }

    pub fn receive_server_first(self, message: &[u8]) -> Result<ClientFinal, Error> {
        if message.is_empty() || message.len() > MAX_SERVER_MESSAGE_BYTES {
            return Err(Error::InvalidServerMessage);
        }
        let text = core::str::from_utf8(message).map_err(|_| Error::InvalidServerMessage)?;
        let mut combined_nonce = None;
        let mut salt = None;
        let mut iterations = None;
        for attribute in text.split(',') {
            let (name, value) = attribute.split_once('=').ok_or(Error::InvalidServerMessage)?;
            match name {
                "r" if combined_nonce.is_none() => combined_nonce = Some(value),
                "s" if salt.is_none() => salt = Some(value),
                "i" if iterations.is_none() => {
                    iterations =
                        Some(value.parse::<u32>().map_err(|_| Error::InvalidServerMessage)?);
                }
                "m" => return Err(Error::UnsupportedExtension),
                _ => return Err(Error::InvalidServerMessage),
            }
        }
        let combined_nonce = combined_nonce.ok_or(Error::InvalidServerMessage)?;
        if !combined_nonce.starts_with(&self.nonce)
            || combined_nonce.len() <= self.nonce.len()
            || !combined_nonce.bytes().all(|byte| (0x21..=0x7e).contains(&byte) && byte != b',')
        {
            return Err(Error::InvalidNonce);
        }
        let iterations = iterations.ok_or(Error::InvalidServerMessage)?;
        if !(MIN_ITERATIONS..=MAX_ITERATIONS).contains(&iterations) {
            return Err(Error::IterationLimit);
        }
        let salt = base64_decode(salt.ok_or(Error::InvalidServerMessage)?)?;
        if salt.is_empty() || salt.len() > MAX_SALT_BYTES {
            return Err(Error::InvalidServerMessage);
        }

        let mut final_without_proof = Zeroizing::new(Vec::with_capacity(9 + combined_nonce.len()));
        final_without_proof.extend_from_slice(b"c=biws,r=");
        final_without_proof.extend_from_slice(combined_nonce.as_bytes());

        let mut auth_message = Zeroizing::new(Vec::with_capacity(
            self.client_first_bare.len() + message.len() + final_without_proof.len() + 2,
        ));
        auth_message.extend_from_slice(&self.client_first_bare);
        auth_message.push(b',');
        auth_message.extend_from_slice(message);
        auth_message.push(b',');
        auth_message.extend_from_slice(&final_without_proof);

        let salted_password = pbkdf2_sha256(&self.password, &salt, iterations);
        let client_key = hmac_sha256(&salted_password[..], b"Client Key");
        let stored_key = Zeroizing::new(digest(&client_key[..]));
        let client_signature = hmac_sha256(&stored_key[..], &auth_message);
        let mut proof = Zeroizing::new([0u8; 32]);
        for index in 0..proof.len() {
            proof[index] = client_key[index] ^ client_signature[index];
        }
        let server_key = hmac_sha256(&salted_password[..], b"Server Key");
        let expected_server_signature = hmac_sha256(&server_key[..], &auth_message);

        let proof = Zeroizing::new(base64_encode(&proof[..], true));
        let mut client_final =
            Zeroizing::new(Vec::with_capacity(final_without_proof.len() + 3 + proof.len()));
        client_final.extend_from_slice(&final_without_proof);
        client_final.extend_from_slice(b",p=");
        client_final.extend_from_slice(proof.as_bytes());
        Ok(ClientFinal {
            message: client_final,
            expected_server_signature,
        })
    }
}

impl ClientFinal {
    /// Borrow the proof bytes while this phase owns them. Request encoding
    /// must copy them into its own zeroizing transport buffer.
    pub fn message(&self) -> &[u8] {
        &self.message
    }

    pub fn receive_server_final(self, message: &[u8]) -> Result<(), Error> {
        if message.is_empty() || message.len() > MAX_SERVER_MESSAGE_BYTES {
            return Err(Error::InvalidServerMessage);
        }
        let expected = &self.expected_server_signature;
        let text = core::str::from_utf8(message).map_err(|_| Error::InvalidServerMessage)?;
        if text.starts_with("e=") {
            return Err(Error::ServerRejected);
        }
        let encoded = text.strip_prefix("v=").ok_or(Error::InvalidServerMessage)?;
        if encoded.contains(',') {
            return Err(Error::InvalidServerMessage);
        }
        let signature = Zeroizing::new(base64_decode(encoded)?);
        if signature.len() != expected.len() || !constant_time_eq(&signature, &expected[..]) {
            return Err(Error::SignatureMismatch);
        }
        Ok(())
    }
}

fn escape_username(username: &str) -> String {
    let mut escaped = String::with_capacity(username.len());
    for character in username.chars() {
        match character {
            ',' => escaped.push_str("=2C"),
            '=' => escaped.push_str("=3D"),
            _ => escaped.push(character),
        }
    }
    escaped
}

fn pbkdf2_sha256(password: &[u8], salt: &[u8], iterations: u32) -> Zeroizing<[u8; 32]> {
    let mut salt_block = Zeroizing::new(Vec::with_capacity(salt.len() + 4));
    salt_block.extend_from_slice(salt);
    salt_block.extend_from_slice(&1u32.to_be_bytes());
    let mut current = hmac_sha256(password, &salt_block);
    let mut result = Zeroizing::new(*current);
    for _ in 1..iterations {
        current = hmac_sha256(password, &current[..]);
        for index in 0..result.len() {
            result[index] ^= current[index];
        }
    }
    result
}

pub fn base64_encode(bytes: &[u8], padding: bool) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = u32::from(chunk[0]) << 16
            | u32::from(*chunk.get(1).unwrap_or(&0)) << 8
            | u32::from(*chunk.get(2).unwrap_or(&0));
        result.push(ALPHABET[((value >> 18) & 0x3f) as usize] as char);
        result.push(ALPHABET[((value >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((value >> 6) & 0x3f) as usize] as char);
        } else if padding {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(value & 0x3f) as usize] as char);
        } else if padding {
            result.push('=');
        }
    }
    result
}

fn base64_decode(value: &str) -> Result<Vec<u8>, Error> {
    if value.is_empty() || value.len() % 4 == 1 {
        return Err(Error::InvalidServerMessage);
    }
    let mut output = vec![];
    let bytes = value.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        let count = remaining.min(4);
        let mut digits = [0u8; 4];
        let mut padding = 0;
        for index in 0..count {
            if bytes[offset + index] == b'=' {
                if offset + count != bytes.len()
                    || !bytes[offset + index..offset + count].iter().all(|byte| *byte == b'=')
                {
                    return Err(Error::InvalidServerMessage);
                }
                padding = count - index;
                break;
            } else {
                digits[index] = base64_digit(bytes[offset + index])?;
            }
        }
        if count < 2 || padding > 2 || count < 4 && padding != 0 {
            return Err(Error::InvalidServerMessage);
        }
        let packed = u32::from(digits[0]) << 18
            | u32::from(digits[1]) << 12
            | u32::from(digits[2]) << 6
            | u32::from(digits[3]);
        output.push((packed >> 16) as u8);
        if count > 2 && padding < 2 {
            output.push((packed >> 8) as u8);
        }
        if count > 3 && padding == 0 {
            output.push(packed as u8);
        }
        offset += count;
    }
    Ok(output)
}

fn base64_digit(byte: u8) -> Result<u8, Error> {
    match byte {
        b'A'..=b'Z' => Ok(byte - b'A'),
        b'a'..=b'z' => Ok(byte - b'a' + 26),
        b'0'..=b'9' => Ok(byte - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        _ => Err(Error::InvalidServerMessage),
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    // Length inequality is not secret here (both sides are fixed-size
    // digests), but it must be rejected explicitly: zipping mismatched slices
    // would otherwise compare only the common prefix.
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip_and_padding() {
        for value in [b"f".as_slice(), b"fo", b"foo", b"SCRAM-SHA-256"] {
            let encoded = base64_encode(value, true);
            assert_eq!(base64_decode(&encoded).unwrap(), value);
        }
        assert_eq!(base64_encode(b"foo", false), "Zm9v");
    }

    #[test]
    fn scram_sha_256_matches_rfc_7677_exchange() {
        let client = Client::new("user", b"pencil", "rOprNGfwEbeRWgbNEkqO").unwrap();
        assert_eq!(&*client.client_first(), b"n,,n=user,r=rOprNGfwEbeRWgbNEkqO");
        let final_message = client
            .receive_server_first(
                b"r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096",
            )
            .unwrap();
        assert_eq!(
            final_message.message(),
            b"c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
        );
        final_message
            .receive_server_final(b"v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=")
            .unwrap();
    }

    #[test]
    fn rejects_nonce_downgrade_and_excessive_work() {
        for (message, expected) in [
            (b"r=other,s=c2FsdA==,i=4096".as_slice(), Error::InvalidNonce),
            (b"r=client-server,s=c2FsdA==,i=1000001", Error::IterationLimit),
        ] {
            let client = Client::new("user", b"public-fixture", "client").unwrap();
            assert_eq!(client.receive_server_first(message).err(), Some(expected));
        }
    }

    #[test]
    fn rejects_malformed_challenges_with_one_consumed_initial_owner() {
        for (message, expected) in [
            (b"".as_slice(), Error::InvalidServerMessage),
            (b"\xff", Error::InvalidServerMessage),
            (b"m=extension", Error::UnsupportedExtension),
            (b"r=client-server,r=client-server,s=c2FsdA==,i=4096", Error::InvalidServerMessage),
            (b"r=client-server,s=!,i=4096", Error::InvalidServerMessage),
            (b"r=client-server,s=,i=4096", Error::InvalidServerMessage),
            (b"r=client-server,s=c2FsdA==,i=0", Error::IterationLimit),
        ] {
            let client = Client::new("user", b"public-fixture", "client").unwrap();
            assert_eq!(client.receive_server_first(message).err(), Some(expected));
        }
        let client = Client::new("user", b"public-fixture", "client").unwrap();
        assert_eq!(
            client.receive_server_first(&vec![b'x'; MAX_SERVER_MESSAGE_BYTES + 1]).err(),
            Some(Error::InvalidServerMessage)
        );
    }

    fn final_phase() -> ClientFinal {
        Client::new("user", b"pencil", "rOprNGfwEbeRWgbNEkqO").unwrap()
            .receive_server_first(b"r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096")
            .unwrap()
    }

    #[test]
    fn every_final_rejection_consumes_the_proof_and_verifier() {
        for (message, expected) in [
            (b"".as_slice(), Error::InvalidServerMessage),
            (b"\xff", Error::InvalidServerMessage),
            (b"e=invalid-proof", Error::ServerRejected),
            (b"v=!!!!", Error::InvalidServerMessage),
            (b"v=AAAA", Error::SignatureMismatch),
            (b"v=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", Error::SignatureMismatch),
            (
                b"v=6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4=,extra=x",
                Error::InvalidServerMessage,
            ),
        ] {
            assert_eq!(final_phase().receive_server_final(message), Err(expected));
        }
        assert_eq!(
            final_phase().receive_server_final(&vec![b'x'; MAX_SERVER_MESSAGE_BYTES + 1]),
            Err(Error::InvalidServerMessage)
        );
    }
}
