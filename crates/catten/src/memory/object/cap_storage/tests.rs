//! Serialized storage-prefix and complete publication/retirement controls.
use super::*;
use crate::memory::{
    self,
    AddressSpaceHandle,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}

pub(crate) fn run() {
    rejected_prefixes();
    ordinary_cancellation();
    containing_rejections();
    publication_and_detachment();
    stale_namespace();
    late_cancellation();
    admission::tests::run();
    guarded_abandonment();
    crate::logln!(
        "[memory capability storage] 192 charged preparation-prefix rejections, 64 heap-held \
         two-capability/namespace publications and paired detachments, post-guard count recovery \
         and stale captured-generation rejection passed"
    );
    crate::logln!(
        "[memory capability preparation] inert partial/full/published ownership; 64 explicit \
         storage-before-authority finishes, 64 local-guard rejection finishes and 192 containing \
         move/loan/copy rejections passed; guarded fallback retains four unused metadata nodes \
         and captured authority/account references, no data/root/pin; namespace retirement may \
         dispose hidden authority, retained payload metadata keeps original byte/count admission"
    );
}

pub(super) fn available() {
    assert!(MEMORY_OBJECTS.try_lock().is_some());
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some());
    crate::capability::admission_tests::test_with_registry_locked(|| {});
}

fn ordinary_cancellation() {
    let handle = root();
    let counts = crate::capability::node_admission_used();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        let owner = Preparing::new(admit_capability(handle.id(), handle).unwrap(), handle).unwrap();
        owner.finish_with(|fields| {
            available();
            assert!(fields.payload.is_none() && fields.namespace.is_none());
            assert!(fields.authority.is_some());
            assert_eq!(crate::capability::node_admission_used().0, counts.0 + 1);
        });
        assert_eq!(crate::capability::node_admission_used(), counts);
        let owner = Preparing::new(admit_capability(handle.id(), handle).unwrap(), handle).unwrap();
        let result = owner.try_prepare::<()>(|_| {
            let _registry = MEMORY_OBJECTS.lock();
            let _table = ADDRESS_SPACE_TABLE.lock();
            let _heap = PRIMARY_ALLOCATOR.lock();
            let _physical = PHYSICAL_FRAME_ALLOCATOR.lock();
            Err(MemoryObjectError::ResourceLimit)
        });
        assert!(matches!(result, Err(MemoryObjectError::ResourceLimit)));
        available();
        assert_eq!(crate::capability::node_admission_used(), counts);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    memory::close_user_address_space_handle(handle).unwrap();
}

