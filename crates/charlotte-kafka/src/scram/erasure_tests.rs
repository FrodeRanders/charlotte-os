//! Inspect public fixture prefixes immediately before the host allocator frees
//! them. The probe is thread-local and inactive during other tests.

extern crate std;

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

use super::{
    base64_decode,
    base64_encode,
};

// Each fixture initializes at least these eight bytes before release, including
// rejection after a valid decoded prefix. Never inspect uninitialized capacity.
const PREFIX_LEN: usize = 8;
const ENCODED: &str = "WlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaWlo=";

#[derive(Clone, Copy, Debug, Default)]
struct Observation {
    allocations: usize,
    growths: usize,
    releases: usize,
    prefix_erased: bool,
}

#[derive(Clone, Copy, Default)]
struct Probe {
    active: bool,
    target: usize,
    observation: Observation,
}

std::thread_local! {
    static PROBE: Cell<Probe> = const { Cell::new(Probe {
        active: false,
        target: 0,
        observation: Observation {
            allocations: 0, growths: 0, releases: 0, prefix_erased: false,
        },
    }) };
}

struct ObservedAllocator;

#[global_allocator]
static ALLOCATOR: ObservedAllocator = ObservedAllocator;

// SAFETY: all allocation operations delegate unchanged to System. Only the
// selected fixture's initialized prefix is read, before System deallocates it.
// TLS updates allocate nothing; unavailable TLS during thread teardown is ignored.
unsafe impl GlobalAlloc for ObservedAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let allocation = unsafe { System.alloc(layout) };
        let _ = PROBE.try_with(|probe| {
            let mut state = probe.get();
            if state.active && !allocation.is_null() {
                state.observation.allocations += 1;
                if state.target == 0 {
                    state.target = allocation.addr();
                }
                probe.set(state);
            }
        });
        allocation
    }

    unsafe fn realloc(&self, allocation: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let replacement = unsafe { System.realloc(allocation, layout, size) };
        let _ = PROBE.try_with(|probe| {
            let mut state = probe.get();
            if state.active && state.target == allocation.addr() {
                state.observation.growths += 1;
                if !replacement.is_null() {
                    state.target = replacement.addr();
                }
                probe.set(state);
            }
        });
        replacement
    }

    unsafe fn dealloc(&self, allocation: *mut u8, layout: Layout) {
        let _ = PROBE.try_with(|probe| {
            let mut state = probe.get();
            if state.active && state.target == allocation.addr() && layout.size() >= PREFIX_LEN {
                // SAFETY: only these three fixtures arm the probe. Their first
                // allocation is the output, and all initialize this prefix
                // before success/late rejection. The allocation is still live.
                state.observation.prefix_erased =
                    (0..PREFIX_LEN).all(|i| unsafe { ptr::read_volatile(allocation.add(i)) == 0 });
                state.observation.releases += 1;
                probe.set(state);
            }
        });
        unsafe { System.dealloc(allocation, layout) };
    }
}

fn observe(operation: impl FnOnce()) -> Observation {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            PROBE.with(|probe| probe.set(Probe::default()));
        }
    }
    let _reset = Reset;
    PROBE.with(|probe| {
        assert!(!probe.get().active);
        probe.set(Probe {
            active: true,
            ..Probe::default()
        });
    });
    operation();
    PROBE.with(|probe| probe.replace(Probe::default()).observation)
}

fn assert_erasing_release(observed: Observation) {
    assert!(observed.prefix_erased, "released prefix retained bytes: {observed:?}");
    assert_eq!(observed.allocations, 1, "one output allocation: {observed:?}");
    assert_eq!(observed.growths, 0, "output must not leave growth copies: {observed:?}");
    assert_eq!(observed.releases, 1, "one final output release: {observed:?}");
}

#[test]
fn encoded_output_erases_before_release() {
    let observed = observe(|| drop(black_box(base64_encode(black_box(&[0x5a; 32]), true))));
    assert_erasing_release(observed);
}

#[test]
fn decoded_output_erases_before_release() {
    let mut accepted = false;
    let observed = observe(|| {
        if let Ok(output) = base64_decode(black_box(ENCODED)) {
            accepted = output.as_slice() == [0x5a; 32];
            drop(black_box(output));
        }
    });
    assert!(accepted);
    assert_erasing_release(observed);
}

#[test]
fn malformed_suffix_erases_decoded_prefix_before_release() {
    for malformed in [
        "WlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpa!!!!",
        "WlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpa=AAA",
        "WlpaWlpaWlpaWlpaWlpaWlpaWlpaWlpaAA=A",
    ] {
        let mut rejected = false;
        let observed = observe(|| {
            rejected = black_box(base64_decode(black_box(malformed))).is_err();
        });
        assert!(rejected);
        assert_erasing_release(observed);
    }
}
