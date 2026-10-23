//! Serialized real-node pressure; no budget resizing or synthetic refund.
use alloc::vec::Vec;

use super::*;

pub(super) fn run() {
    allocation_failure::<LimitsRecord>();
    allocation_failure::<UsageRecord>();
    allocation_failure::<AuthorityRecord>();
    pressure();
    crate::logln!(
        "[root metadata admission] 192 allocator rejections, partial-quota rollback, \
         ordinary/total real-node exhaustion, trusted reserve, immutable classification and \
         exact-root close/recovery passed"
    );
}

fn allocation_failure<T>() {
    let before = admission::snapshot();
    for _ in 0..64 {
        assert!(admission::prepare_with::<T>(false, |_| Err(AllocError)).is_err());
        assert_eq!(admission::snapshot(), before);
    }
}

fn rejected_registration() {
    let before = admission::snapshot();
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let roots = ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count();
    assert_eq!(
        register_user_address_space(AddressSpace::try_new_user().unwrap()),
        Err(AddressSpaceRegistrationError::DomainMetadataAllocationFailed)
    );
    assert_eq!(admission::snapshot(), before);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    assert_eq!(ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count(), roots);
}

fn pressure() {
    let before = admission::snapshot();
    let old = register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    let mut ordinary: Vec<Storage<LimitsRecord>> = Vec::with_capacity(12_288);
    while let Ok(node) = admission::prepare(false) {
        ordinary.push(node);
        assert!(ordinary.len() <= 12_288);
    }
    assert_eq!(admission::snapshot().1[1], 9_216);
    for _ in 0..64 {
        assert!(
            admission::prepare_with::<LimitsRecord>(false, |_| panic!(
                "denied root metadata reached allocator"
            ))
            .is_err()
        );
    }
    rejected_registration();
    // A root needs three independently owned nodes. One/two available slots
    // must return every partial preparation, without authority/ASID publication.
    for available in 1..=2 {
        for _ in 0..available {
            drop(ordinary.pop().unwrap());
        }
        rejected_registration();
        for _ in 0..available {
            ordinary.push(admission::prepare(false).unwrap());
        }
    }
    let full_ordinary = admission::snapshot();
    budget::mark_platform(old);
    assert_eq!(admission::snapshot(), full_ordinary);
    let trusted =
        register_user_address_space(AddressSpace::try_new_platform_user().unwrap()).unwrap();
    assert_eq!(admission::snapshot().1, full_ordinary.1);
    assert_eq!(admission::snapshot().0[1], full_ordinary.0[1] + 3);
    close_user_address_space_handle(trusted).unwrap();
    assert_eq!(admission::snapshot(), full_ordinary);
    let mut reserve: Vec<Storage<LimitsRecord>> = Vec::with_capacity(12_288);
    while let Ok(node) = admission::prepare(true) {
        reserve.push(node);
        assert!(reserve.len() <= 12_288);
    }
    let full = admission::snapshot();
    assert_eq!(full.0[1], 12_288);
    assert!(
        admission::prepare_with::<LimitsRecord>(true, |_| panic!(
            "total metadata denial reached allocator"
        ))
        .is_err()
    );
    // Late platform promotion did not reclassify old's three ordinary nodes.
    close_user_address_space_handle(old).unwrap();
    assert_eq!(admission::snapshot().0[1], full.0[1] - 3);
    assert_eq!(admission::snapshot().1[1], full.1[1] - 3);
    let recovered = register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    close_user_address_space_handle(recovered).unwrap();
    let ordinary_count = ordinary.len();
    let reserve_count = reserve.len();
    drop(reserve);
    drop(ordinary);
    assert_eq!(admission::snapshot(), before);
    crate::logln!(
        "[root metadata pressure] {} ordinary + {} reserved actual nodes; 9216/12288 ceilings, \
         one/two-node partial rollback, trusted root publication and close at total exhaustion \
         passed",
        ordinary_count,
        reserve_count
    );
}
