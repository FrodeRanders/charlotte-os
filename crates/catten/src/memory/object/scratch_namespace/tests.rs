//! Serialized namespace/extent preparation, publication and lifetime controls.
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
    assert!(SCRATCH_WINDOWS.try_lock().is_some());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
    assert!(PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some());
}

pub(crate) fn run() {
    rejection();
    publication();
    competing_namespace();
    stale_generation();
    retention();
    crate::logln!(
        "[scratch storage] 192 unlocked prefix rejections, 64 heap-held namespace/extent \
         publications/detachments, competing namespace, full/invalid/stale rejection and guarded \
         partial/retired retention passed; six isolated metadata nodes retained, no data/root"
    );
}

fn rejection() {
    let handle = root();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let counts = crate::capability::node_admission_used();
    for reject in 0..3 {
        for _ in 0..64 {
            let result = with_address_space_operation(handle.id(), || {
                reserve_with(handle.id(), 1, |handle| {
                    Preparing::with(handle, |stage| {
                        available();
                        if reject == stage {
                            Err(MemoryObjectError::ResourceLimit)
                        } else {
                            Ok(())
                        }
                    })
                })
            });
            assert_eq!(result, Err(MemoryObjectError::ResourceLimit));
            assert!(SCRATCH_WINDOWS.lock().get(&handle.id()).is_none());
            assert_eq!(crate::capability::node_admission_used(), counts);
            assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
        }
    }
    for pages in [0, usize::MAX, SCRATCH_WINDOW_PAGES + 1] {
        assert_eq!(
            reserve_with(handle.id(), pages, |_| panic!("invalid scratch entered allocation")),
            Err(MemoryObjectError::OutOfScratch)
        );
    }
    let base = reserve_scratch(handle.id(), SCRATCH_WINDOW_PAGES).unwrap();
    assert_eq!(
        reserve_with(handle.id(), 1, |_| panic!("full scratch entered allocation")),
        Err(MemoryObjectError::OutOfScratch)
    );
    release_scratch(handle.id(), base, SCRATCH_WINDOW_PAGES).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}
fn publication() {
    let handle = root();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        let mut local = Namespaces::new();
        let mut first = Preparing::new(handle).unwrap();
        let mut second = Preparing::new(handle).unwrap();
        let (extent, namespace) = {
            let _windows = SCRATCH_WINDOWS.lock();
            let _heap = PRIMARY_ALLOCATOR.lock();
            assert_eq!(first.publish(&mut local, PAGE_SIZE * 2), Ok(0));
            assert_eq!(second.publish(&mut local, PAGE_SIZE), Ok(PAGE_SIZE * 2));
            assert!(second.0.namespace.is_some(), "unused candidate disposed under guard");
            let extent = take_extent(&mut local, handle, 0, PAGE_SIZE * 2).unwrap();
            assert_eq!(local.get(&handle.id()).unwrap().window.first_fit(PAGE_SIZE * 2), Ok(0));
            let namespace = detach_from(&mut local, handle).unwrap();
            (extent, namespace)
        };
        first.finish();
        second.finish();
        extent.release();
        // This private metadata control installed no page tables or externally
        // owned range. Its complete isolated namespace has no physical work.
        namespace.release();
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    memory::close_user_address_space_handle(handle).unwrap();
}
fn competing_namespace() {
    let handle = root();
    let mut winner = None;
    let base = with_address_space_operation(handle.id(), || {
        reserve_with(handle.id(), 1, |handle| {
            let prepared = Preparing::new(handle)?;
            winner = Some(reserve_scratch(handle.id(), 2)?);
            Ok(prepared)
        })
    })
    .unwrap();
    let winner = winner.unwrap();
    assert_eq!(base, winner + PAGE_SIZE * 2);
    release_scratch(handle.id(), winner, 2).unwrap();
    release_scratch(handle.id(), base, 1).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}
fn stale_generation() {
    let old = root();
    let mut prepared = Preparing::new(old).unwrap();
    memory::close_user_address_space_handle(old).unwrap();
    let fresh = root();
    assert_eq!(fresh.id(), old.id());
    assert_ne!(fresh, old);
    let base = reserve_scratch(fresh.id(), 1).unwrap();
    {
        let mut windows = SCRATCH_WINDOWS.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(
            prepared.publish(&mut windows, PAGE_SIZE),
            Err(MemoryObjectError::AddressSpaceMissing)
        );
        assert!(matches!(
            take_extent(&mut windows, old, 0, PAGE_SIZE),
            Err(MemoryObjectError::AddressSpaceMissing)
        ));
        assert!(detach_from(&mut windows, old).is_none());
        assert_eq!(windows.get(&fresh.id()).unwrap().window.extent_count(), 1);
    }
    prepared.finish();
    release_scratch(fresh.id(), base, 1).unwrap();
    memory::close_user_address_space_handle(fresh).unwrap();
}
fn retention() {
    let handle = root();
    for stage in 0..3 {
        let mut owner = Preparing(ManuallyDrop::new(Resources {
            handle,
            extent: None,
            namespace: None,
        }));
        if stage >= 1 {
            owner.0.extent = Some(scratch::PreparedExtent::try_new().unwrap());
        }
        if stage >= 2 {
            owner.0.namespace = Some(Storage::try_new().unwrap());
        }
        let _windows = SCRATCH_WINDOWS.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(owner); // Three empty nodes retained across these partial phases.
    }
    let mut local = Namespaces::new();
    let mut first = Preparing::new(handle).unwrap();
    let mut second = Preparing::new(handle).unwrap();
    first.publish(&mut local, PAGE_SIZE).unwrap();
    second.publish(&mut local, PAGE_SIZE).unwrap();
    first.finish();
    second.finish();
    let namespace = detach_from(&mut local, handle).unwrap();
    {
        let _windows = SCRATCH_WINDOWS.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(namespace); // Namespace + two extents; no physical mapping or lease.
    }
    memory::close_user_address_space_handle(handle).unwrap();
}
