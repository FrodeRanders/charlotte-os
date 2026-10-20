//! Local key/path owners use public request fixtures.
use core::hint::black_box;

use catten_services::s3 as wire;
use charlotte_s3::{
    request::OwnedRequest,
    sigv4::canonical_uri,
};
mod support;
use support::observe;

fn encode(request: wire::ObjectRequest<'_>) -> Vec<u8> {
    let mut bytes = vec![0; request.encoded_len()];
    request.encode(&mut bytes).unwrap();
    bytes
}
fn packet(key: &[u8]) -> Vec<u8> {
    encode(wire::ObjectRequest::get(key))
}
#[test]
fn decoded_key_erases_before_release() {
    let bytes = packet(b"ZZZZZZZZ/key");
    let observed = observe(0, 0, 0, || {
        let request = OwnedRequest::decode(black_box(&bytes)).unwrap();
        assert_eq!(request.key(), b"ZZZZZZZZ/key");
        drop(black_box(request));
    });
    assert!(observed.copy_erased, "request key retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}
#[test]
fn raw_path_erases_without_growth() {
    let request = OwnedRequest::decode(&packet(b"ZZZZZZZZ/key")).unwrap();
    let prefix = "reports/";
    let offset = "/bucket/reports/".len();
    let observed = observe(0, 0, offset, || {
        let path = request.object_path("bucket", prefix).unwrap();
        assert_eq!(path.as_str(), "/bucket/reports/ZZZZZZZZ/key");
        drop(black_box(path));
    });
    assert!(observed.copy_erased, "raw path retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0, "raw path grew: {observed:?}");
    assert_eq!(observed.copy_releases, 1);
}
#[test]
fn rejected_path_erases_key() {
    let bytes = packet(b"ZZZZZZZZ/\xff");
    let mut rejected = false;
    let observed = observe(0, 0, 0, || {
        let request = OwnedRequest::decode(&bytes).unwrap();
        rejected = request.object_path("bucket", "reports").is_none();
        drop(black_box(request));
    });
    assert!(rejected);
    assert!(observed.copy_erased, "rejected key retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.copy_releases, 1);
    assert_eq!(observed.growths, 0);
}
#[test]
fn invalid_wire_rejects_before_allocation() {
    let valid = packet(b"ZZZZZZZZ");
    let mut cases = Vec::new();
    let mut magic = valid.clone();
    magic[0] ^= 0xff;
    cases.push(magic);
    let mut version = valid.clone();
    version[4..6].copy_from_slice(&0u16.to_le_bytes());
    cases.push(version);
    let mut flags = valid.clone();
    flags[6..8].copy_from_slice(&0xffffu16.to_le_bytes());
    cases.push(flags);
    let mut empty = valid.clone();
    empty[64..66].fill(0);
    cases.push(empty);
    let mut large = valid.clone();
    large[64..66].copy_from_slice(&((wire::MAX_KEY_LEN + 1) as u16).to_le_bytes());
    cases.push(large);
    let mut extra = valid.clone();
    extra.push(0);
    cases.push(extra);
    cases.push(valid[..valid.len() - 1].to_vec());
    cases.push(Vec::new());
    let mut range = valid;
    range[16..24].copy_from_slice(&1u64.to_le_bytes());
    cases.push(range);
    for bytes in cases {
        let mut rejected = false;
        let observed = observe(0, 0, 0, || {
            rejected = OwnedRequest::decode(&bytes).is_none();
        });
        assert!(rejected);
        assert_eq!(observed.allocations, 0, "invalid request copied: {observed:?}");
    }
}
#[test]
fn paths_preserve_prefix_and_percent_encoding() {
    let request = OwnedRequest::decode(&packet("雪/a b+/%2e".as_bytes())).unwrap();
    for prefix in ["", "reports", "reports/", "reports///"] {
        let path = request.object_path("bucket", prefix).unwrap();
        let expected = if prefix.is_empty() {
            "/bucket/雪/a b+/%2e"
        } else {
            "/bucket/reports/雪/a b+/%2e"
        };
        assert_eq!(path.as_str(), expected);
        let expected = if prefix.is_empty() {
            "/bucket/%E9%9B%AA/a%20b%2B/%252e"
        } else {
            "/bucket/reports/%E9%9B%AA/a%20b%2B/%252e"
        };
        assert_eq!(canonical_uri(&path).unwrap().as_str(), expected);
    }
}
#[test]
fn forbidden_keys_reject_before_path_allocation() {
    for key in [b"/absolute".as_slice(), b"a/../b", b"./key", b"key/.", b"a\0b", b"\xff"] {
        let request = OwnedRequest::decode(&packet(key)).unwrap();
        let mut rejected = false;
        let observed = observe(0, 0, 0, || {
            rejected = request.object_path("bucket", "reports").is_none();
        });
        assert!(rejected);
        assert_eq!(observed.allocations, 0);
    }
}
#[test]
fn maximum_key_and_scalar_fields_are_preserved() {
    let key = vec![b'Z'; wire::MAX_KEY_LEN];
    let bytes = encode(
        wire::ObjectRequest::put(&key, u64::MAX, [0x5a; 32]).with_range(10, 20).create_only(),
    );
    let request = OwnedRequest::decode(&bytes).unwrap();
    assert_eq!(request.key(), key);
    assert_eq!(request.content_length, u64::MAX);
    assert_eq!(request.range_start, 10);
    assert_eq!(request.range_end, 20);
    assert_eq!(request.payload_sha256, [0x5a; 32]);
    assert_eq!(request.flags, wire::FLAG_RANGE | wire::FLAG_IF_NONE_MATCH);
    assert_eq!(
        request.object_path("bucket", "reports").unwrap().len(),
        "/bucket/reports/".len() + wire::MAX_KEY_LEN
    );
}
#[test]
fn decoded_snapshot_has_an_independent_lifetime() {
    let mut bytes = packet(b"ZZZZZZZZ");
    let request = OwnedRequest::decode(&bytes).unwrap();
    bytes[wire::REQUEST_HEADER_LEN..].fill(b'X');
    drop(bytes);
    assert_eq!(request.key(), b"ZZZZZZZZ");
    assert_eq!(request.object_path("bucket", "").unwrap().as_str(), "/bucket/ZZZZZZZZ");
}
