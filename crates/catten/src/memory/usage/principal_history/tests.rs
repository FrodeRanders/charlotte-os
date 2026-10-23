//! Serialized controls use private maps for publication and the production
//! registry for exact-root retirement/restart. Heap holds detect allocation
//! or candidate destruction during publication, duplicate use and updates.
use super::*;
use crate::memory::{
    self,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

pub(in crate::memory::usage) fn run() {
    let mut history = Table::new();
    for principal in 1..=64 {
        let mut first = PreparingHistory::try_new(false).unwrap();
        let mut duplicate = PreparingHistory::try_new(false).unwrap();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            first.publish_into(&mut history, principal);
            assert!(first.0.is_none());
            let entry = history.get_mut(&principal).unwrap();
            entry.stack_pages = 7;
            entry.heap_bytes = 8192;
            duplicate.publish_into(&mut history, principal);
            assert!(duplicate.0.is_some());
            assert_eq!(
                history.get(&principal),
                Some(&History {
                    stack_pages: 7,
                    heap_bytes: 8192,
                    ..History::default()
                })
            );
        }
        first.cancel_unpublished();
        duplicate.cancel_unpublished();
        history.take(&principal).unwrap().release();
    }
    assert!(history.iter().next().is_none());
    super::storage_tests::run();
    exact_root_reuse();
    super::admission_tests::run();
    super::structural_tests::run();
    super::thread_metadata_tests::run();
    crate::logln!(
        "[principal history preparation] 64 heap-held first/duplicate publications; \
         allocation-free monotonic stack/heap updates, root retirement/restart and \
         unrelated-principal ASID reuse passed"
    );
}

fn exact_root_reuse() {
    let principal = charlotte_launch::artifact_principal_id(b"principal-history-control");
    let other = charlotte_launch::artifact_principal_id(b"principal-history-other-control");
    assert_ne!(principal, other);
    assert_eq!(stack_high_water(principal), 0);
    assert_eq!(heap_peak(principal), 0);
    let first =
        memory::register_user_address_space(memory::AddressSpace::try_new_user().unwrap()).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        memory::register_domain_authority(first, principal, 0).unwrap();
        memory::usage::note_thread_released(first.id(), 0, 7);
        memory::usage::remember_principal_heap_peak(principal, 8192);
        // Both zero samples and lower observations leave the history unchanged.
        memory::usage::remember_principal_heap_peak(principal, 0);
        memory::usage::remember_principal_heap_peak(principal, 4096);
    }
    memory::close_user_address_space_handle(first).unwrap();
    assert_eq!(stack_high_water(principal), 7);
    assert_eq!(heap_peak(principal), 8192);
    assert_eq!(charlotte_lifecycle::adaptive_stack_pages(stack_high_water(principal), 4, 64), 8);
    let next =
        memory::register_user_address_space(memory::AddressSpace::try_new_user().unwrap()).unwrap();
    assert_eq!(next.id(), first.id());
    assert_eq!(next.generation(), first.generation() + 1);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        memory::register_domain_authority(next, principal, 0).unwrap();
        memory::usage::remember_principal_stack_high_water(principal, 3);
        memory::usage::remember_principal_stack_high_water(principal, 0);
        assert_eq!(stack_high_water(principal), 7);
        assert_eq!(heap_peak(principal), 8192);
    }
    memory::close_user_address_space_handle(next).unwrap();
    let unrelated =
        memory::register_user_address_space(memory::AddressSpace::try_new_user().unwrap()).unwrap();
    assert_eq!(unrelated.id(), first.id());
    assert_eq!(unrelated.generation(), next.generation() + 1);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        memory::register_domain_authority(unrelated, other, 0).unwrap();
        assert_eq!(stack_high_water(other), 0);
        assert_eq!(heap_peak(other), 0);
        assert_eq!(stack_high_water(principal), 7);
        assert_eq!(heap_peak(principal), 8192);
    }
    memory::close_user_address_space_handle(unrelated).unwrap();
}
