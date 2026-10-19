//! Public SigV4 fixtures and complete owned HTTP request headers.
use core::hint::black_box;

use charlotte_s3::{
    http::{
        Error,
        RequestHead,
        request_head,
    },
    sigv4::{
        Credentials,
        EMPTY_PAYLOAD_SHA256,
        Header,
        Request,
        Signature,
        Timestamp,
        sign,
    },
};

mod support;
use support::observe;

fn signed() -> Signature {
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
    sign(
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
            secret_key: b"public signing fixture",
        },
    )
    .unwrap()
}

#[test]
fn authorization_erases_before_release() {
    let signature = signed();
    let pointer = signature.authorization.as_ptr().addr();
    let offset = signature.authorization.rfind("Signature=").unwrap() + "Signature=".len();
    let observed = observe(pointer, offset, 0, || drop(black_box(signature)));
    assert!(observed.input_erased, "authorization retained: {observed:?}");
    assert_eq!(observed.input_releases, 1);
}

#[test]
fn signature_text_erases_before_release() {
    let signature = signed();
    let pointer = signature.signature.as_ptr().addr();
    let observed = observe(pointer, 0, 0, || drop(black_box(signature)));
    assert!(observed.input_erased, "signature retained: {observed:?}");
    assert_eq!(observed.input_releases, 1);
}

#[test]
fn request_header_erases_without_growth() {
    let prefix = concat!(
        "GET /bucket/key HTTP/1.1\r\nHost: example.test\r\n",
        "x-amz-content-sha256: public-hash\r\nx-amz-date: 20261019T120000Z\r\nAuthorization: "
    )
    .as_bytes();
    let headers = [
        Header {
            name: "Host",
            value: "example.test",
        },
        Header {
            name: "x-amz-content-sha256",
            value: "public-hash",
        },
        Header {
            name: "x-amz-date",
            value: "20261019T120000Z",
        },
        Header {
            name: "Authorization",
            value: "ZZZZZZZZ",
        },
    ];
    let request = RequestHead {
        method: "GET",
        path: "/bucket/key",
        headers: &headers,
        content_length: None,
    };
    let observed = observe(0, 0, prefix.len(), || {
        let output = request_head(black_box(&request)).unwrap();
        assert!(output.starts_with(prefix));
        assert_eq!(&output[prefix.len()..prefix.len() + 8], b"ZZZZZZZZ");
        drop(black_box(output));
    });
    assert!(observed.copy_erased, "wire authorization retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.copy_releases, 1);
    assert_eq!(observed.growths, 0, "request header grew: {observed:?}");
}

#[test]
fn invalid_wire_fields_reject_before_allocation() {
    let valid = Header {
        name: "Host",
        value: "example.test",
    };
    let missing = [];
    let invalid = [
        [
            valid,
            Header {
                name: "Authorization",
                value: "ZZZZZZZZ\r\nX-Forged: yes",
            },
        ],
        [
            valid,
            Header {
                name: "Bad\r\nName",
                value: "ZZZZZZZZ",
            },
        ],
        [
            valid,
            Header {
                name: "X-Public",
                value: "ZZZZZZZZ\0",
            },
        ],
        [
            valid,
            Header {
                name: "X-Public",
                value: "ZZZZZZZZ\x7f",
            },
        ],
        [
            valid,
            Header {
                name: "host",
                value: "duplicate.test",
            },
        ],
        [
            valid,
            Header {
                name: "Content-Length",
                value: "1",
            },
        ],
        [
            valid,
            Header {
                name: "Transfer-Encoding",
                value: "chunked",
            },
        ],
        [
            valid,
            Header {
                name: "Connection",
                value: "keep-alive",
            },
        ],
    ];
    for headers in invalid.iter().map(|h| h.as_slice()).chain([missing.as_slice()]) {
        let observed = observe(0, 0, 0, || {
            assert_eq!(
                request_head(&RequestHead {
                    method: "GET",
                    path: "/key",
                    headers,
                    content_length: None
                })
                .err(),
                Some(Error::InvalidHeader)
            );
        });
        assert_eq!(observed.allocations, 0, "rejected fields copied: {observed:?}");
    }
    for (method, path, host) in [
        ("GET\r\nX", "/key", "example.test"),
        ("GET", "/key HTTP/1.1", "example.test"),
        ("GET", "/key", " \t"),
    ] {
        let headers = [Header {
            name: "Host",
            value: host,
        }];
        let observed = observe(0, 0, 0, || {
            assert_eq!(
                request_head(&RequestHead {
                    method,
                    path,
                    headers: &headers,
                    content_length: None
                })
                .err(),
                Some(Error::InvalidHeader)
            );
        });
        assert_eq!(observed.allocations, 0);
    }
}

#[test]
fn request_bytes_preserve_optional_fields_and_u64_length() {
    let headers = [
        Header {
            name: "Host",
            value: "example.test",
        },
        Header {
            name: "Authorization",
            value: "public fixture",
        },
        Header {
            name: "Range",
            value: "bytes=1-7",
        },
        Header {
            name: "If-None-Match",
            value: "*",
        },
    ];
    for (len, decimal) in [(0, "0"), (8, "8"), (u64::MAX, "18446744073709551615")] {
        let bytes = request_head(&RequestHead {
            method: "PUT",
            path: "/bucket/key%20name",
            headers: &headers,
            content_length: Some(len),
        })
        .unwrap();
        let expected = format!(
            "PUT /bucket/key%20name HTTP/1.1\r\nHost: example.test\r\nAuthorization: public \
             fixture\r\nRange: bytes=1-7\r\nIf-None-Match: *\r\nContent-Length: \
             {decimal}\r\nConnection: close\r\n\r\n"
        );
        assert_eq!(bytes.as_slice(), expected.as_bytes());
    }
}
