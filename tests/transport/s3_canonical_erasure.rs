//! Public canonical-signing fixtures and initialized-byte release controls.
use core::hint::black_box;

use charlotte_s3::sigv4::{
    Credentials,
    EMPTY_PAYLOAD_SHA256,
    Error,
    Header,
    Query,
    Request,
    Signature,
    Timestamp,
    canonical_query,
    canonical_uri,
    sign,
};
mod support;
use support::observe;
#[path = "fixtures/s3_canonical.rs"]
mod fixture;

const QUERY: [Query<'static>; 3] = [
    Query {
        name: "z",
        value: "ZZZZZZZZ",
    },
    Query {
        name: "a",
        value: "雪",
    },
    Query {
        name: "a",
        value: "a/b c",
    },
];
const HEADERS: [Header<'static>; 4] = [
    Header {
        name: "Host",
        value: "example.test",
    },
    Header {
        name: "X-Amz-Content-Sha256",
        value: EMPTY_PAYLOAD_SHA256,
    },
    Header {
        name: "X-Amz-Date",
        value: "20261020T120000Z",
    },
    Header {
        name: "X-ZZZZZZZZ",
        value: " private\t value \r\n 雪 ",
    },
];
fn request() -> Request<'static> {
    Request {
        method: "GET",
        path: "/bucket/ZZZZZZZZ key/雪",
        query: &QUERY,
        headers: &HEADERS,
        payload_sha256: EMPTY_PAYLOAD_SHA256,
        region: "us-east-1",
        service: "s3",
        timestamp: Timestamp {
            year: 2026,
            month: 10,
            day: 20,
            hour: 12,
            minute: 0,
            second: 0,
        },
    }
}
fn signed() -> Signature {
    sign(
        &request(),
        Credentials {
            access_key: "PUBLIC",
            secret_key: b"public signing fixture",
        },
    )
    .unwrap()
}

#[test]
fn uri_erases_without_growth() {
    let observed = observe(0, 0, 8, || {
        let uri = canonical_uri(black_box(request().path)).unwrap();
        assert_eq!(uri.as_str(), fixture::URI);
        drop(black_box(uri));
    });
    assert!(observed.copy_erased, "URI retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.copy_releases, 1);
    assert_eq!(observed.growths, 0, "URI grew: {observed:?}");
}
#[test]
fn query_erases_before_release() {
    let query = canonical_query(&QUERY).unwrap();
    let ptr = query.as_ptr().addr();
    let offset = query.find("ZZZZZZZZ").unwrap();
    let observed = observe(ptr, offset, 0, || drop(black_box(query)));
    assert!(observed.input_erased, "query retained: {observed:?}");
    assert_eq!(observed.input_releases, 1);
}
#[test]
fn canonical_request_erases_before_release() {
    let signature = signed();
    let ptr = signature.canonical_request.as_ptr().addr();
    let offset = signature.canonical_request.find("ZZZZZZZZ").unwrap();
    let observed = observe(ptr, offset, 0, || drop(black_box(signature)));
    assert!(observed.input_erased, "canonical request retained: {observed:?}");
    assert_eq!(observed.input_releases, 1);
}
#[test]
fn string_to_sign_erases_before_release() {
    let signature = signed();
    let ptr = signature.string_to_sign.as_ptr().addr();
    let offset = signature.string_to_sign.rfind('\n').unwrap() + 1;
    let observed = observe(ptr, offset, 0, || drop(black_box(signature)));
    assert!(observed.input_erased, "signing transcript retained: {observed:?}");
    assert_eq!(observed.input_releases, 1);
}
#[test]
fn signed_names_erase_before_release() {
    let signature = signed();
    let ptr = signature.signed_headers.as_ptr().addr();
    let offset = signature.signed_headers.find("zzzzzzzz").unwrap();
    let observed = observe(ptr, offset, 0, || drop(black_box(signature)));
    assert!(observed.input_erased, "signed names retained: {observed:?}");
    assert_eq!(observed.input_releases, 1);
}
#[test]
fn canonical_values_match_independent_fixture() {
    let signature = signed();
    assert_eq!(canonical_query(&QUERY).unwrap().as_str(), fixture::QUERY);
    assert_eq!(signature.canonical_request.as_str(), fixture::CANONICAL_REQUEST);
    assert_eq!(signature.string_to_sign.as_str(), fixture::STRING_TO_SIGN);
    assert_eq!(signature.signed_headers.as_str(), fixture::SIGNED_HEADERS);
    assert_eq!(signature.signature.as_str(), fixture::SIGNATURE);
    assert_eq!(signature.authorization.as_str(), fixture::AUTHORIZATION);
}
#[test]
fn invalid_headers_and_relative_paths_reject() {
    for extra in [
        Header {
            name: "Invalid!",
            value: "ZZZZZZZZ",
        },
        Header {
            name: "HOST",
            value: "duplicate",
        },
    ] {
        let mut headers = HEADERS.to_vec();
        headers.push(extra);
        assert_eq!(
            sign(
                &Request {
                    headers: &headers,
                    ..request()
                },
                Credentials {
                    access_key: "PUBLIC",
                    secret_key: b"public signing fixture"
                }
            )
            .err(),
            Some(Error::InvalidHeader)
        );
    }
    assert_eq!(
        sign(
            &Request {
                headers: &HEADERS[1..],
                ..request()
            },
            Credentials {
                access_key: "PUBLIC",
                secret_key: b"public signing fixture"
            }
        )
        .err(),
        Some(Error::InvalidHeader)
    );
    let observed = observe(0, 0, 0, || {
        assert_eq!(canonical_uri("relative/ZZZZZZZZ").err(), Some(Error::InvalidPath))
    });
    assert_eq!(observed.allocations, 0);
}
#[test]
fn empty_query_duplicate_values_and_ascii_folding_preserve_bytes() {
    assert!(canonical_query(&[]).unwrap().is_empty());
    let items = [
        Query {
            name: "a",
            value: " ",
        },
        Query {
            name: "empty",
            value: "",
        },
        Query {
            name: "a",
            value: "",
        },
    ];
    assert_eq!(canonical_query(&items).unwrap().as_str(), "a=&a=%20&empty=");
    let mut headers = HEADERS;
    headers[3].value = " \t \r\n ";
    let result = sign(
        &Request {
            headers: &headers,
            query: &[],
            ..request()
        },
        Credentials {
            access_key: "PUBLIC",
            secret_key: b"public signing fixture",
        },
    )
    .unwrap();
    assert!(result.canonical_request.contains("x-zzzzzzzz:\n\n"));
    headers[3].value = "a\u{a0}b";
    let result = sign(
        &Request {
            headers: &headers,
            query: &[],
            ..request()
        },
        Credentials {
            access_key: "PUBLIC",
            secret_key: b"public signing fixture",
        },
    )
    .unwrap();
    assert!(result.canonical_request.contains("x-zzzzzzzz:a\u{a0}b\n"));
}
