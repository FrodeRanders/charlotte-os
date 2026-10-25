//! Serialized object-node preparation, relinking and complete backing retirement.
use super::*;
use crate::memory::{
    self,
    AddressSpaceHandle,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}
fn amount() -> memory::budget::Amount {
    memory::budget::Amount {
        pages: 1,
        objects: 1,
    }
}
fn available() {
    assert!(MEMORY_OBJECTS.try_lock().is_some());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some());
}

pub(super) fn run() {
    rejections();
    relinking();
    failed_physical_release();
    let mut isolated = MemoryObjectRegistry::new();
    isolated.next_object = u64::MAX;
    assert_eq!(isolated.allocate_object_id(), Err(MemoryObjectError::ResourceLimit));
    assert_eq!(isolated.next_object, u64::MAX);
    crate::logln!(
        "[memory object storage] 128 charged preparation and 64 copied-prefix rejections, 64 \
         heap-held object relinks/detaches, post-physical node/charge disposal, non-reused IDs \
         and terminal release rejection passed; one data page, complete object node and original \
         charge deliberately retained"
    );
}

fn rejections() {
    let handle = root();
    let target = root();
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..128 {
        assert!(matches!(
            ChargedFrames::allocate_with(handle, 1, || {
                available();
                assert_eq!(memory::budget::used(handle), amount());
                Err(MemoryObjectError::ResourceLimit)
            }),
            Err(MemoryObjectError::ResourceLimit)
        ));
        assert_eq!(memory::budget::used(handle), memory::budget::Amount::default());
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    }
    let cap = allocate(handle.id(), 1).unwrap();
    let before = crate::capability::node_admission_used();
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..64 {
        assert!(matches!(
            prepare_copy_with(handle.id(), cap, target.id(), |owner, pages| {
                ChargedFrames::allocate_with(owner, pages, || {
                    available();
                    assert_eq!(
                        memory::budget::used(handle),
                        memory::budget::Amount {
                            pages: 2,
                            objects: 2
                        }
                    );
                    Err(MemoryObjectError::ResourceLimit)
                })
            }),
            Err(MemoryObjectError::ResourceLimit)
        ));
        assert_eq!(crate::capability::node_admission_used(), before);
        assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
        assert_eq!(memory::budget::used(handle), amount());
        assert!(info(handle.id(), cap).is_ok());
        write_bytes(handle.id(), cap, &[0x5a]).unwrap();
    }
    close_cap(handle.id(), cap).unwrap();
    memory::close_user_address_space_handle(target).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn relinking() {
    let handle = root();
    let mut local = MemoryObjectRegistry::new();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        let staged = ChargedFrames::allocate(handle, 1).unwrap();
        let (storage, frames, charge) = staged.into_parts();
        let object = MemoryObject {
            owner: handle.id(),
            frames,
            charge,
            mappings: AdmittedMap::new(),
            lend_state: LendState::None,
            dma_pins: 0,
            exclusive_dma_pins: 0,
            copy_pins: 0,
            retirement_pins: 0,
            destroy_when_unpinned: false,
            close_interest: None,
            transfer_root: None,
            retired_caps: cap_storage::RetiredCapabilities::new(),
        };
        let id = local.allocate_object_id().unwrap();
        let node = {
            let _registry = MEMORY_OBJECTS.lock();
            let _heap = PRIMARY_ALLOCATOR.lock();
            local.objects.insert(storage, id, object);
            local.objects.take(&id).unwrap()
        };
        assert_eq!(memory::budget::used(handle), amount());
        let backing = ChargedFrames::retired(node);
        assert_eq!(backing.frame_count(), 1);
        backing.release().unwrap();
        assert_eq!(memory::budget::used(handle), memory::budget::Amount::default());
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    memory::close_user_address_space_handle(handle).unwrap();
}

fn failed_physical_release() {
    let handle = root();
    let cap = allocate(handle.id(), 1).unwrap();
    let mut retired = try_retire_cap(handle.id(), cap).unwrap();
    let mut backing = retired.backing.take().unwrap();
    drop(retired);
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let mut attempts = 0;
    assert!(backing.free_with(|batch| {
        available();
        assert_eq!(batch.len(), 1);
        attempts += 1;
        false // Deliberately retain this allocated frame; never re-adopt it.
    }));
    assert_eq!(attempts, 1);
    assert!(backing.0.failed && backing.0.retired.is_some());
    assert_eq!(memory::budget::used(handle), amount());
    assert!(backing.free_with(|_| panic!("terminal physical attempt was replayed")));
    {
        let _registry = MEMORY_OBJECTS.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(backing);
    }
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    assert_eq!(memory::budget::used(handle), amount());
    memory::close_user_address_space_handle(handle).unwrap();
    let fresh = root();
    assert_eq!(fresh.id(), handle.id());
    assert_ne!(fresh, handle);
    assert_eq!(memory::budget::used(fresh), memory::budget::Amount::default());
    assert_eq!(memory::budget::used(handle), amount());
    memory::close_user_address_space_handle(fresh).unwrap();
}
