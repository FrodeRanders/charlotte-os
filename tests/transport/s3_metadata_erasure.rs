//! Metadata ranges borrow a single consumed response owner.
use core::hint::black_box;

use charlotte_s3::http::{
    Error,
    MAX_RESPONSE_HEAD,
    ResponseHead,
    ResponseHeadBuffer,
};
use zeroize::Zeroizing;
mod support;
use support::observe;

fn parse(bytes: Zeroizing<Vec<u8>>) -> Result<ResponseHead, Error> {
    ResponseHead::parse(bytes)
}
fn frame(bytes: &[u8]) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(bytes.to_vec())
}
fn observe_parse(input: &[u8], expected_error: Option<Error>) {
    let bytes = frame(input);
    let pointer = bytes.as_ptr().addr();
    let offset = bytes.windows(8).position(|b| b == b"ZZZZZZZZ").unwrap();
    let mut error = None;
    let observed = observe(pointer, offset, 0, || match parse(bytes) {
        Ok(head) => drop(black_box(head)),
        Err(e) => error = Some(e),
    });
    assert_eq!(error, expected_error);
    assert!(observed.input_erased, "response owner retained: {observed:?}");
    assert_eq!(observed.input_releases, 1);
    assert_eq!(observed.allocations, 0, "metadata copy allocated: {observed:?}");
    assert_eq!(observed.copy_releases, 0);
}
#[test]
fn etag_erases_without_copy() {
    observe_parse(b"HTTP/1.1 200 OK\r\nETag: \"ZZZZZZZZ\"\r\n\r\n", None);
}
#[test]
fn version_id_erases_without_copy() {
    observe_parse(b"HTTP/1.1 200 OK\r\nx-amz-version-id: ZZZZZZZZ\r\n\r\n", None);
}
#[test]
fn request_id_erases_without_copy() {
    observe_parse(b"HTTP/1.1 200 OK\r\nx-amz-request-id: ZZZZZZZZ\r\n\r\n", None);
}
#[test]
fn late_rejection_erases_without_copy() {
    for (bytes,error) in [
 (b"HTTP/1.1 200 OK\r\nETag: ZZZZZZZZ\r\ninvalid\r\n\r\n".as_slice(),Error::InvalidHeader),
 (b"HTTP/1.1 200 OK\r\nETag: ZZZZZZZZ\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n".as_slice(),Error::ConflictingLength),
 (b"HTTP/1.1 200 OK\r\nETag: ZZZZZZZZ\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n".as_slice(),Error::ConflictingLength),
 (b"HTTP/1.1 200 OK\r\nETag: ZZZZZZZZ\r\nTransfer-Encoding: gzip\r\n\r\n".as_slice(),Error::UnsupportedTransferEncoding),
 ]{observe_parse(bytes,Some(error));}
}
#[test]
fn replacement_erases_without_copy() {
    let bytes = frame(b"HTTP/1.1 200 OK\r\nETag: ZZZZZZZZ\r\nETag: YYYYYYYY\r\n\r\n");
    let pointer = bytes.as_ptr().addr();
    let offset = bytes.windows(8).position(|b| b == b"ZZZZZZZZ").unwrap();
    let mut valid = false;
    let observed = observe(pointer, offset, 0, || {
        if let Ok(head) = parse(bytes) {
            valid = head.etag() == Some("YYYYYYYY");
            drop(black_box(head));
        }
    });
    assert!(valid);
    assert!(observed.input_erased);
    assert_eq!(observed.input_releases, 1);
    assert_eq!(observed.allocations, 0, "replacement copied metadata: {observed:?}");
    assert_eq!(observed.copy_releases, 0);
}
#[test]
fn ranges_preserve_trimming_aliases_unicode_and_binary_body() {
    let bytes=b"HTTP/1.1 206 Partial Content\r\nX-Unrelated: \xe9\x9b\xaa\r\neTAG: \t\"\"\xe9\x9b\xaa value\"\" \t\r\nX-Amz-Version-Id: \tversion \xe9\x9b\xaa\t \r\nX-Amz-Request-Id: original\r\nx-emc-request-id: replacement\r\nContent-Length: 2\r\nConnection: close\r\n\r\n\xff\0";
    let head = parse(frame(bytes)).unwrap();
    assert_eq!(head.status, 206);
    assert_eq!(head.content_length, Some(2));
    assert!(head.connection_close);
    assert_eq!(head.etag(), Some("雪 value"));
    assert_eq!(head.version_id(), Some("version 雪"));
    assert_eq!(head.request_id(), Some("replacement"));
    assert_eq!(head.body_prefix(), b"\xff\0");
    for value in ["", " \t ", "\"\"", "\"\"\"\""] {
        let text = format!("HTTP/1.1 200 OK\r\nETag:{value}\r\n\r\n");
        let head = parse(frame(text.as_bytes())).unwrap();
        assert_eq!(head.etag(), Some(""));
        assert_eq!(head.version_id(), None);
        assert_eq!(head.request_id(), None);
    }
}
#[test]
fn all_header_splits_keep_metadata_after_accumulator_and_chunks_drop() {
    let bytes=b"HTTP/1.1 200 OK\r\nETag: \"ZZZZZZZZ\"\r\nx-amz-version-id: version\r\nx-emc-request-id: request\r\n\r\n";
    for split in 1..bytes.len() {
        let (head, body) = {
            let mut buffer = ResponseHeadBuffer::new();
            let first = frame(&bytes[..split]);
            assert!(buffer.push(&first).unwrap().is_none());
            let mut last = frame(&bytes[split..]);
            last.extend_from_slice(b"body");
            buffer.push(&last).unwrap().unwrap()
        };
        assert_eq!(head.etag(), Some("ZZZZZZZZ"));
        assert_eq!(head.version_id(), Some("version"));
        assert_eq!(head.request_id(), Some("request"));
        assert_eq!(body.as_slice(), b"body");
        assert!(head.body_prefix().is_empty());
    }
}
#[test]
fn parse_errors_keep_the_existing_status_and_consume_input() {
    for (bytes, error) in [
        (b"HTTP/1.1 200 OK\r\n".as_slice(), Error::Incomplete),
        (b"HTTP/1.0 200 OK\r\n\r\n".as_slice(), Error::InvalidStatus),
        (b"HTTP/1.1 200 OK\r\nX-Bad: \xff\r\n\r\n".as_slice(), Error::InvalidHeader),
    ] {
        let bytes = frame(bytes);
        let pointer = bytes.as_ptr().addr();
        let mut result = None;
        let observed = observe(pointer, 0, 0, || {
            result = parse(bytes).err();
        });
        assert_eq!(result, Some(error));
        assert!(observed.input_erased);
        assert_eq!(observed.input_releases, 1);
        assert_eq!(observed.allocations, 0);
    }
    let full = Zeroizing::new(vec![b'Z'; MAX_RESPONSE_HEAD]);
    assert_eq!(parse(full).err(), Some(Error::Incomplete));
    let excess = Zeroizing::new(vec![b'Z'; MAX_RESPONSE_HEAD + 1]);
    assert_eq!(parse(excess).err(), Some(Error::HeadTooLarge));
}
