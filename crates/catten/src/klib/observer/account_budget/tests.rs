//! Serialized pressure uses actual account allocations and root transactions.
use alloc::{
    sync::Arc,
    vec::Vec,
};

use super::*;
use crate::{
    klib::observer::{
        WaitSponsor,
        registration::RegistrationError,
        waiter_budget,
    },
    memory::{
        self,
        AddressSpace,
        AddressSpaceRegistrationError,
        PHYSICAL_FRAME_ALLOCATOR,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};

pub(super) fn run() {
    pressure();
    retirement();
    crate::logln!(
        "[waiter account admission] real-account ordinary/total exhaustion, pre-allocation \
         denial, trusted root reserve, immutable classification, root rollback and final-weak \
         release across exact ASID reuse passed"
    );
}

fn rejected_root() {
    let before = snapshot();
    let frames = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let roots = memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count();
    assert_eq!(
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()),
        Err(AddressSpaceRegistrationError::MemorySponsorAllocationFailed)
    );
    assert_eq!(snapshot(), before);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
    assert_eq!(
        memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|root| root.is_some()).count(),
        roots
    );
}

fn pressure() {
    let before = snapshot();
    let old = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    let mut ordinary = Vec::with_capacity(NODE_LIMIT as usize);
    while let Ok(account) = WaitSponsor::try_new(false) {
        ordinary.push(account);
        assert!(ordinary.len() <= NODE_LIMIT as usize);
    }
    assert_eq!(snapshot().1, ORDINARY_LIMIT);
    // Real admission rejection must precede either heap allocation.
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            assert!(WaitSponsor::try_new(false).is_err());
        }
    }
    rejected_root();
    let full_ordinary = snapshot();
    memory::budget::mark_platform(old);
    assert_eq!(snapshot(), full_ordinary);
    let trusted =
        memory::register_user_address_space(AddressSpace::try_new_platform_user().unwrap())
            .unwrap();
    assert_eq!(snapshot(), (full_ordinary.0 + 1, full_ordinary.1));
    // Trusted backing does not itself grant platform waiter-entry admission.
    assert!(!memory::budget::waiter_sponsor(trusted.id()).list_platform().unwrap());
    memory::close_user_address_space_handle(trusted).unwrap();
    assert_eq!(snapshot(), full_ordinary);
    let mut reserved = Vec::with_capacity(NODE_LIMIT as usize);
    while let Ok(account) = WaitSponsor::try_new(true) {
        reserved.push(account);
        assert!(reserved.len() <= NODE_LIMIT as usize);
    }
    let full = snapshot();
    assert_eq!(full.0, NODE_LIMIT);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(WaitSponsor::try_new(true).is_err());
    }
    rejected_root();
    memory::close_user_address_space_handle(old).unwrap();
    assert_eq!(snapshot(), (full.0 - 1, full.1 - 1));
    let recovered =
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    memory::close_user_address_space_handle(recovered).unwrap();
    let ordinary_count = ordinary.len();
    let reserved_count = reserved.len();
    drop(reserved);
    drop(ordinary);
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[waiter account pressure] {} ordinary + {} reserved actual accounts; {}/{} ceilings and \
         exact recovery passed",
        ordinary_count,
        reserved_count,
        ORDINARY_LIMIT,
        NODE_LIMIT
    );
}

fn retirement() {
    let before = snapshot();
    let entries = waiter_budget::node_used();
    let old = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    let sponsor = memory::budget::waiter_sponsor(old.id());
    let weak = Arc::downgrade(&sponsor.0);
    let entry = waiter_budget::reserve(&sponsor.0).unwrap();
    memory::close_user_address_space_handle(old).unwrap();
    assert_eq!(snapshot(), (before.0 + 1, before.1 + 1));
    assert!(matches!(waiter_budget::reserve(&sponsor.0), Err(RegistrationError::Closed)));
    let next = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    assert_eq!(next.id(), old.id());
    assert_ne!(next.generation(), old.generation());
    let successor = memory::budget::waiter_sponsor(next.id());
    assert_eq!(successor.used(), 0);
    drop(sponsor);
    drop(entry);
    assert_eq!(waiter_budget::node_used(), entries);
    assert!(weak.upgrade().is_none());
    assert_eq!(snapshot(), (before.0 + 2, before.1 + 2));
    drop(weak);
    assert_eq!(snapshot(), (before.0 + 1, before.1 + 1));
    assert_eq!(successor.used(), 0);
    drop(successor);
    memory::close_user_address_space_handle(next).unwrap();
    assert_eq!(snapshot(), before);
}
