//! Public fixtures inspect initialized authentication bytes before host release.
use core::{
    cell::Cell,
    hint::black_box,
    ptr,
};
use std::alloc::{
    GlobalAlloc,
    Layout,
    System,
};

use charlotte_kafka::{
    self as wire,
    Error,
    SaslAuthenticate,
};
use zeroize::Zeroizing;

const CORRELATION: i32 = 7;
const CLIENT: &[u8] = b"fixture";
const REQUEST_AUTH_OFFSET: usize = 18 + CLIENT.len();
const RESPONSE_AUTH_OFFSET: usize = 16;
const PREFIX: usize = 8;

#[derive(Clone, Copy, Debug, Default)]
struct Observation {
    allocations: usize,
    growths: usize,
    input_releases: usize,
    input_erased: bool,
    copy_releases: usize,
    copy_erased: bool,
}
#[derive(Clone, Copy, Default)]
struct Probe {
    active: bool,
    input: usize,
    input_offset: usize,
    copy: usize,
    copy_offset: usize,
    observed: Observation,
}
thread_local! {
    static PROBE: Cell<Probe> = const { Cell::new(Probe {
        active: false, input: 0, input_offset: 0, copy: 0, copy_offset: 0,
        observed: Observation { allocations: 0, growths: 0, input_releases: 0,
            input_erased: false, copy_releases: 0, copy_erased: false },
    }) };
}
struct ObservedAllocator;
#[global_allocator]
static ALLOCATOR: ObservedAllocator = ObservedAllocator;

// SAFETY: delegate unchanged to System. Constant TLS updates allocate nothing;
// thread-teardown TLS rejection is ignored. Read only a fixture's initialized
// eight-byte field, while its original allocation is still live.
unsafe impl GlobalAlloc for ObservedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let allocation = unsafe { System.alloc(layout) };
        let _ = PROBE.try_with(|cell| {
            let mut p = cell.get();
            if p.active && !allocation.is_null() {
                p.observed.allocations += 1;
                if p.copy == 0 {
                    p.copy = allocation.addr();
                }
                cell.set(p);
            }
        });
        allocation
    }

    unsafe fn realloc(&self, allocation: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let replacement = unsafe { System.realloc(allocation, layout, size) };
        let _ = PROBE.try_with(|cell| {
            let mut p = cell.get();
            if p.active && p.copy == allocation.addr() {
                p.observed.growths += 1;
                if !replacement.is_null() {
                    p.copy = replacement.addr();
                }
                cell.set(p);
            }
        });
        replacement
    }

    unsafe fn dealloc(&self, allocation: *mut u8, layout: Layout) {
        let _ = PROBE.try_with(|cell| {
            let mut p = cell.get();
            if p.active {
                // Fixtures retain at least eight initialized authentication
                // bytes at these offsets, including both late parser errors.
                // Layout checking never authorizes reading spare capacity.
                for (target, offset, input) in
                    [(p.input, p.input_offset, true), (p.copy, p.copy_offset, false)]
                {
                    if target != 0
                        && allocation.addr() == target
                        && layout.size() >= offset + PREFIX
                    {
                        let erased = (0..PREFIX).all(|i| unsafe {
                            ptr::read_volatile(allocation.add(offset + i)) == 0
                        });
                        if input {
                            p.observed.input_releases += 1;
                            p.observed.input_erased = erased;
                        } else {
                            p.observed.copy_releases += 1;
                            p.observed.copy_erased = erased;
                        }
                    }
                }
                cell.set(p);
            }
        });
        unsafe { System.dealloc(allocation, layout) };
    }
}

fn observe(
    input: usize,
    input_offset: usize,
    copy_offset: usize,
    operation: impl FnOnce(),
) -> Observation {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            PROBE.with(|cell| cell.set(Probe::default()));
        }
    }
    let _reset = Reset;
    PROBE.with(|cell| {
        assert!(!cell.get().active);
        cell.set(Probe {
            active: true,
            input,
            input_offset,
            copy_offset,
            ..Probe::default()
        });
    });
    operation();
    PROBE.with(|cell| cell.replace(Probe::default()).observed)
}

fn parse_owned(frame: Zeroizing<Vec<u8>>) -> Result<SaslAuthenticate, Error> {
    wire::parse_sasl_authenticate(frame, CORRELATION)
}
fn auth_bytes(reply: &SaslAuthenticate) -> &[u8] {
    reply.auth_bytes()
}

fn response() -> Zeroizing<Vec<u8>> {
    let mut frame = Zeroizing::new(Vec::new());
    frame.extend_from_slice(&0i32.to_be_bytes());
    frame.extend_from_slice(&CORRELATION.to_be_bytes());
    frame.extend_from_slice(&0i16.to_be_bytes());
    frame.extend_from_slice(&(-1i16).to_be_bytes());
    frame.extend_from_slice(&32i32.to_be_bytes());
    frame.extend_from_slice(&[0x5a; 32]);
    frame.extend_from_slice(&60_000i64.to_be_bytes());
    set_length(&mut frame);
    frame
}
fn set_length(frame: &mut [u8]) {
    let payload = (frame.len() - 4) as i32;
    frame[..4].copy_from_slice(&payload.to_be_bytes());
}

