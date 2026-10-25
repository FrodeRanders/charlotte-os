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
pub(super) fn available() {
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
    admission::tests::run();
    rejected_prefixes();
    lost_source();
    reused_generation();
    ordinary_disposal();
    metadata_publication();
    guarded_preparation();
    rollback_record_lifetime();
    uncertain_rollback();
    crate::logln!(
        "[memory mapping storage] 192 unlocked record/snapshot prefix rejections, source-close \
         revalidation, 64 heap-held snapshot fills/node publications/detachments, 32 rollback \
         receipts through confirmed maintenance, and guarded uncertain rollback passed; two data \
         pages, their mapping node/root and original charge deliberately retained"
    );
    crate::logln!(
        "[mapping preparation ownership] captured generation rejects real ASID reuse; 64 ordinary \
         success/rollback disposal entries outside memory/table/heap/frame guards; guarded \
         partial/full/published preparation retains two empty nodes and two snapshots, no \
         additional data/root/lease; existing uncertain rollback remains separately retained"
    );
}

fn rejected_prefixes() {
    let admitted = snapshot_storage::admitted();
    let nodes = admission::snapshot();
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
                    (
                        cleanup,
                        |handle, pages| {
                            Preparing::with(handle, pages, |stage| {
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
                        },
                        |_| {},
                    ),
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
            assert_eq!(snapshot_storage::admitted(), admitted);
            assert_eq!(admission::snapshot(), nodes);
        }
    }
    assert_eq!(Preparing::new(handle, usize::MAX).err(), Some(MemoryObjectError::FrameAllocFailed));
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
            (
                cleanup,
                |handle, pages| {
                    let prepared = Preparing::new(handle, pages)?;
                    available();
                    close_cap(handle.id(), cap).unwrap();
                    Ok(prepared)
                },
                |prepared| {
                    available();
                    assert!(prepared.frames().is_empty());
                },
            ),
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
        let mut prepared = Preparing::new(handle, 35).unwrap();
        let pin = {
            let mut registry = MEMORY_OBJECTS.lock();
            let _heap = PRIMARY_ALLOCATOR.lock();
            let mut pin = admission::with_locked(|| {
                snapshot_storage::with_budget_locked(|| {
                    prepared.publish(
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
                    )
                })
            });
            assert_eq!(prepared.frames(), registry.objects[&object].frames.as_slice());
            pin.retired_mapping =
                registry.objects.get_mut(&object).unwrap().mappings.take(&handle.id());
            pin
        };
        assert_eq!(pin.retired_mapping.as_ref().unwrap().value().1.state.installed_pages, 0);
        // No leaves were installed; this isolated metadata owner needs no TLB
        // operation. Its explicit release consumes the original pin/node once.
        prepared.finish();
        pin.release(None);
        assert_eq!(MEMORY_OBJECTS.lock().objects[&object].retirement_pins, 0);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn reused_generation() {
    let old = root();
    let cap = allocate(old.id(), 1).unwrap();
    let mut successor = None;
    let mut pin = None;
    // This private serialized adapter deliberately retires a pin-free fixture.
    // Public mapping callers keep their original root lease throughout.
    let result = map_locked_with_preparation(
        old.id(),
        cap,
        VAddr::from(0x280000usize),
        true,
        false,
        &mut pin,
        (
            cleanup,
            |handle, pages| {
                let prepared = Preparing::new(handle, pages)?;
                close_cap(old.id(), cap).unwrap();
                memory::close_user_address_space_handle(old).unwrap();
                let fresh = root();
                assert_eq!(fresh.id(), old.id());
                assert_ne!(fresh, old);
                successor = Some((fresh, allocate(fresh.id(), 1).unwrap()));
                Ok(prepared)
            },
            |prepared| {
                available();
                assert!(prepared.frames().is_empty());
            },
        ),
    );
    assert_eq!(result, Err(MemoryObjectError::AddressSpaceMissing));
    assert!(pin.is_none());
    let (fresh, cap) = successor.unwrap();
    assert!(!info(fresh.id(), cap).unwrap().mapped);
    write_bytes(fresh.id(), cap, &[0x7b]).unwrap();
    close_cap(fresh.id(), cap).unwrap();
    memory::close_user_address_space_handle(fresh).unwrap();
}

fn ordinary_disposal() {
    let handle = root();
    let cap = allocate(handle.id(), 2).unwrap();
    let foreign = allocate(handle.id(), 1).unwrap();
    let object = MEMORY_OBJECTS.lock().lookup(handle.id(), cap).unwrap().object;
    let base = VAddr::from(0x280000usize);
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for rollback in [false, true] {
        if rollback {
            map(handle.id(), foreign, base + PAGE_SIZE, true).unwrap();
        }
        for _ in 0..32 {
            with_address_space_operation(handle.id(), || {
                let mut pin = None;
                let result = map_locked_with_preparation(
                    handle.id(),
                    cap,
                    base,
                    true,
                    false,
                    &mut pin,
                    (cleanup, Preparing::new, |prepared| {
                        available();
                        assert_eq!(prepared.frames().len(), 2);
                        assert_eq!(MEMORY_OBJECTS.lock().objects[&object].retirement_pins, 1);
                        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
                    }),
                );
                assert_eq!(
                    result,
                    if rollback {
                        Err(MemoryObjectError::MapFailed)
                    } else {
                        Ok(())
                    }
                );
                let result = finish_mapping(
                    handle.id(),
                    base,
                    2,
                    false,
                    result,
                    pin,
                    crate::cpu::isa::memory::tlb::try_inval_range_user,
                );
                assert_eq!(
                    result,
                    if rollback {
                        Err(MemoryObjectError::MapFailed)
                    } else {
                        Ok(())
                    }
                );
                Ok(())
            })
            .unwrap();
            if !rollback {
                unmap(handle.id(), cap).unwrap();
            }
        }
        if rollback {
            unmap(handle.id(), foreign).unwrap();
        }
    }
    close_cap(handle.id(), foreign).unwrap();
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn guarded_preparation() {
    let admitted = snapshot_storage::admitted();
    let nodes = admission::snapshot();
    let handle = root();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    // One partial empty node and one complete unused node/snapshot pair. No
    // source identities or pins exist, so the fixture root is unrelated to them.
    let partial = Preparing(ManuallyDrop::new(Resources {
        node: Some(admission::prepare(false).unwrap()),
        snapshot: None,
    }));
    let full = Preparing::new(handle, 35).unwrap();
    {
        let _registry = MEMORY_OBJECTS.lock();
        let _table = ADDRESS_SPACE_TABLE.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _physical = PHYSICAL_FRAME_ALLOCATOR.lock();
        admission::with_locked(|| {
            snapshot_storage::with_budget_locked(|| {
                drop(partial);
                drop(full);
            })
        });
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    let cap = allocate(handle.id(), 35).unwrap();
    let object = MEMORY_OBJECTS.lock().lookup(handle.id(), cap).unwrap().object;
    let mut prepared = Preparing::new(handle, 35).unwrap();
    let mut pin = {
        let mut registry = MEMORY_OBJECTS.lock();
        prepared.publish(
            &mut registry,
            object,
            handle.id(),
            MemoryMappingState {
                address_space: handle,
                base: VAddr::from(0x280000usize),
                installed_pages: 0,
                writable: false,
                scratch: false,
            },
        )
    };
    {
        let mut registry = MEMORY_OBJECTS.lock();
        let _table = ADDRESS_SPACE_TABLE.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _physical = PHYSICAL_FRAME_ALLOCATOR.lock();
        assert_eq!(prepared.frames().len(), 35);
        pin.retired_mapping =
            registry.objects.get_mut(&object).unwrap().mappings.take(&handle.id());
        admission::with_locked(|| snapshot_storage::with_budget_locked(|| drop(prepared)));
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    // No leaves or externally owned mapping were installed by this isolated
    // metadata fixture. Explicitly consume that pin/node, then close backing.
    pin.release(None);
    assert_eq!(MEMORY_OBJECTS.lock().objects[&object].retirement_pins, 0);
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(
        snapshot_storage::admitted(),
        ([admitted.0[0] + 70 * 8, admitted.0[1] + 2], [admitted.1[0] + 70 * 8, admitted.1[1] + 2])
    );
    let bytes = 2 * Storage::allocation_bytes() as u64;
    assert_eq!(
        admission::snapshot(),
        ([nodes.0[0] + bytes, nodes.0[1] + 2], [nodes.1[0] + bytes, nodes.1[1] + 2])
    );
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
    let nodes = admission::snapshot();
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
        admission::with_locked(|| {
            finish_mapping(handle.id(), base, 2, false, result, pin, |_, _, _| Err(cause))
        })
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
    let bytes = Storage::allocation_bytes() as u64;
    assert_eq!(
        admission::snapshot(),
        ([nodes.0[0] + bytes, nodes.0[1] + 1], [nodes.1[0] + bytes, nodes.1[1] + 1])
    );
}
