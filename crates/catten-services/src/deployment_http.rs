//! Bounded, fallible deployment ingress framing, before signature validation.

use alloc::vec::Vec;

use crate::clusterctl;

pub const HEADER_LIMIT: usize = 1024;
pub const MAX_REQUEST_BYTES: usize =
    HEADER_LIMIT + charlotte_launch::operations_bundle::MAX_BUNDLE_LEN;

#[derive(Debug, Eq, PartialEq)]
pub enum Request {
    Notify(Vec<u8>),
    Release(Vec<u8>),
    Operations(Vec<u8>),
    Shutdown(Vec<u8>),
    IngressPolicy(Vec<u8>),
    IngressPolicyStatus,
    Status(Vec<u8>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Malformed,
    TooLarge,
    AllocationFailed,
}

/// Owns the sole request allocation. Completed POST bodies reuse that backing.
#[derive(Default)]
pub struct RequestBuffer {
    bytes: Vec<u8>,
}

impl RequestBuffer {
    pub fn has_header(&self) -> bool {
        header_end(&self.bytes).is_some()
    }

    /// Allocation rejection preserves the existing prefix. Malformed input
    /// must be discarded by dropping this owner, rather than retried.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Option<Request>, Error> {
        let length = self.bytes.len().checked_add(bytes.len()).ok_or(Error::TooLarge)?;
        if length > MAX_REQUEST_BYTES {
            return Err(Error::TooLarge);
        }
        self.bytes.try_reserve_exact(bytes.len()).map_err(|_| Error::AllocationFailed)?;
        self.bytes.extend_from_slice(bytes);
        let Some(head) = parse(&self.bytes)? else {
            return Ok(None);
        };
        let mut backing = core::mem::take(&mut self.bytes);
        Ok(Some(match head {
            Head::Ready(request) => request,
            Head::Body(route, start) => {
                // Remove the header in place; no second attacker-sized copy.
                backing.drain(..start);
                route.finish(backing)
            }
        }))
    }
}

enum Head {
    Ready(Request),
    Body(Route, usize),
}

enum Route {
    Notify,
    Release,
    Operations,
    Shutdown,
    IngressPolicy,
}

impl Route {
    fn limits(&self) -> (usize, usize) {
        use charlotte_launch::{
            deployment,
            ingress_policy,
            operations_bundle,
            release,
            shutdown,
        };
        match self {
            Self::Notify => (deployment::MIN_HEADER_LEN, deployment::MAX_DESCRIPTOR_LEN),
            Self::Release => (release::HEADER_LEN, release::MAX_RELEASE_LEN),
            Self::Operations => (operations_bundle::HEADER_LEN, operations_bundle::MAX_BUNDLE_LEN),
            Self::Shutdown => (shutdown::ENCODED_LEN, shutdown::ENCODED_LEN),
            Self::IngressPolicy => (ingress_policy::HEADER_LEN, ingress_policy::MAX_ENCODED_LEN),
        }
    }

    fn finish(self, body: Vec<u8>) -> Request {
        match self {
            Self::Notify => Request::Notify(body),
            Self::Release => Request::Release(body),
            Self::Operations => Request::Operations(body),
            Self::Shutdown => Request::Shutdown(body),
            Self::IngressPolicy => Request::IngressPolicy(body),
        }
    }
}

fn header_end(bytes: &[u8]) -> Option<usize> {
    bytes[..bytes.len().min(HEADER_LIMIT)]
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|offset| offset + 4)
}