#[test]
fn request_owner_erases_authentication_before_release() {
    let mut accepted = false;
    let observed = observe(0, 0, REQUEST_AUTH_OFFSET, || {
        if let Ok(request) =
            wire::sasl_authenticate_request(CORRELATION, CLIENT, black_box(&[0x5a; 32]))
        {
            accepted = request[REQUEST_AUTH_OFFSET..] == [0x5a; 32];
            drop(black_box(request));
        }
    });
    assert!(accepted);
    assert!(observed.copy_erased, "request authentication retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn oversized_complete_request_rejects_before_copying() {
    let body = vec![0x5a; wire::MAX_FRAME_LEN];
    let mut result = None;
    let observed = observe(0, 0, REQUEST_AUTH_OFFSET, || {
        result = Some(wire::sasl_authenticate_request(CORRELATION, CLIENT, black_box(&body)).err());
    });
    assert_eq!(result, Some(Some(Error::TooLarge)));
    assert_eq!(observed.allocations, 0, "rejected request copied authentication: {observed:?}");
    assert_eq!(observed.growths, 0);
}

#[test]
fn accepted_reply_retains_one_frame_without_authentication_copy() {
    let frame = response();
    let input = frame.as_ptr().addr();
    let mut accepted = false;
    let observed = observe(input, RESPONSE_AUTH_OFFSET, 0, || {
        if let Ok(reply) = parse_owned(black_box(frame)) {
            accepted = auth_bytes(&reply) == [0x5a; 32] && reply.session_lifetime_ms == 60_000;
            drop(black_box(reply));
        }
    });
    assert!(accepted);
    assert_eq!(observed.allocations, 0, "parser copied authentication: {observed:?}");
    assert_eq!(observed.input_releases, 1);
    assert!(observed.input_erased);
}

#[test]
fn late_reply_rejections_release_the_original_frame_without_a_copy() {
    for trailing in [false, true] {
        let mut frame = response();
        let expected = if trailing {
            frame.push(0);
            Error::Invalid
        } else {
            let len = frame.len() - 1;
            frame.truncate(len);
            Error::Incomplete
        };
        set_length(&mut frame);
        let input = frame.as_ptr().addr();
        let mut rejected = None;
        let observed = observe(input, RESPONSE_AUTH_OFFSET, 0, || {
            rejected = parse_owned(black_box(frame)).err();
        });
        assert_eq!(rejected, Some(expected));
        assert_eq!(observed.allocations, 0, "rejected reply copied authentication: {observed:?}");
        assert_eq!(observed.input_releases, 1);
        assert!(observed.input_erased);
    }
}

#[test]
fn request_bytes_and_complete_frame_limit_remain_exact() {
    let request = wire::sasl_authenticate_request(0x0102_0304, b"id", b"auth").unwrap();
    assert_eq!(
        &*request,
        &[
            0, 0, 0, 20, 0, 36, 0, 1, 1, 2, 3, 4, 0, 2, b'i', b'd', 0, 0, 0, 4, b'a', b'u', b't',
            b'h',
        ]
    );
    let len = wire::MAX_FRAME_LEN - (REQUEST_AUTH_OFFSET - 4);
    let body = vec![0x5a; len];
    let request = wire::sasl_authenticate_request(CORRELATION, CLIENT, &body).unwrap();
    assert_eq!(request.len(), wire::MAX_FRAME_LEN + 4);
    assert_eq!(&request[..4], &(wire::MAX_FRAME_LEN as i32).to_be_bytes());
    assert_eq!(&request[REQUEST_AUTH_OFFSET..], body);
    assert_eq!(
        wire::sasl_authenticate_request(0, &vec![0; wire::MAX_STRING_LEN + 1], b"").err(),
        Some(Error::TooLarge)
    );
}

#[test]
fn reply_ranges_cover_nullable_and_nonempty_fields() {
    for authentication in [None, Some(b"".as_slice()), Some(b"server-proof".as_slice())] {
        let mut frame = Zeroizing::new(Vec::new());
        frame.extend_from_slice(&0i32.to_be_bytes());
        frame.extend_from_slice(&CORRELATION.to_be_bytes());
        frame.extend_from_slice(&wire::SASL_AUTHENTICATION_FAILED.to_be_bytes());
        frame.extend_from_slice(&4i16.to_be_bytes());
        frame.extend_from_slice(b"oops");
        match authentication {
            None => frame.extend_from_slice(&(-1i32).to_be_bytes()),
            Some(bytes) => {
                frame.extend_from_slice(&(bytes.len() as i32).to_be_bytes());
                frame.extend_from_slice(bytes);
            }
        }
        frame.extend_from_slice(&(-1i64).to_be_bytes());
        set_length(&mut frame);
        let reply = parse_owned(frame).unwrap();
        assert_eq!(reply.error, wire::SASL_AUTHENTICATION_FAILED);
        assert_eq!(reply.auth_bytes(), authentication.unwrap_or_default());
        assert_eq!(reply.session_lifetime_ms, -1);
    }
}

#[test]
fn early_rejections_consume_and_erase_the_complete_input() {
    for (offset, bytes, expected) in [
        (4, 8i32.to_be_bytes(), Error::Correlation),
        (0, (-1i32).to_be_bytes(), Error::TooLarge),
        (0, 4i32.to_be_bytes(), Error::Incomplete),
        (12, (-2i32).to_be_bytes(), Error::Invalid),
        (12, (wire::MAX_FRAME_LEN as i32 + 1).to_be_bytes(), Error::TooLarge),
    ] {
        let mut frame = response();
        frame[offset..offset + 4].copy_from_slice(&bytes);
        let input = frame.as_ptr().addr();
        let mut result = None;
        let observed = observe(input, RESPONSE_AUTH_OFFSET, 0, || {
            result = parse_owned(black_box(frame)).err();
        });
        assert_eq!(result, Some(expected));
        assert_eq!(observed.allocations, 0);
        assert_eq!(observed.input_releases, 1);
        assert!(observed.input_erased);
    }
}