fn containing_rejections() {
    let handle = root();
    let target = root();
    let cap = allocate(handle.id(), 1).unwrap();
    let counts = crate::capability::node_admission_used();
    let used = memory::budget::used(handle);
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..64 {
        assert!(matches!(
            prepare_move(handle.id(), u64::MAX, target.id()),
            Err(MemoryObjectError::UnknownCapability)
        ));
        assert!(matches!(
            prepare_source_transfer_with(handle.id(), cap, target.id(), Some(false), false, |_| {
                Err(MemoryObjectError::ResourceLimit)
            }),
            Err(MemoryObjectError::ResourceLimit)
        ));
        assert!(matches!(
            prepare_copy_with(handle.id(), cap, target.id(), |_, _| Err(
                MemoryObjectError::FrameAllocFailed
            )),
            Err(MemoryObjectError::FrameAllocFailed)
        ));
        assert_eq!(crate::capability::node_admission_used(), counts);
        assert_eq!(memory::budget::used(handle), used);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
        {
            let registry = MEMORY_OBJECTS.lock();
            let object = registry.lookup(handle.id(), cap).unwrap().object;
            assert_eq!(registry.objects[&object].copy_pins, 0);
        }
        write_bytes(handle.id(), cap, &[0x5a]).unwrap();
    }
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(target).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn late_cancellation() {
    let old = root();
    let counts = crate::capability::node_admission_used();
    let owner = Preparing::new(admit_capability(old.id(), old).unwrap(), old).unwrap();
    assert_eq!(crate::capability::node_admission_used().0, counts.0 + 1);
    memory::close_user_address_space_handle(old).unwrap();
    // Namespace teardown owns removal of its hidden staged authority. The
    // preparation still owns unused payload storage and the old account Arc.
    assert_eq!(crate::capability::node_admission_used(), counts);
    let fresh = root();
    assert_eq!(fresh.id(), old.id());
    assert_ne!(fresh, old);
    let cap = allocate(fresh.id(), 1).unwrap();
    let live = crate::capability::node_admission_used();
    owner.finish_with(|fields| {
        available();
        assert_eq!(fields.handle, old);
        assert_eq!(crate::capability::node_admission_used(), live);
    });
    assert_eq!(crate::capability::node_admission_used(), live);
    write_bytes(fresh.id(), cap, &[0x5a]).unwrap();
    close_cap(fresh.id(), cap).unwrap();
    memory::close_user_address_space_handle(fresh).unwrap();
    assert_eq!(crate::capability::node_admission_used(), counts);
}

fn guarded_abandonment() {
    let handle = root();
    let counts = crate::capability::node_admission_used();
    let nodes = admission::snapshot();
    let retained = |payloads: u64, namespaces: u64| {
        let bytes = payloads * Storage::allocation_bytes() as u64
            + namespaces * NamespaceStorage::allocation_bytes() as u64;
        (
            [nodes.0[0] + bytes, nodes.0[1] + payloads + namespaces],
            [nodes.1[0] + bytes, nodes.1[1] + payloads + namespaces],
        )
    };
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let partial = Preparing(ManuallyDrop::new(Resources {
        payload: Some(admission::prepare(false).unwrap()),
        namespace: None,
        authority: Some(admit_capability(handle.id(), handle).unwrap()),
        handle,
        platform: false,
    }));
    let full = Preparing::new(admit_capability(handle.id(), handle).unwrap(), handle).unwrap();
    let first_hidden = partial.identity();
    let second_hidden = full.identity();
    let mut local = MemoryObjectRegistry::new();
    let mut published =
        Preparing::new(admit_capability(handle.id(), handle).unwrap(), handle).unwrap();
    let mut replacement =
        Preparing::new(admit_capability(handle.id(), handle).unwrap(), handle).unwrap();
    published.publish().unwrap();
    published.install(&mut local, payload());
    replacement.publish().unwrap();
    replacement.install(&mut local, payload());
    assert!(replacement.0.namespace.is_some());
    {
        let _registry = MEMORY_OBJECTS.lock();
        let _table = ADDRESS_SPACE_TABLE.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        let _physical = PHYSICAL_FRAME_ALLOCATOR.lock();
        admission::with_locked(|| {
            crate::capability::admission_tests::test_with_registry_locked(|| {
                drop(partial);
                drop(full);
                drop(published);
                drop(replacement);
            });
        });
    }
    assert_eq!(admission::snapshot(), retained(4, 3));
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    assert_eq!(crate::capability::node_admission_used().0, counts.0 + 4);
    let namespace = retire_namespace(&mut local, handle.id()).unwrap();
    namespace.release();
    assert_eq!(admission::snapshot(), retained(2, 2));
    assert_eq!(crate::capability::node_admission_used().0, counts.0 + 2);
    for id in [first_hidden, second_hidden] {
        assert!(!crate::capability::contains(
            handle.id(),
            id,
            crate::capability::ObjectKind::Memory
        ));
    }
    // Existing namespace teardown can consume staged authority. The inert
    // payload preparations are not leases or retry owners; their unused nodes
    // and captured account references retain their original byte/count admission.
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(admission::snapshot(), retained(2, 2));
    assert_eq!(crate::capability::node_admission_used(), counts);
    let fresh = root();
    assert_eq!(fresh.id(), handle.id());
    assert_ne!(fresh, handle);
    let cap = allocate(fresh.id(), 1).unwrap();
    write_bytes(fresh.id(), cap, &[0x5a]).unwrap();
    close_cap(fresh.id(), cap).unwrap();
    memory::close_user_address_space_handle(fresh).unwrap();
    assert_eq!(admission::snapshot(), retained(2, 2));
    assert_eq!(crate::capability::node_admission_used(), counts);
}

fn rejected_prefixes() {
    let handle = root();
    let counts = crate::capability::node_admission_used();
    let nodes = admission::snapshot();
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for reject in 0..3 {
        for _ in 0..64 {
            assert!(matches!(
                Preparing::with(admit_capability(handle.id(), handle).unwrap(), handle, |stage| {
                    assert!(MEMORY_OBJECTS.try_lock().is_some());
                    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
                    assert!(PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some());
                    assert_eq!(crate::capability::node_admission_used().0, counts.0 + 1);
                    if stage == reject {
                        Err(MemoryObjectError::ResourceLimit)
                    } else {
                        Ok(())
                    }
                }),
                Err(MemoryObjectError::ResourceLimit)
            ));
            assert_eq!(crate::capability::node_admission_used(), counts);
            assert_eq!(admission::snapshot(), nodes);
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
            assert!(MEMORY_OBJECTS.lock().caps.get(&handle.id()).is_none());
        }
    }
    memory::close_user_address_space_handle(handle).unwrap();
}

pub(super) fn payload() -> MemoryCap {
    // Isolated metadata below is never installed into MEMORY_OBJECTS. Shared
    // authority is published only in an unstarted fixture root, then retired.
    MemoryCap {
        object: u64::MAX,
        rights: MemoryObjectRights::ALL,
        transfer_in_flight: false,
        delivery_pending: false,
    }
}

fn publication_and_detachment() {
    let handle = root();
    let counts = crate::capability::node_admission_used();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        let mut local = MemoryObjectRegistry::new();
        let mut first =
            Preparing::new(admit_capability(handle.id(), handle).unwrap(), handle).unwrap();
        let mut second =
            Preparing::new(admit_capability(handle.id(), handle).unwrap(), handle).unwrap();
        let first_id = first.identity();
        let second_id = second.identity();
        let (cap, namespace) = {
            let _registry = MEMORY_OBJECTS.lock();
            let _heap = PRIMARY_ALLOCATOR.lock();
            first.validate(&local).unwrap();
            first.publish().unwrap();
            first.install(&mut local, payload());
            second.validate(&local).unwrap();
            second.publish().unwrap();
            second.install(&mut local, payload());
            assert!(second.0.namespace.is_some(), "losing candidate freed under publication guard");
            let cap = retire(&mut local, handle.id(), first_id);
            assert!(local.caps[&handle.id()].caps.contains_key(&second_id));
            let namespace = retire_namespace(&mut local, handle.id()).unwrap();
            (cap, namespace)
        };
        assert_eq!(crate::capability::node_admission_used().0, counts.0 + 2);
        // Unused namespace candidates release before final authority references.
        first.finish();
        second.finish();
        cap.release();
        assert_eq!(crate::capability::node_admission_used().0, counts.0 + 1);
        namespace.release();
        assert_eq!(crate::capability::node_admission_used(), counts);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    memory::close_user_address_space_handle(handle).unwrap();
}

fn stale_namespace() {
    let old = root();
    let mut local = MemoryObjectRegistry::new();
    local.caps.insert(
        admission::prepare(false).unwrap(),
        old.id(),
        AddressSpaceCaps {
            handle: old,
            caps: Caps::new(),
        },
    );
    memory::close_user_address_space_handle(old).unwrap();
    let fresh = root();
    assert_eq!(fresh.id(), old.id());
    assert_ne!(fresh, old);
    let counts = crate::capability::node_admission_used();
    let preparation = Preparing::new(admit_capability(fresh.id(), fresh).unwrap(), fresh).unwrap();
    let namespace = {
        let _registry = MEMORY_OBJECTS.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(preparation.validate(&local), Err(MemoryObjectError::AddressSpaceMissing));
        retire_namespace(&mut local, old.id()).unwrap()
    };
    assert!(!crate::capability::contains(
        fresh.id(),
        preparation.identity(),
        crate::capability::ObjectKind::Memory
    ));
    preparation.finish();
    namespace.release();
    assert_eq!(crate::capability::node_admission_used(), counts);
    memory::close_user_address_space_handle(fresh).unwrap();
}
