//! Public fixtures inspect initialized plaintext bytes before host release.
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

use catten_services::tls_client::read_owned_chunk;
use charlotte_kafka::{
    self as wire,
    frame::{
        FrameReceiver,
        ReceiveError,
    },
};
use zeroize::Zeroizing;
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
                // bytes at these offsets, including the partial-frame reset.
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

#[derive(Clone, Copy)]
enum Outcome {
    Success,
    Failure,
    Eof,
    InvalidCount,
}
struct Reader(Outcome);
impl embedded_io::ErrorType for Reader {
    type Error = embedded_io::ErrorKind;
}
impl embedded_io::Read for Reader {
    fn read(&mut self, bytes: &mut [u8]) -> Result<usize, Self::Error> {
        bytes.fill(0x5a);
        match self.0 {
            Outcome::Success => Ok(bytes.len() / 2),
            Outcome::Failure => Err(embedded_io::ErrorKind::Other),
            Outcome::Eof => Ok(0),
            Outcome::InvalidCount => Ok(bytes.len() + 1),
        }
    }
}
fn frame() -> Vec<u8> {
    let mut bytes = 32i32.to_be_bytes().to_vec();
    bytes.extend_from_slice(&[0x5a; 32]);
    bytes
}
fn chunk(bytes: &[u8]) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(bytes.to_vec())
}

