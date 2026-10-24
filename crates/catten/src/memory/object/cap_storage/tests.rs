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
    publication_and_detachment();
    stale_namespace();
    crate::logln!(
        "[memory capability storage] 192 charged preparation-prefix rejections, 64 heap-held \
         two-capability/namespace publications and paired detachments, post-guard count recovery \
         and stale captured-generation rejection passed"
    );
}

fn rejected_prefixes() {
    let handle = root();
    let counts = crate::capability::node_admission_used();
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
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
            assert!(MEMORY_OBJECTS.lock().caps.get(&handle.id()).is_none());
        }
    }
    memory::close_user_address_space_handle(handle).unwrap();
}

fn payload() -> MemoryCap {
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
            assert!(second.namespace.is_some(), "losing candidate freed under publication guard");
            let cap = retire(&mut local, handle.id(), first_id);
            assert!(local.caps[&handle.id()].caps.contains_key(&second_id));
            let namespace = retire_namespace(&mut local, handle.id()).unwrap();
            (cap, namespace)
        };
        assert_eq!(crate::capability::node_admission_used().0, counts.0 + 2);
        // Unused namespace candidates release before final authority references.
        drop(first);
        drop(second);
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
        NamespaceStorage::try_new().unwrap(),
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
    drop(preparation);
    namespace.release();
    assert_eq!(crate::capability::node_admission_used(), counts);
    memory::close_user_address_space_handle(fresh).unwrap();
}
