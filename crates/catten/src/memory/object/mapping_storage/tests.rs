//! Serialized map preparation/revalidation and metadata-publication controls.
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
fn cleanup(
    space: &mut crate::cpu::isa::memory::paging::AddressSpace,
    base: VAddr,
    frame: PAddr,
) -> bool {
    unmap_owned_leaf(space, base, frame).is_ok()
}

pub(crate) fn run() {
    rejected_prefixes();
    lost_source();
    metadata_publication();
    rollback_record_lifetime();
    uncertain_rollback();
    crate::logln!(
        "[memory mapping storage] 192 unlocked record/snapshot prefix rejections, source-close \
         revalidation, 64 heap-held snapshot fills/node publications/detachments, 32 rollback \
         receipts through confirmed maintenance, and guarded uncertain rollback passed; two data \
         pages, their mapping node/root and original charge deliberately retained"
    );
}

fn rejected_prefixes() {
    let handle = root();
    let cap = allocate(handle.id(), 2).unwrap();
    let id = MEMORY_OBJECTS.lock().lookup(handle.id(), cap).unwrap().object;
    let used = memory::budget::used(handle);
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let counts = crate::capability::node_admission_used();
    for reject in 0..3 {
        for _ in 0..64 {
            let base = reserve_scratch(handle.id(), 2).unwrap();
            with_address_space_operation(handle.id(), || {
                let mut pin = None;
                let result = map_locked_with_preparation(
                    handle.id(),
                    cap,
                    base,
                    true,
                    true,
                    &mut pin,
                    (cleanup, |pages| {
                        Preparing::with(pages, |stage| {
                            available();
                            assert_eq!(pages, 2);
                            assert_eq!(MEMORY_OBJECTS.lock().objects[&id].retirement_pins, 0);
                            write_bytes(handle.id(), cap, &[0x5a]).unwrap();
                            if stage == reject {
                                Err(MemoryObjectError::ResourceLimit)
                            } else {
                                Ok(())
                            }
                        })
                    }),
                );
                assert!(pin.is_none());
                assert_eq!(
                    finish_mapping(handle.id(), base, 2, true, result, pin, |_, _, _| panic!(
                        "unpublished mapping entered maintenance"
                    )),
                    Err(MemoryObjectError::ResourceLimit)
                );
                Ok(())
            })
            .unwrap();
            let next = reserve_scratch(handle.id(), 2).unwrap();
            assert_eq!(next, base);
            release_scratch(handle.id(), next, 2).unwrap();
            assert!(!info(handle.id(), cap).unwrap().mapped);
            assert_eq!(memory::budget::used(handle), used);
            assert_eq!(crate::capability::node_admission_used(), counts);
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
        }
    }
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn lost_source() {
    let handle = root();
    let cap = allocate(handle.id(), 1).unwrap();
    let result = with_address_space_operation(handle.id(), || {
        let mut pin = None;
        let result = map_locked_with_preparation(
            handle.id(),
            cap,
            VAddr::from(0x1c0000usize),
            true,
            false,
            &mut pin,
            (cleanup, |pages| {
                let prepared = Preparing::new(pages)?;
                available();
                close_cap(handle.id(), cap).unwrap();
                Ok(prepared)
            }),
        );
        assert!(pin.is_none());
        result
    });
    assert_eq!(result, Err(MemoryObjectError::UnknownCapability));
    assert_eq!(memory::budget::used(handle), memory::budget::Amount::default());
    memory::close_user_address_space_handle(handle).unwrap();
}

fn metadata_publication() {
    let handle = root();
    let cap = allocate(handle.id(), 35).unwrap();
    let object = MEMORY_OBJECTS.lock().lookup(handle.id(), cap).unwrap().object;
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        let mut prepared = Preparing::new(35).unwrap();
        let pin = {
            let mut registry = MEMORY_OBJECTS.lock();
            let _heap = PRIMARY_ALLOCATOR.lock();
            let mut pin = prepared.publish(
                &mut registry,
                object,
                handle.id(),
                MemoryMappingState {
                    address_space: handle,
                    base: VAddr::from(0x1c0000usize),
                    installed_pages: 0,
                    writable: false,
                    scratch: false,
                },
            );
            assert_eq!(prepared.frames.as_slice(), registry.objects[&object].frames.as_slice());
            pin.retired_mapping =
                registry.objects.get_mut(&object).unwrap().mappings.take(&handle.id());
            pin
        };
        assert_eq!(pin.retired_mapping.as_ref().unwrap().value().1.state.installed_pages, 0);
        // No leaves were installed; this isolated metadata owner needs no TLB
        // operation. Its explicit release consumes the original pin/node once.
        drop(prepared);
        pin.release(None);
        assert_eq!(MEMORY_OBJECTS.lock().objects[&object].retirement_pins, 0);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn rollback_record_lifetime() {
    let handle = root();
    let cap = allocate(handle.id(), 2).unwrap();
    let foreign = allocate(handle.id(), 1).unwrap();
    let base = VAddr::from(0x1e0000usize);
    map(handle.id(), foreign, base + PAGE_SIZE, true).unwrap();
    for _ in 0..32 {
        with_address_space_operation(handle.id(), || {
            let mut pin = None;
            let result = map_locked(handle.id(), cap, base, true, false, &mut pin);
            assert_eq!(result, Err(MemoryObjectError::MapFailed));
            assert_eq!(
                pin.as_ref()
                    .unwrap()
                    .retired_mapping
                    .as_ref()
                    .unwrap()
                    .value()
                    .1
                    .state
                    .installed_pages,
                1
            );
            let object = pin.as_ref().unwrap().object;
            assert_eq!(MEMORY_OBJECTS.lock().objects[&object].retirement_pins, 1);
            assert_eq!(
                finish_mapping(
                    handle.id(),
                    base,
                    2,
                    false,
                    result,
                    pin,
                    crate::cpu::isa::memory::tlb::try_inval_range_user
                ),
                Err(MemoryObjectError::MapFailed)
            );
            assert_eq!(MEMORY_OBJECTS.lock().objects[&object].retirement_pins, 0);
            Ok(())
        })
        .unwrap();
    }
    unmap(handle.id(), foreign).unwrap();
    close_cap(handle.id(), foreign).unwrap();
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

// Deliberate failed confirmation after real leaf rollback. Never re-adopt or
// release the retained pin/node; the source remains terminally fenced.
fn uncertain_rollback() {
    use crate::cpu::isa::memory::tlb::InvalidationError;
    let handle = root();
    let cap = allocate(handle.id(), 2).unwrap();
    let foreign = allocate(handle.id(), 1).unwrap();
    let base = VAddr::from(0x200000usize);
    map(handle.id(), foreign, base + PAGE_SIZE, true).unwrap();
    #[cfg(target_arch = "aarch64")]
    let cause = InvalidationError::AddressSpaceMissing;
    #[cfg(target_arch = "x86_64")]
    let cause = InvalidationError::Busy;
    let object = MEMORY_OBJECTS.lock().lookup(handle.id(), cap).unwrap().object;
    let result = with_address_space_operation(handle.id(), || {
        let mut pin = None;
        let result = map_locked(handle.id(), cap, base, true, false, &mut pin);
        assert_eq!(result, Err(MemoryObjectError::MapFailed));
        let state = pin.as_ref().unwrap().retired_mapping.as_ref().unwrap().value().1.state;
        assert_eq!(state.installed_pages, 1);
        assert_eq!(state.address_space, handle);
        let _registry = MEMORY_OBJECTS.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        // Confirmation rejects once; complete pin/node fallback must neither
        // enter either allocator/registry nor reclaim the original backing.
        finish_mapping(handle.id(), base, 2, false, result, pin, |_, _, _| Err(cause))
    });
    assert_eq!(result, Err(MemoryObjectError::InvalidationFailed(cause)));
    assert_eq!(MEMORY_OBJECTS.lock().objects[&object].retirement_pins, 1);
    assert!(info(handle.id(), foreign).unwrap().mapped);
    unmap(handle.id(), foreign).unwrap();
    close_cap(handle.id(), foreign).unwrap();
    assert_eq!(close_cap(handle.id(), cap), Err(MemoryObjectError::LendingActive));
    assert_eq!(
        memory::budget::used(handle),
        memory::budget::Amount {
            pages: 2,
            objects: 1
        }
    );
    assert_eq!(
        memory::close_user_address_space_handle(handle),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    assert_eq!(
        memory::budget::used(handle),
        memory::budget::Amount {
            pages: 2,
            objects: 1
        }
    );
}