fn content_length(headers: &[u8]) -> Result<Option<usize>, Error> {
    let mut length = None;
    for raw_line in headers.split(|byte| *byte == b'\n').skip(1) {
        if raw_line.is_empty() || raw_line == b"\r" {
            continue;
        }
        let line = raw_line.strip_suffix(b"\r").ok_or(Error::Malformed)?;
        let colon = line.iter().position(|byte| *byte == b':').ok_or(Error::Malformed)?;
        let name = &line[..colon];
        if name.is_empty()
            || !name
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(byte))
        {
            return Err(Error::Malformed);
        }
        if name.eq_ignore_ascii_case(b"transfer-encoding") {
            return Err(Error::Malformed);
        }
        if name.eq_ignore_ascii_case(b"content-length") {
            if length.is_some() {
                return Err(Error::Malformed);
            }
            let value = line[colon + 1..].trim_ascii();
            if value.is_empty() {
                return Err(Error::Malformed);
            }
            let mut number = 0usize;
            for byte in value {
                if !byte.is_ascii_digit() {
                    return Err(Error::Malformed);
                }
                number = number
                    .checked_mul(10)
                    .and_then(|number| number.checked_add(usize::from(byte - b'0')))
                    .ok_or(Error::Malformed)?;
            }
            length = Some(number);
        }
    }
    Ok(length)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_path_name(path: &[u8]) -> Result<Vec<u8>, Error> {
    let prefix = clusterctl::NOTIFY_PATH;
    if !path.starts_with(prefix) || path.get(prefix.len()) != Some(&b'/') {
        return Err(Error::Malformed);
    }
    let encoded = &path[prefix.len() + 1..];
    let mut name = Vec::new();
    name.try_reserve_exact(encoded.len()).map_err(|_| Error::AllocationFailed)?;
    let mut index = 0;
    while index < encoded.len() {
        if encoded[index] == b'%' {
            let high = hex_digit(*encoded.get(index + 1).ok_or(Error::Malformed)?)
                .ok_or(Error::Malformed)?;
            let low = hex_digit(*encoded.get(index + 2).ok_or(Error::Malformed)?)
                .ok_or(Error::Malformed)?;
            name.push((high << 4) | low);
            index += 3;
        } else {
            name.push(encoded[index]);
            index += 1;
        }
    }
    charlotte_launch::deployment::valid_artifact_name(&name).then_some(name).ok_or(Error::Malformed)
}

