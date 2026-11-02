//! Empty real capacity only; physical frames stay with the complete backing owner.
use super::*;
use crate::memory::{
    allocators::global_allocator::PRIMARY_ALLOCATOR,
    object::cap_storage::admission as budget,
};
fn buffers(platform: bool) -> Vec<FrameVector> {
    let max_bytes = (MAX_MEMORY_OBJECT_PAGES * core::mem::size_of::<PAddr>()) as u64;
    let total = crate::resource_policy::POLICY.memory_capability_nodes.total[0];
    let mut result = Vec::with_capacity((total / max_bytes) as usize + MAX_MEMORY_OBJECT_PAGES);
    for pages in [MAX_MEMORY_OBJECT_PAGES, 1] {
        while let Ok(vector) = FrameVector::try_new(platform, pages) {
            assert_eq!(vector.capacity(), pages);
            result.push(vector);
        }
    }
    result
}
pub(in crate::memory::object::backing) fn run() {
    let before = budget::snapshot();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        assert!(
            FrameVector::try_new_with(false, 2, |frames, pages| {
                frames.try_reserve_exact(pages).unwrap();
                Err(MemoryObjectError::FrameAllocFailed)
            })
            .is_err()
        );
        assert_eq!(budget::snapshot(), before);
    }
    let ordinary = buffers(false);
    let full = budget::snapshot();
    assert_eq!(full.1[1], before.1[1]);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(FrameVector::try_new(false, 1).is_err());
    }
    let trusted = FrameVector::try_new(true, 1).unwrap();
    assert_eq!(budget::snapshot().1, full.1);
    trusted.finish();
    let trusted = buffers(true);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(FrameVector::try_new(false, 1).is_err());
        assert!(FrameVector::try_new(true, 1).is_err());
    }
    let counts = (ordinary.len(), trusted.len());
    for vector in ordinary.into_iter().chain(trusted) {
        vector.finish();
    }
    assert_eq!(budget::snapshot(), before);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[object frame-vector admission] real empty buffers ordinary={} trusted={}; byte-only \
         ordinary/total exhaustion, heap-held pre-allocator denial, 64 allocated-prefix \
         rejections, captured trusted reserve, node counts unchanged and exact recovery passed; \
         max={} bytes",
        counts.0,
        counts.1,
        MAX_MEMORY_OBJECT_PAGES * core::mem::size_of::<PAddr>()
    );
}
