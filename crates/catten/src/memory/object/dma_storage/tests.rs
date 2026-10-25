//! Memory-pin controls only: no IOMMU descriptor, command or hardware claim.
use super::*;
use crate::memory::{
    self,
    AddressSpaceHandle,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}
fn rw(exclusive: bool) -> Access {
    Access {
        reads: true,
        writes: true,
        exclusive,
    }
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
fn counts(object: MemoryObjectId) -> (usize, usize) {
    let registry = MEMORY_OBJECTS.lock();
    let entry = &registry.objects[&object];
    (entry.dma_pins, entry.exclusive_dma_pins)
}

pub(crate) fn run() {
    rejected_prefixes();
    changed_source();
    reused_generation();
    publication();
    retention();
    crate::logln!(
        "[DMA snapshot storage] 128 unlocked shared/exclusive prefix rejections, invalid/overflow \
         rejection, source close/CPU-map/DMA/copy/loan and real ASID-reuse revalidation, 64 \
         heap-held fills/dual-counter publications and snapshot-before-unpin disposal passed; \
         guarded exclusive pin/snapshot/data page/original charge retained past source-root \
         retirement and ASID reuse; no hardware claim or retained root"
    );
}

fn rejected_prefixes() {
    let handle = root();
    let cap = allocate(handle.id(), 2).unwrap();
    let object = id(handle, cap);
    let used = memory::budget::used(handle);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let authority = crate::capability::node_admission_used();
    for reject in 0..2 {
        for repeat in 0..64 {
            let result = pin_with(handle.id(), cap, rw(repeat % 2 == 0), |pages| {
                Preparing::with(pages, |stage| {
                    available();
                    assert_eq!(pages, 2);
                    assert_eq!(counts(object), (0, 0));
                    write_bytes(handle.id(), cap, &[0x5a]).unwrap();
                    if stage == reject {
                        Err(MemoryObjectError::ResourceLimit)
                    } else {
                        Ok(())
                    }
                })
            });
            assert_eq!(result.err(), Some(MemoryObjectError::ResourceLimit));
            assert_eq!(counts(object), (0, 0));
            assert_eq!(memory::budget::used(handle), used);
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
            assert_eq!(crate::capability::node_admission_used(), authority);
        }
    }
    assert_eq!(
        pin_with(
            handle.id(),
            cap,
            Access {
                reads: false,
                writes: false,
                exclusive: true
            },
            |_| panic!("invalid direction entered factory")
        )
        .err(),
        Some(MemoryObjectError::MissingRight)
    );
    assert_eq!(
        pin_with(handle.id(), cap, rw(true), |_| Err(MemoryObjectError::FrameAllocFailed)).err(),
        Some(MemoryObjectError::FrameAllocFailed)
    );
    assert_eq!(Preparing::new(usize::MAX).err(), Some(MemoryObjectError::FrameAllocFailed));
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn changed_source() {
    let handle = root();
    let cap = allocate(handle.id(), 1).unwrap();
    let result = pin_with(handle.id(), cap, rw(true), |pages| {
        let prepared = Preparing::new(pages)?;
        close_cap(handle.id(), cap).unwrap();
        Ok(prepared)
    });
    assert_eq!(result.err(), Some(MemoryObjectError::UnknownCapability));
    let cap = allocate(handle.id(), 1).unwrap();
    let object = id(handle, cap);
    let base = VAddr::from(0x260000usize);
    let result = pin_with(handle.id(), cap, rw(true), |pages| {
        let prepared = Preparing::new(pages)?;
        map(handle.id(), cap, base, true).unwrap();
        Ok(prepared)
    });
    assert_eq!(result.err(), Some(MemoryObjectError::LendingActive));
    assert_eq!(counts(object), (0, 0));
    unmap(handle.id(), cap).unwrap();
    let mut other = None;
    let result = pin_with(handle.id(), cap, rw(true), |pages| {
        let prepared = Preparing::new(pages)?;
        other = Some(pin_for_dma(handle.id(), cap, true, false, false)?);
        Ok(prepared)
    });
    assert_eq!(result.err(), Some(MemoryObjectError::LendingActive));
    assert_eq!(counts(object), (1, 0));
    unpin_dma(other.take().unwrap());
    let mut copy = None;
    let result = pin_with(handle.id(), cap, rw(false), |pages| {
        let prepared = Preparing::new(pages)?;
        copy = Some(pin_for_copy(handle.id(), cap)?);
        Ok(prepared)
    });
    assert_eq!(result.err(), Some(MemoryObjectError::LendingActive));
    assert_eq!(counts(object), (0, 0));
    unpin_copy(copy.take().unwrap());
    let borrower = root();
    let mut loan = None;
    let result = pin_with(handle.id(), cap, rw(false), |pages| {
        let prepared = Preparing::new(pages)?;
        loan = Some(lend_read(handle.id(), cap, borrower.id())?);
        Ok(prepared)
    });
    assert_eq!(result.err(), Some(MemoryObjectError::LendingActive));
    assert_eq!(counts(object), (0, 0));
    revoke_lend(handle.id(), cap, borrower.id(), loan.unwrap()).unwrap();
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(borrower).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn reused_generation() {
    let old = root();
    let cap = allocate(old.id(), 1).unwrap();
    let mut successor = None;
    let result = pin_with(old.id(), cap, rw(true), |pages| {
        let prepared = Preparing::new(pages)?;
        close_cap(old.id(), cap).unwrap();
        memory::close_user_address_space_handle(old).unwrap();
        let fresh = root();
        assert_eq!(fresh.id(), old.id());
        assert_ne!(fresh, old);
        successor = Some((fresh, allocate(fresh.id(), 1).unwrap()));
        Ok(prepared)
    });
    assert_eq!(result.err(), Some(MemoryObjectError::AddressSpaceMissing));
    let (fresh, cap) = successor.unwrap();
    let object = id(fresh, cap);
    assert_eq!(counts(object), (0, 0));
    write_bytes(fresh.id(), cap, &[0x7b]).unwrap();
    close_cap(fresh.id(), cap).unwrap();
    memory::close_user_address_space_handle(fresh).unwrap();
}

fn publication() {
    let handle = root();
    let cap = allocate(handle.id(), 35).unwrap();
    let object = id(handle, cap);
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for repeat in 0..64 {
        let access = rw(repeat % 2 == 0);
        let mut prepared = Preparing::new(35).unwrap();
        let pin = {
            let mut registry = MEMORY_OBJECTS.lock();
            let _heap = PRIMARY_ALLOCATOR.lock();
            assert_eq!(admission(&registry, handle.id(), cap, access), Ok((object, 35)));
            let pin = publish(&mut prepared, &mut registry, object, access);
            assert_eq!(pin.frames(), registry.objects[&object].frames.as_slice());
            assert_eq!(registry.objects[&object].dma_pins, 1);
            assert_eq!(registry.objects[&object].exclusive_dma_pins, usize::from(access.exclusive));
            pin
        };
        prepared.finish();
        unpin_dma_with(pin, || {
            available();
            assert_eq!(counts(object), (1, usize::from(access.exclusive)));
            assert_eq!(close_cap(handle.id(), cap), Err(MemoryObjectError::LendingActive));
        });
        assert_eq!(counts(object), (0, 0));
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn retention() {
    let handle = root();
    let cap = allocate(handle.id(), 1).unwrap();
    let object = id(handle, cap);
    let pin = pin_for_dma(handle.id(), cap, true, true, true).unwrap();
    let used = memory::budget::used(handle);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    {
        let _registry = MEMORY_OBJECTS.lock();
        let _table = ADDRESS_SPACE_TABLE.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _physical = PHYSICAL_FRAME_ALLOCATOR.lock();
        drop(pin);
    }
    assert_eq!(counts(object), (1, 1));
    assert_eq!(close_cap(handle.id(), cap), Err(MemoryObjectError::LendingActive));
    assert_eq!(memory::budget::used(handle), used);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    // This pin was never handed to hardware. It owns backing, not a root lease.
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(memory::budget::used(handle), used);
    assert_eq!(counts(object), (1, 1));
    assert!(MEMORY_OBJECTS.lock().objects[&object].destroy_when_unpinned);
    let successor = root();
    assert_eq!(successor.id(), handle.id());
    assert_ne!(successor, handle);
    assert_eq!(memory::budget::used(successor), memory::budget::Amount::default());
    let fresh = allocate(successor.id(), 1).unwrap();
    let fresh_pin = pin_for_dma(successor.id(), fresh, true, true, true).unwrap();
    assert_ne!(fresh_pin.object_id(), object);
    unpin_dma(fresh_pin);
    assert_eq!(counts(object), (1, 1));
    write_bytes(successor.id(), fresh, &[0x9c]).unwrap();
    close_cap(successor.id(), fresh).unwrap();
    memory::close_user_address_space_handle(successor).unwrap();
    assert_eq!(memory::budget::used(handle), used);
}
