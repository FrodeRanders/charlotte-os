//! Actual mailbox candidate pressure and complete operation recovery.
use super::*;
use crate::memory::{
    self,
    AddressSpace,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn root(platform: bool) -> memory::AddressSpaceHandle {
    let space = if platform {
        AddressSpace::try_new_platform_user()
    } else {
        AddressSpace::try_new_user()
    };
    memory::register_user_address_space(space.unwrap()).unwrap()
}
fn denied(root: memory::AddressSpaceHandle, captured: MailboxIdentity) {
    assert_eq!(open(root.id(), Some(get_lp_id()), captured), Err(Error::ResourceLimit));
}
pub(super) fn run() {
    mailbox_node_budget::test_rejection();
    let before = mailbox_node_budget::snapshot();
    let records = mailbox_budget::node_used();
    let authority = crate::capability::node_admission_used();
    let old = root(false);
    let trusted = root(true);
    let captured = capture_mailbox_identity(old.id());
    let receiver = open(old.id(), None, captured).unwrap();
    open(old.id(), Some(get_lp_id()), captured).unwrap();
    let account = USER_MAILBOX_CAPS.read().get(&old.id()).unwrap().budget.clone();
    let bytes = mailbox_node_budget::Prepared::allocation_bytes() as u64;
    assert_eq!(
        mailbox_node_budget::snapshot(),
        ([before.0[0] + 2 * bytes, before.0[1] + 2], [before.1[0] + 2 * bytes, before.1[1] + 2])
    );
    let mut ordinary = alloc::vec::Vec::with_capacity(mailbox_node_budget::ORDINARY[1] as usize);
    while let Ok(node) = mailbox_node_budget::prepare(false) {
        ordinary.push(node);
        assert!(ordinary.len() <= mailbox_node_budget::ORDINARY[1] as usize);
    }
    let full_ordinary = mailbox_node_budget::snapshot();
    assert_eq!(full_ordinary.1[1], mailbox_node_budget::ORDINARY[1]);
    memory::budget::mark_platform(old);
    let promoted = capture_mailbox_identity(old.id());
    assert!(!captured.storage.platform() && promoted.storage.platform());
    // A fresh authorized capture can use reserve. The earlier operation must
    // keep its ordinary class, so exercise it separately with the heap held.
    let promoted_cap = open(old.id(), Some(get_lp_id()), promoted).unwrap();
    assert_eq!(
        mailbox_node_budget::snapshot(),
        ([full_ordinary.0[0] + bytes, full_ordinary.0[1] + 1], full_ordinary.1)
    );
    assert_eq!(mailbox_budget::node_used(), (records.0 + 3, records.1 + 2));
    close(old.id(), promoted_cap).unwrap();
    assert_eq!(mailbox_node_budget::snapshot(), full_ordinary);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            denied(old, captured);
        }
        assert_eq!(open(old.id(), None, capture_mailbox_identity(old.id())), Ok(receiver));
    }
    assert_eq!(mailbox_node_budget::snapshot(), full_ordinary);
    assert_eq!(mailbox_budget::node_used(), (records.0 + 2, records.1 + 2));
    assert_eq!(crate::capability::node_admission_used(), (authority.0 + 2, authority.1 + 2));
    open(trusted.id(), None, capture_mailbox_identity(trusted.id())).unwrap();
    assert_eq!(mailbox_budget::node_used(), (records.0 + 3, records.1 + 3));
    assert_eq!(
        mailbox_node_budget::snapshot(),
        ([full_ordinary.0[0] + bytes, full_ordinary.0[1] + 1], full_ordinary.1)
    );
    let mut reserve = alloc::vec::Vec::with_capacity(
        (mailbox_node_budget::LIMIT[1] - mailbox_node_budget::ORDINARY[1]) as usize,
    );
    while let Ok(node) = mailbox_node_budget::prepare(true) {
        reserve.push(node);
        assert!(
            reserve.len()
                <= (mailbox_node_budget::LIMIT[1] - mailbox_node_budget::ORDINARY[1]) as usize
        );
    }
    let full = mailbox_node_budget::snapshot();
    assert_eq!(full.0[1], mailbox_node_budget::LIMIT[1]);
    let trusted_capture = capture_mailbox_identity(trusted.id());
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            denied(trusted, trusted_capture);
        }
        assert_eq!(open(old.id(), None, capture_mailbox_identity(old.id())), Ok(receiver));
    }
    memory::close_user_address_space_handle(old).unwrap();
    assert_eq!(account.used(), 0);
    assert_eq!(
        mailbox_node_budget::snapshot(),
        ([full.0[0] - 2 * bytes, full.0[1] - 2], [full.1[0] - 2 * bytes, full.1[1] - 2])
    );
    let successor = root(false);
    assert_eq!(successor.id(), old.id());
    assert_ne!(successor, old);
    mailbox_node_budget::with_locked(|| {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(open(old.id(), Some(get_lp_id()), captured), Err(Error::Retired));
    });
    let fresh = capture_mailbox_identity(successor.id());
    open(successor.id(), None, fresh).unwrap();
    open(successor.id(), Some(get_lp_id()), fresh).unwrap();
    assert_eq!(mailbox_node_budget::snapshot(), full);
    memory::close_user_address_space_handle(successor).unwrap();
    memory::close_user_address_space_handle(trusted).unwrap();
    let counts = (ordinary.len(), reserve.len());
    drop((ordinary, reserve));
    assert_eq!(mailbox_node_budget::snapshot(), before);
    assert_eq!(mailbox_budget::node_used(), records);
    assert_eq!(crate::capability::node_admission_used(), authority);
    crate::logln!(
        "[mailbox node backing] bytes={} actual candidates ordinary={} trusted={}; ordinary/total \
         ceilings {}/{}; 64 allocation failures, byte denials, 128 heap-held admission denials, \
         receiver reuse, pre-promotion capture, fresh promotion reserve, trusted backing without \
         grants, final-root release and exact ASID-reuse recovery passed",
        bytes,
        counts.0,
        counts.1,
        mailbox_node_budget::ORDINARY[1],
        mailbox_node_budget::LIMIT[1]
    );
}