#[test]
fn plaintext_success_erases_before_release() {
    let mut valid = false;
    let observed = observe(0, 0, 0, || {
        if let Ok(bytes) = read_owned_chunk(&mut Reader(Outcome::Success), black_box(32)) {
            valid = bytes.as_slice() == [0x5a; 16];
            drop(black_box(bytes));
        }
    });
    assert!(valid);
    assert!(observed.copy_erased, "plaintext retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn plaintext_truncated_tail_erases_before_release() {
    // The reader initializes all 32 bytes but returns 16. Inspect a known
    // initialized field beyond the shortened logical length, never spare bytes
    // that have not been initialized.
    let observed = observe(0, 0, 24, || {
        drop(black_box(read_owned_chunk(&mut Reader(Outcome::Success), black_box(32))));
    });
    assert!(observed.copy_erased, "truncated tail retained: {observed:?}");
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn producer_failure_eof_and_invalid_count_erase_the_destination() {
    // Deliberate fault readers write before error/EOF or report an invalid count.
    // The pinned TLS implementation errors before copying; this tests the generic
    // helper's stronger boundary, not a reproduced TLS partial-write failure.
    for outcome in [Outcome::Failure, Outcome::Eof, Outcome::InvalidCount] {
        let mut rejected = false;
        let observed = observe(0, 0, 0, || {
            rejected = black_box(read_owned_chunk(&mut Reader(outcome), black_box(32))).is_err();
        });
        assert!(rejected);
        assert!(observed.copy_erased, "failed destination retained: {observed:?}");
        assert_eq!(observed.copy_releases, 1);
    }
}

#[test]
fn fragmented_reply_reserves_once_and_erases_chunk_and_frame() {
    let bytes = frame();
    let mut chunks = [chunk(&bytes[..4]), chunk(&bytes[4..])].into_iter();
    let mut receiver = FrameReceiver::new();
    let mut valid = false;
    let observed = observe(0, 0, 4, || {
        if let Ok(bytes) = receiver.read_frame(|| chunks.next().ok_or(7)) {
            valid = bytes.len() == 36 && bytes[4..] == [0x5a; 32];
            drop(black_box(bytes));
        }
        receiver.reset();
    });
    assert!(valid);
    assert!(observed.copy_erased, "frame retained: {observed:?}");
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0, "frame grew: {observed:?}");
    assert_eq!(observed.copy_releases, 1);
}

#[test]
fn partial_failure_reset_erases_the_containing_owner() {
    let bytes = frame();
    let pending = chunk(&bytes[..16]);
    let input = pending.as_ptr().addr();
    let mut pending = Some(pending);
    let mut receiver = FrameReceiver::new();
    let mut failed = false;
    let observed = observe(input, 4, 4, || {
        failed = matches!(
            receiver.read_frame(|| pending.take().ok_or(7)),
            Err(ReceiveError::Transport(7))
        );
        receiver.reset();
        // The old receiver kept its allocation after clear/reset. Release the
        // containing owner inside the observation window as well.
        drop(receiver);
    });
    assert!(failed);
    assert!(observed.copy_erased, "partial frame retained: {observed:?}");
    assert!(observed.input_erased);
    assert_eq!(observed.input_releases, 1);
    assert_eq!(observed.copy_releases, 1);
    assert_eq!(observed.allocations, 1);
    assert_eq!(observed.growths, 0);
}

#[test]
fn every_fragment_boundary_and_coalesced_frames_preserve_bytes() {
    let bytes = frame();
    for split in 1..bytes.len() {
        let mut chunks = [chunk(&bytes[..split]), chunk(&bytes[split..])].into_iter();
        let mut receiver = FrameReceiver::new();
        assert_eq!(receiver.read_frame(|| chunks.next().ok_or(7)).unwrap().as_slice(), bytes);
    }
    let mut joined = bytes.clone();
    joined.extend_from_slice(&bytes);
    let mut pending = Some(chunk(&joined));
    let mut calls = 0;
    let mut receiver = FrameReceiver::new();
    for _ in 0..2 {
        let result = receiver
            .read_frame(|| {
                calls += 1;
                pending.take().ok_or(7)
            })
            .unwrap();
        assert_eq!(result.as_slice(), bytes);
    }
    assert_eq!(calls, 1);
}

#[test]
fn invalid_header_and_empty_chunk_fence_before_further_receives() {
    for payload in [-1, 0, 3, wire::MAX_FRAME_LEN as i32 + 1] {
        let mut pending = Some(chunk(&payload.to_be_bytes()));
        let mut receiver = FrameReceiver::new();
        assert!(matches!(
            receiver.read_frame(|| pending.take().ok_or(7)),
            Err(ReceiveError::Protocol(_))
        ));
        let mut calls = 0;
        assert!(matches!(
            receiver.read_frame(|| {
                calls += 1;
                Err::<Zeroizing<Vec<u8>>, _>(7)
            }),
            Err(ReceiveError::Poisoned)
        ));
        assert_eq!(calls, 0);
    }
    let mut receiver = FrameReceiver::new();
    assert!(matches!(
        receiver.read_frame(|| Ok::<_, ()>(chunk(&[]))),
        Err(ReceiveError::EmptyChunk)
    ));
    assert!(matches!(
        receiver.read_frame(|| Err::<Zeroizing<Vec<u8>>, _>(7)),
        Err(ReceiveError::Poisoned)
    ));
}

#[test]
fn failure_and_unwind_require_reset_before_a_new_source() {
    let mut receiver = FrameReceiver::new();
    assert!(matches!(
        receiver.read_frame(|| Err::<Zeroizing<Vec<u8>>, _>(7)),
        Err(ReceiveError::Transport(7))
    ));
    assert!(matches!(
        receiver.read_frame(|| Ok::<_, ()>(chunk(&frame()))),
        Err(ReceiveError::Poisoned)
    ));
    receiver.reset();
    assert_eq!(receiver.read_frame(|| Ok::<_, ()>(chunk(&frame()))).unwrap().as_slice(), frame());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = receiver.read_frame::<()>(|| panic!("public producer-interruption fixture"));
    }));
    assert!(result.is_err());
    assert!(matches!(
        receiver.read_frame(|| Ok::<_, ()>(chunk(&frame()))),
        Err(ReceiveError::Poisoned)
    ));
}

#[test]
fn maximum_frame_and_chunk_bounds_are_checked_separately() {
    let mut bytes = (wire::MAX_FRAME_LEN as i32).to_be_bytes().to_vec();
    bytes.resize(wire::MAX_FRAME_LEN + 4, 0x5a);
    let mut pending = Some(chunk(&bytes));
    let mut receiver = FrameReceiver::new();
    assert_eq!(receiver.read_frame(|| pending.take().ok_or(7)).unwrap().as_slice(), bytes);
    let mut receiver = FrameReceiver::new();
    let mut pending = Some(Zeroizing::new(vec![0; wire::MAX_FRAME_LEN + 5]));
    assert!(matches!(
        receiver.read_frame(|| pending.take().ok_or(7)),
        Err(ReceiveError::ChunkTooLarge)
    ));
}