fn parse(request: &[u8]) -> Result<Option<Head>, Error> {
    let Some(start) = header_end(request) else {
        return if request.len() <= HEADER_LIMIT {
            Ok(None)
        } else {
            Err(Error::TooLarge)
        };
    };
    let line_end = request.iter().position(|byte| *byte == b'\r').ok_or(Error::Malformed)?;
    let mut words = request[..line_end].split(|byte| *byte == b' ');
    let method = words.next().ok_or(Error::Malformed)?;
    let path = words.next().ok_or(Error::Malformed)?;
    if !matches!(words.next(), Some(b"HTTP/1.0" | b"HTTP/1.1")) || words.next().is_some() {
        return Err(Error::Malformed);
    }
    let length = content_length(&request[..start])?;
    if method == b"GET" {
        if length.is_some_and(|length| length != 0) || request.len() != start {
            return Err(Error::Malformed);
        }
        return Ok(Some(Head::Ready(
            if path == clusterctl::INGRESS_POLICY_PATH {
                Request::IngressPolicyStatus
            } else {
                Request::Status(decode_path_name(path)?)
            },
        )));
    }
    if method != b"POST" {
        return Err(Error::Malformed);
    }
    let route = match path {
        clusterctl::NOTIFY_PATH => Route::Notify,
        clusterctl::RELEASE_PATH => Route::Release,
        clusterctl::OPERATIONS_PATH => Route::Operations,
        clusterctl::SHUTDOWN_PATH => Route::Shutdown,
        clusterctl::INGRESS_POLICY_PATH => Route::IngressPolicy,
        _ => return Err(Error::Malformed),
    };
    let length = length.ok_or(Error::Malformed)?;
    let (minimum, maximum) = route.limits();
    if !(minimum..=maximum).contains(&length) {
        return Err(Error::TooLarge);
    }
    let end = start.checked_add(length).ok_or(Error::TooLarge)?;
    match request.len().cmp(&end) {
        core::cmp::Ordering::Less => Ok(None),
        core::cmp::Ordering::Equal => Ok(Some(Head::Body(route, start))),
        core::cmp::Ordering::Greater => Err(Error::Malformed),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use alloc::{
        format,
        vec,
    };
    use std::{
        alloc::{
            GlobalAlloc,
            Layout,
            System,
        },
        cell::Cell,
    };

    use super::*;

    std::thread_local! { static FAIL_NEXT: Cell<bool> = const { Cell::new(false) }; }
    struct TestAllocator;
    #[global_allocator]
    static ALLOCATOR: TestAllocator = TestAllocator;
    unsafe impl GlobalAlloc for TestAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            if FAIL_NEXT.try_with(|flag| flag.replace(false)).unwrap_or(false) {
                core::ptr::null_mut()
            } else {
                unsafe { System.alloc(layout) }
            }
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            if FAIL_NEXT.try_with(|flag| flag.replace(false)).unwrap_or(false) {
                core::ptr::null_mut()
            } else {
                unsafe { System.realloc(pointer, layout, size) }
            }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            unsafe { System.dealloc(pointer, layout) }
        }
    }

    #[test]
    fn fragmented_body_reuses_request_backing() {
        let length = charlotte_launch::operations_bundle::MAX_BUNDLE_LEN;
        let header = format!("POST /v1/operations HTTP/1.1\r\nContent-Length: {length}\r\n\r\n");
        let mut buffer = RequestBuffer::default();
        assert_eq!(buffer.push(header.as_bytes()), Ok(None));
        let bytes = vec![0xa5; length];
        assert_eq!(buffer.push(&bytes[..length - 1]), Ok(None));
        // Reserve final growth before capture; completion must not copy it.
        buffer.bytes.try_reserve_exact(1).unwrap();
        let original = buffer.bytes.as_ptr();
        let Some(Request::Operations(body)) = buffer.push(&bytes[length - 1..]).unwrap() else {
            panic!("complete operations body");
        };
        assert_eq!(body.as_ptr(), original);
        assert_eq!(body, bytes);
        assert!(buffer.bytes.is_empty());
    }

    #[test]
    fn allocation_rejection_preserves_prefix_and_allows_owned_retry() {
        let mut buffer = RequestBuffer::default();
        assert_eq!(buffer.push(b"GET /v1/deployments/"), Ok(None));
        FAIL_NEXT.with(|flag| flag.set(true));
        assert_eq!(buffer.push(b"hello HTTP/1.1\r\n\r\n"), Err(Error::AllocationFailed));
        assert_eq!(buffer.bytes, b"GET /v1/deployments/");
        assert_eq!(
            buffer.push(b"hello HTTP/1.1\r\n\r\n"),
            Ok(Some(Request::Status(b"hello".to_vec())))
        );
        FAIL_NEXT.with(|flag| flag.set(true));
        assert_eq!(decode_path_name(b"/v1/deployments/hello"), Err(Error::AllocationFailed));
    }

    #[test]
    fn ambiguous_framing_and_versions_reject_before_body_wait() {
        for extra in [
            "Content-Length: 100\r\nContent-Length: 100\r\n",
            "Content-Length: 100\r\nTransfer-Encoding: chunked\r\n",
            "Content-Length: 100\r\nTransfer-Encoding : chunked\r\n",
            "Content-Length : 100\r\n",
            "Content-Length: 100 junk\r\n",
            "Content-Length: \r\n",
            "Content-Length: 999999999999999999999999999999999999\r\n",
        ] {
            let bytes = format!("POST /v1/operations HTTP/1.1\r\n{extra}\r\n");
            assert_eq!(RequestBuffer::default().push(bytes.as_bytes()), Err(Error::Malformed));
        }
        for bytes in [
            b"GET /v1/ingress-policy HTTP/1.9\r\n\r\n".as_slice(),
            b"GET /v1/ingress-policy HTTP/1.1\r\nContent-Length: 1\r\n\r\nx",
            b"GET /v1/ingress-policy HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
        ] {
            assert_eq!(RequestBuffer::default().push(bytes), Err(Error::Malformed));
        }
    }

    #[test]
    fn limits_and_trailing_bytes_are_checked_without_body_copy() {
        assert_eq!(
            RequestBuffer::default().push(&vec![b'x'; HEADER_LIMIT + 1]),
            Err(Error::TooLarge)
        );
        assert_eq!(
            RequestBuffer::default().push(&vec![b'x'; MAX_REQUEST_BYTES + 1]),
            Err(Error::TooLarge)
        );
        let mut request = format!(
            "POST /v1/shutdowns HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            charlotte_launch::shutdown::ENCODED_LEN
        )
        .into_bytes();
        request.extend(vec![0; charlotte_launch::shutdown::ENCODED_LEN + 1]);
        assert_eq!(RequestBuffer::default().push(&request), Err(Error::Malformed));
        assert_eq!(
            RequestBuffer::default()
                .push(b"POST /v1/operations HTTP/1.1\r\nContent-Length: 9999999\r\n\r\n"),
            Err(Error::TooLarge)
        );
    }

    #[test]
    fn fragmented_headers_and_percent_encoded_names_complete() {
        let mut buffer = RequestBuffer::default();
        assert_eq!(
            buffer.push(b"GET /v1/deployments/hel%6co HTTP/1.0\r\nHost: localhost\r\n\r"),
            Ok(None)
        );
        assert!(!buffer.has_header());
        assert_eq!(buffer.push(b"\n"), Ok(Some(Request::Status(b"hello".to_vec()))));
        assert!(decode_path_name(b"/v1/deployments/%00").is_err());
        assert!(decode_path_name(b"/v1/deployments/bad%7").is_err());
    }
}
