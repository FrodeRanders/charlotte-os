//! Kernel-side scratch allocator for deferred verifiers.
//!
//! Deferred verifiers map moved-memory reply caps into the *kernel* address
//! space to read their contents. Because every verifier shares one address
//! space, fixed scratch vaddrs are a collision surface — two tests once both
//! used `0x601000`, corrupting each other's reads. The allocator bump-carves
//! distinct scratch pages from a reserved region, so each mapping gets a
//! vaddr that no other test can own, by construction.

use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

/// The reserved kernel scratch region. `0x600000..0x700000` is not used by
/// the kernel image or its data; the allocator hands out pages from here and
/// never reuses them.
const SCRATCH_REGION_BASE: usize = 0x0000_0000_0060_0000;
const SCRATCH_REGION_END: usize = 0x0000_0000_0070_0000;
const SCRATCH_PAGE_SIZE: usize = 0x1000;

static NEXT_SCRATCH: AtomicUsize = AtomicUsize::new(SCRATCH_REGION_BASE);

/// Allocate one scratch page in the kernel address space. The page is never
/// handed out again, so concurrent verifiers cannot collide. Returns `None`
/// when the region is exhausted.
pub fn allocate_scratch_page() -> Option<usize> {
    let next = NEXT_SCRATCH.fetch_add(SCRATCH_PAGE_SIZE, Ordering::Relaxed);
    if next + SCRATCH_PAGE_SIZE > SCRATCH_REGION_END {
        return None;
    }
    Some(next)
}
