//! Complete payload retention after extraction; these private table owners
//! never enter hardware. Real maintenance/competing-operation evidence is in
//! the QEMU NVMe recovery fixture.
use core::sync::atomic::{
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::device::detached_domain::DetachedDomain;

static DROPS: AtomicUsize = AtomicUsize::new(0);
struct Metadata {
    _allocation: u64,
}
impl Drop for Metadata {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::Relaxed);
    }
}
struct Payload {
    tables: Tables,
    metadata: Vec<Metadata>,
}
fn payload(pages: usize) -> Payload {
    let mut tables = Tables::new(Scope::Domain);
    tables.allocate(pages, PAGE).unwrap();
    tables.publish(); // Private fixture; no hardware publication or fake drain.
    let metadata = alloc::vec![Metadata {
        _allocation: 0x444d_415f_4f57_4e52
    }];
    Payload {
        tables,
        metadata,
    }
}

pub(super) fn run() {
    let baseline = used();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let drops = DROPS.load(Ordering::Relaxed);
    // Preserve the containing allocation, not just the table's own fallback.
    let mut slot = Some(payload(1));
    let metadata = slot.as_ref().unwrap().metadata.as_ptr();
    let owner = DetachedDomain::new(slot.take().unwrap());
    assert!(slot.is_none());
    preparation_tests::drop_under_guards(|| drop(owner));
    assert!(slot.is_none());
    assert_eq!(DROPS.load(Ordering::Relaxed), drops);
    assert_eq!(used(), (baseline.0 + 1, baseline.1 + 1));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 1);
    // The address is only diagnostic identity; never re-adopt abandoned data.
    assert!(!metadata.is_null());

    let mut slot = Some(payload(2));
    let metadata = slot.as_ref().unwrap().metadata.as_ptr();
    let mut owner = DetachedDomain::new(slot.take().unwrap());
    let mut calls = 0;
    assert_eq!(
        owner.value_mut().tables.release_with(|frame| {
            calls += 1;
            if calls == 2 {
                Err(PhysicalError::InvalidPAddr)
            } else {
                PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame)
            }
        }),
        Err(Error::MapFailed)
    );
    assert_eq!(calls, 2);
    assert!(slot.is_none());
    // Restore while allocators/registries are held: no reinsertion allocation.
    preparation_tests::drop_under_guards(|| slot = Some(owner.into_inner()));
    assert_eq!(slot.as_ref().unwrap().metadata.as_ptr(), metadata);
    let mut owner = DetachedDomain::new(slot.take().unwrap());
    assert_eq!(
        owner.value_mut().tables.release_with(|_| panic!("terminal physical retry")),
        Err(Error::MapFailed)
    );
    preparation_tests::drop_under_guards(|| drop(owner));
    assert_eq!(DROPS.load(Ordering::Relaxed), drops);
    assert_eq!(used(), (baseline.0 + 3, baseline.1 + 3));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 2);

    let mut slot = Some(payload(1));
    let mut owner = DetachedDomain::new(slot.take().unwrap());
    owner.value_mut().tables.release().unwrap();
    drop(owner.into_inner());
    assert_eq!(DROPS.load(Ordering::Relaxed), drops + 1);
    assert_eq!(used(), (baseline.0 + 3, baseline.1 + 3));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free - 2);
    crate::logln!(
        "[IOMMU detached ownership] complete metadata retained under guards; exact-slot partial \
         rejection is terminal; 3 original domain charges/2 frames retained"
    );
}
