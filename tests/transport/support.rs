//! Host-only release observer for initialized public fixture fields.
use core::{
    cell::Cell,
    ptr,
};
use std::alloc::{
    GlobalAlloc,
    Layout,
    System,
};

const PREFIX: usize = 8;

#[derive(Clone, Copy, Debug, Default)]
pub struct Observation {
    pub allocations: usize,
    pub growths: usize,
    pub input_releases: usize,
    pub input_erased: bool,
    pub copy_releases: usize,
    pub copy_erased: bool,
}
#[derive(Clone, Copy, Default)]
struct Probe {
    active: bool,
    input: usize,
    input_offset: usize,
    copy: usize,
    copy_offset: usize,
    copy_ordinal: usize,
    observed: Observation,
}
thread_local! {
    static PROBE: Cell<Probe> = const { Cell::new(Probe {
        active: false, input: 0, input_offset: 0, copy: 0, copy_offset: 0, copy_ordinal: 0,
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
                if p.observed.allocations == p.copy_ordinal {
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
                // Every tracked offset names eight initialized fixture bytes,
                // including buffers whose logical length was later shortened.
                // Layout checking never authorizes reading spare capacity.
                for (target, offset, input) in
                    [(p.input, p.input_offset, true), (p.copy, p.copy_offset, false)]
                {
                    if target != 0 && allocation.addr() == target {
                        if layout.size() >= offset + PREFIX {
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
                        // The selected allocation is ending. Never inspect a
                        // later allocation that happens to reuse its address.
                        if input {
                            p.input = 0;
                        } else {
                            p.copy = 0;
                        }
                    }
                }
                cell.set(p);
            }
        });
        unsafe { System.dealloc(allocation, layout) };
    }
}

pub fn observe(
    input: usize,
    input_offset: usize,
    copy_offset: usize,
    operation: impl FnOnce(),
) -> Observation {
    observe_nth(1, input, input_offset, copy_offset, operation)
}

/// Select a known initialized fixture allocation by its construction order.
/// The caller must establish its field offset and lifetime before observing.
pub fn observe_nth(
    copy_ordinal: usize,
    input: usize,
    input_offset: usize,
    copy_offset: usize,
    operation: impl FnOnce(),
) -> Observation {
    assert!(copy_ordinal > 0);
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
            copy_ordinal,
            ..Probe::default()
        });
    });
    operation();
    PROBE.with(|cell| cell.replace(Probe::default()).observed)
}
