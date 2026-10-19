//! Allocation and release checks for public HTTP fixture bytes.
use core::hint::black_box;

use charlotte_s3::http::{
    ChunkedDecoder,
    Error,
    MAX_RESPONSE_HEAD,
    ResponseHeadBuffer,
};

mod support;
use support::observe;

#[test]
fn fragmented_head_never_grows_and_erases_on_release() {
    let observed = observe(0, 0, 0, || {
        let mut buffer = ResponseHeadBuffer::new();
        assert!(buffer.push(b"HTTP/1.1 200 OK\r\n").unwrap().is_none());
        assert!(buffer.push(b"Content-Length: 0\r\n").unwrap().is_none());
        let (head, body) = buffer.push(b"\r\n").unwrap().unwrap();
        assert_eq!(head.content_length, Some(0));
        assert!(body.is_empty());
        drop(black_box(buffer));
    });
    assert!(observed.copy_erased, "header retained: {observed:?}");
    assert_eq!(observed.copy_releases, 1);
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0, "header grew: {observed:?}");
}

#[test]
fn malformed_and_abandoned_head_erase_their_existing_prefix() {
    for malformed in [false, true] {
        let observed = observe(0, 0, 0, || {
            let mut buffer = ResponseHeadBuffer::new();
            assert!(buffer.push(b"HTTP/1.1 200 OK\r\n").unwrap().is_none());
            if malformed {
                assert!(matches!(buffer.push(b"invalid\r\n\r\n"), Err(Error::InvalidHeader)));
                assert!(buffer.push(b"HTTP/1.1 200 OK\r\n\r\n").is_err());
            }
            drop(black_box(buffer));
        });
        assert!(observed.copy_erased, "header prefix retained: {observed:?}");
        assert_eq!(observed.copy_releases, 1);
        assert_eq!(observed.growths, 0);
    }
}

#[test]
fn decoded_output_never_grows_and_erases_on_release() {
    let mut decoder = ChunkedDecoder::new();
    let observed = observe(0, 0, 0, || {
        let bytes =
            decoder.decode(black_box(b"8\r\nZZZZZZZZ\r\n8\r\nZZZZZZZZ\r\n0\r\n\r\n")).unwrap();
        assert_eq!(bytes.as_slice(), b"ZZZZZZZZZZZZZZZZ");
        drop(black_box(bytes));
    });
    assert!(decoder.is_complete());
    assert!(observed.copy_erased, "body retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.copy_releases, 1);
    assert_eq!(observed.growths, 0, "decoded body grew: {observed:?}");
}

#[test]
fn malformed_chunk_erases_partial_output_and_fences_decoder() {
    let mut decoder = ChunkedDecoder::new();
    let observed = observe(0, 0, 0, || {
        assert_eq!(
            decoder.decode(black_box(b"8\r\nZZZZZZZZoops")).err(),
            Some(Error::InvalidChunk)
        );
    });
    assert!(observed.copy_erased, "rejected body retained: {observed:?}");
    assert_eq!(observed.copy_releases, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(decoder.decode(b"\r\n0\r\n\r\n").err(), Some(Error::InvalidChunk));
}

#[test]
fn head_delimiter_splits_preserve_body_and_terminal_fence() {
    let head = b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n";
    for split in 1..head.len() {
        let mut buffer = ResponseHeadBuffer::new();
        assert!(buffer.push(&head[..split]).unwrap().is_none());
        let mut last = head[split..].to_vec();
        last.extend_from_slice(b"ZZZZZZZZ");
        let (response, body) = buffer.push(&last).unwrap().unwrap();
        assert_eq!(response.content_length, Some(8));
        assert_eq!(body.as_slice(), b"ZZZZZZZZ");
        assert!(buffer.push(b"HTTP/1.1 200 OK\r\n\r\n").is_err());
    }
}

#[test]
fn head_limit_excludes_body_suffix_and_rejects_unterminated_header() {
    let mut head = b"HTTP/1.1 200 OK\r\nX-Public: ".to_vec();
    head.resize(MAX_RESPONSE_HEAD - 4, b'Z');
    head.extend_from_slice(b"\r\n\r\n");
    let mut with_body = head.clone();
    with_body.extend_from_slice(b"ZZZZZZZZ");
    let (_, body) = ResponseHeadBuffer::new().push(&with_body).unwrap().unwrap();
    assert_eq!(body.as_slice(), b"ZZZZZZZZ");
    head.pop();
    head.push(b'Z');
    let mut buffer = ResponseHeadBuffer::new();
    assert_eq!(buffer.push(&head).err(), Some(Error::HeadTooLarge));
    assert!(buffer.push(b"\r\n\r\n").is_err());
}

#[test]
fn header_body_suffix_keeps_an_erasing_owner() {
    let mut buffer = ResponseHeadBuffer::new();
    assert!(buffer.push(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n").unwrap().is_none());
    let observed = observe(0, 0, 0, || {
        let (_, body) = buffer.push(b"\r\nZZZZZZZZ").unwrap().unwrap();
        assert_eq!(body.as_slice(), b"ZZZZZZZZ");
        drop(black_box(body));
    });
    assert!(observed.copy_erased, "body suffix retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn chunked_body_survives_every_split_and_rejects_overlong_framing() {
    let input = b"3;public=yes\r\nabc\r\n5\r\ndefgh\r\n0\r\nX-Public: value\r\n\r\n";
    for split in 0..=input.len() {
        let mut decoder = ChunkedDecoder::new();
        let mut body = Vec::new();
        for part in [&input[..split], &input[split..]] {
            let output = decoder.decode(part).unwrap();
            assert!(output.len() <= part.len());
            body.extend_from_slice(&output);
        }
        assert!(decoder.is_complete());
        assert_eq!(body, b"abcdefgh");
    }
    let mut decoder = ChunkedDecoder::new();
    let mut body = Vec::new();
    for byte in input {
        body.extend_from_slice(&decoder.decode(&[*byte]).unwrap());
    }
    assert_eq!(body, b"abcdefgh");
    assert!(decoder.is_complete());
    let mut decoder = ChunkedDecoder::new();
    assert_eq!(decoder.decode(&[b'Z'; 129]).err(), Some(Error::InvalidChunk));
    assert_eq!(decoder.decode(b"0\r\n\r\n").err(), Some(Error::InvalidChunk));
}
