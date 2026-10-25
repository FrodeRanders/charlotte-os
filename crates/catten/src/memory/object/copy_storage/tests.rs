//! Serialized source changes across the pin-free factory interval and inert fallback.
use super::*;
use crate::memory::{
    self,
    AddressSpaceHandle,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}
fn available() {
    assert!(MEMORY_OBJECTS.try_lock().is_some());
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some());
}
fn id(handle: AddressSpaceHandle, cap: MemoryObjectCap) -> MemoryObjectId {
    MEMORY_OBJECTS.lock().lookup(handle.id(), cap).unwrap().object
}

pub(crate) fn run() {
    rejected_prefixes();
    changed_source();
    reused_generation();
    publication();
    retention();
    crate::logln!(
        "[copy snapshot storage] 128 unlocked prefix rejections, overflow rejection, source \
         close/write-map/DMA and real ASID-reuse revalidation, 64 heap-held snapshot fills/pin \
         publications and post-guard snapshot-before-unpin disposal passed; guarded fallback \
         retains one unused snapshot and one real pin/snapshot/data page/original charge past \
         source-root retirement and ASID reuse"
    );
}

fn rejected_prefixes() {
    let admitted = snapshot_storage::admitted();
    let handle = root();
    let cap = allocate(handle.id(), 2).unwrap();
    let object = id(handle, cap);
    let used = memory::budget::used(handle);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let authority = crate::capability::node_admission_used();
    for reject in 0..2 {
        for _ in 0..64 {
            let result = pin_with(handle.id(), cap, |handle, pages| {
                Preparing::with(handle, pages, |stage| {
                    available();
                    assert_eq!(pages, 2);
                    assert_eq!(MEMORY_OBJECTS.lock().objects[&object].copy_pins, 0);
                    write_bytes(handle.id(), cap, &[0x5a]).unwrap();
                    if stage == reject {
                        Err(MemoryObjectError::ResourceLimit)
                    } else {
                        Ok(())
                    }
                })
            });
            assert_eq!(result.err(), Some(MemoryObjectError::ResourceLimit));
            assert_eq!(MEMORY_OBJECTS.lock().objects[&object].copy_pins, 0);
            assert_eq!(memory::budget::used(handle), used);
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
            assert_eq!(crate::capability::node_admission_used(), authority);
            assert_eq!(snapshot_storage::admitted(), admitted);
        }
    }
    assert_eq!(Preparing::new(handle, usize::MAX).err(), Some(MemoryObjectError::FrameAllocFailed));
    assert_eq!(
        pin_with(handle.id(), cap, |_, _| Err(MemoryObjectError::FrameAllocFailed)).err(),
        Some(MemoryObjectError::FrameAllocFailed)
    );
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn changed_source() {
    let handle = root();
    let cap = allocate(handle.id(), 1).unwrap();
    let result = pin_with(handle.id(), cap, |handle, pages| {
        let prepared = Preparing::new(handle, pages)?;
        available();
        close_cap(handle.id(), cap).unwrap();
        Ok(prepared)
    });
    assert_eq!(result.err(), Some(MemoryObjectError::UnknownCapability));
    assert_eq!(memory::budget::used(handle), memory::budget::Amount::default());

    let cap = allocate(handle.id(), 1).unwrap();
    let object = id(handle, cap);
    let base = VAddr::from(0x240000usize);
    let result = pin_with(handle.id(), cap, |handle, pages| {
        let prepared = Preparing::new(handle, pages)?;
        map(handle.id(), cap, base, true).unwrap();
        Ok(prepared)
    });
    assert_eq!(result.err(), Some(MemoryObjectError::AlreadyMapped));
    assert_eq!(MEMORY_OBJECTS.lock().objects[&object].copy_pins, 0);
    unmap(handle.id(), cap).unwrap();
    let mut dma = None;
    let result = pin_with(handle.id(), cap, |handle, pages| {
        let prepared = Preparing::new(handle, pages)?;
        dma = Some(pin_for_dma(handle.id(), cap, true, false, false)?);
        Ok(prepared)
    });
    assert_eq!(result.err(), Some(MemoryObjectError::LendingActive));
    assert_eq!(MEMORY_OBJECTS.lock().objects[&object].copy_pins, 0);
    unpin_dma(dma.take().unwrap());
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn reused_generation() {
    let old = root();
    let cap = allocate(old.id(), 1).unwrap();
    let mut successor = None;
    let result = pin_with(old.id(), cap, |handle, pages| {
        let prepared = Preparing::new(handle, pages)?;
        // No borrowed frames or source pin exist during preparation. This raw
        // serialized fixture can close and really reuse the entire namespace.
        close_cap(old.id(), cap).unwrap();
        memory::close_user_address_space_handle(old).unwrap();
        let fresh = root();
        assert_eq!(fresh.id(), old.id());
        assert_ne!(fresh, old);
        let fresh_cap = allocate(fresh.id(), 1).unwrap();
        successor = Some((fresh, fresh_cap));
        Ok(prepared)
    });
    assert_eq!(result.err(), Some(MemoryObjectError::AddressSpaceMissing));
    let (fresh, cap) = successor.unwrap();
    let object = id(fresh, cap);
    assert_eq!(MEMORY_OBJECTS.lock().objects[&object].copy_pins, 0);
    write_bytes(fresh.id(), cap, &[0x7b]).unwrap();
    close_cap(fresh.id(), cap).unwrap();
    memory::close_user_address_space_handle(fresh).unwrap();
}

fn publication() {
    let handle = root();
    let cap = allocate(handle.id(), 35).unwrap();
    let object = id(handle, cap);
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        let mut prepared = Preparing::new(handle, 35).unwrap();
        let pin = {
            let mut registry = MEMORY_OBJECTS.lock();
            let _heap = PRIMARY_ALLOCATOR.lock();
            assert_eq!(copy_admission(&registry, handle.id(), cap), Ok((object, 35)));
            let pin = snapshot_storage::with_budget_locked(|| {
                publish(&mut prepared, &mut registry, object)
            });
            assert_eq!(pin.frames.as_slice(), registry.objects[&object].frames.as_slice());
            assert_eq!(registry.objects[&object].copy_pins, 1);
            pin
        };
        prepared.finish();
        unpin_copy_with(pin, || {
            available();
            assert_eq!(MEMORY_OBJECTS.lock().objects[&object].copy_pins, 1);
            assert_eq!(write_bytes(handle.id(), cap, &[1]), Err(MemoryObjectError::LendingActive));
        });
        assert_eq!(MEMORY_OBJECTS.lock().objects[&object].copy_pins, 0);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn retention() {
    let admitted = snapshot_storage::admitted();
    let handle = root();
    let prepared = Preparing::new(handle, 35).unwrap();
    {
        let _registry = MEMORY_OBJECTS.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        snapshot_storage::with_budget_locked(|| drop(prepared));
    }
    let cap = allocate(handle.id(), 1).unwrap();
    let object = id(handle, cap);
    let pin = pin_for_copy(handle.id(), cap).unwrap();
    let used = memory::budget::used(handle);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    {
        let _registry = MEMORY_OBJECTS.lock();
        let _table = ADDRESS_SPACE_TABLE.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _physical = PHYSICAL_FRAME_ALLOCATOR.lock();
        snapshot_storage::with_budget_locked(|| drop(pin));
    }
    assert_eq!(MEMORY_OBJECTS.lock().objects[&object].copy_pins, 1);
    assert_eq!(close_cap(handle.id(), cap), Err(MemoryObjectError::LendingActive));
    assert_eq!(memory::budget::used(handle), used);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    // An unmapped copy pin owns backing, not a root lease. Logical namespace
    // teardown may retire this source while its original backing stays charged.
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(memory::budget::used(handle), used);
    {
        let registry = MEMORY_OBJECTS.lock();
        assert_eq!(registry.objects[&object].copy_pins, 1);
        assert!(registry.objects[&object].destroy_when_unpinned);
        assert_eq!(registry.objects[&object].frames.len(), 1);
    }
    let successor = root();
    assert_eq!(successor.id(), handle.id());
    assert_ne!(successor, handle);
    assert_eq!(memory::budget::used(successor), memory::budget::Amount::default());
    assert_eq!(
        pin_with(successor.id(), cap, |_, _| panic!("retired source reached factory")).err(),
        Some(MemoryObjectError::UnknownCapability)
    );
    let fresh = allocate(successor.id(), 1).unwrap();
    // Capability scalars may be reused inside the new namespace. Its actual
    // owner is the new captured generation/object, not the old integer's history.
    let fresh_pin = pin_for_copy(successor.id(), fresh).unwrap();
    assert_ne!(fresh_pin.object, object);
    assert_eq!(MEMORY_OBJECTS.lock().objects[&object].copy_pins, 1);
    unpin_copy(fresh_pin);
    assert_eq!(memory::budget::used(handle), used);
    write_bytes(successor.id(), fresh, &[0x9c]).unwrap();
    close_cap(successor.id(), fresh).unwrap();
    memory::close_user_address_space_handle(successor).unwrap();
    assert_eq!(memory::budget::used(handle), used);
    let after = snapshot_storage::admitted();
    assert_eq!(
        after,
        ([admitted.0[0] + 36 * 8, admitted.0[1] + 2], [admitted.1[0] + 36 * 8, admitted.1[1] + 2])
    );
}
