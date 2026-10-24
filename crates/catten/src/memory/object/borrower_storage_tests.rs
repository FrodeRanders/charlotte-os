//! Serialized controls for read-loan storage before source authority mutation.
use super::*;
use crate::memory::{
    self,
    AddressSpaceHandle,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}

pub(super) fn run() {
    rejection_and_cancel();
    relinking();
    readers_and_namespace_close();
    crate::logln!(
        "[memory borrower storage] 128 pre-escrow rejections, 64 unused cancellations, 64 \
         heap-held two-reader relinks/detaches and 32 reader-preserving revocation/namespace \
         closes passed; capability/namespace payload maps remain separate"
    );
}

fn rejection_and_cancel() {
    let source = root();
    let target = root();
    let cap = allocate(source.id(), 1).unwrap();
    let used = crate::capability::node_admission_used();
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..128 {
        assert!(matches!(
            prepare_source_transfer_with(source.id(), cap, target.id(), Some(false), false, || {
                assert!(MEMORY_OBJECTS.try_lock().is_some());
                assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
                assert!(PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some());
                assert!(!info(source.id(), cap).unwrap().lent);
                // Admission owns exactly one additional destination count before
                // entering the payload allocator. No source escrow/pin yet.
                assert!(crate::capability::node_admission_used() > used);
                write_bytes(source.id(), cap, &[0x5a]).unwrap();
                Err(MemoryObjectError::ResourceLimit)
            }),
            Err(MemoryObjectError::ResourceLimit)
        ));
        assert_eq!(crate::capability::node_admission_used(), used);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
        assert!(!info(source.id(), cap).unwrap().lent);
        write_bytes(source.id(), cap, &[0xa5]).unwrap();
    }
    for _ in 0..64 {
        let prepared = prepare_loan(source.id(), cap, target.id(), false).unwrap();
        assert!(prepared.borrower_storage.is_some());
        let destination = prepared.target_cap();
        assert!(!crate::capability::contains(
            target.id(),
            destination,
            crate::capability::ObjectKind::Memory
        ));
        drop(prepared);
        assert_eq!(crate::capability::node_admission_used(), used);
        write_bytes(source.id(), cap, &[0x3c]).unwrap();
    }
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    close_cap(source.id(), cap).unwrap();
    memory::close_user_address_space_handle(target).unwrap();
    memory::close_user_address_space_handle(source).unwrap();
}

fn relinking() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        let first = BorrowerStorage::try_new().unwrap();
        let second = BorrowerStorage::try_new().unwrap();
        let mut state = LendState::None;
        let (first, second) = {
            let _registry = MEMORY_OBJECTS.lock();
            let _heap = PRIMARY_ALLOCATOR.lock();
            // Isolated scalar records exercise the exact production publisher,
            // without publishing synthetic shared capability authority.
            state.publish_read(first, 1, 11);
            state.publish_read(second, 2, 22);
            assert!(state.references_cap(1, 11));
            assert!(state.references_cap(2, 22));
            let LendState::Read {
                borrowers,
            } = &mut state
            else {
                unreachable!()
            };
            let first = borrowers.take(&1).unwrap();
            assert_eq!(borrowers.get(&2).unwrap().cap, 22);
            let second = borrowers.take(&2).unwrap();
            (first, second)
        };
        first.release();
        second.release();
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
}

fn readers_and_namespace_close() {
    let source = root();
    let target = root();
    let used = crate::capability::node_admission_used();
    for _ in 0..32 {
        let peer = root();
        let cap = allocate(source.id(), 1).unwrap();
        let first = lend_read(source.id(), cap, target.id()).unwrap();
        let second = lend_read(source.id(), cap, peer.id()).unwrap();
        map_any(target.id(), first, false).unwrap();
        revoke_lend(source.id(), cap, target.id(), first).unwrap();
        assert!(info(peer.id(), second).unwrap().lent);
        assert_eq!(write_bytes(source.id(), cap, &[1]), Err(MemoryObjectError::LendingActive));
        // Unmapped borrower retirement detaches and disposes its original node
        // through the production namespace path, without a revocation replay.
        memory::close_user_address_space_handle(peer).unwrap();
        assert!(!info(source.id(), cap).unwrap().lent);
        write_bytes(source.id(), cap, &[2]).unwrap();
        close_cap(source.id(), cap).unwrap();
        assert_eq!(crate::capability::node_admission_used(), used);
        assert_eq!(memory::budget::used(source), memory::budget::Amount::default());
    }
    memory::close_user_address_space_handle(target).unwrap();
    memory::close_user_address_space_handle(source).unwrap();
}
