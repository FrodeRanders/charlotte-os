//! Mailbox record admission through the actual syscall dispatch path, plus
//! captured-generation publication and retained-charge retirement fixtures.

use alloc::vec::Vec;

use mailbox_budget::{
    DOMAIN_LIMIT,
    Error,
};

use super::*;

const CLIENT: AddressSpaceId = 0x5e30;

fn dispatch(asid: AddressSpaceId, number: u16, arg: u64) -> TrapFrame {
    let mut frame = TrapFrame {
        regs: [0; 19],
        elr_el1: 0xdeadbeef0000,
        spsr_el1: 0,
        sp_el0: 0,
        lp_id: get_lp_id(),
        asid,
    };
    frame.regs[1] = arg;
    syscall_dispatch(&mut frame, number);
    frame
}

fn sender(asid: AddressSpaceId) -> u64 {
    dispatch(asid, call_no::MAILBOX_OPEN_SEND, 0).regs[0]
}

fn receiver(asid: AddressSpaceId) -> u64 {
    dispatch(asid, call_no::MAILBOX_OPEN_RECV, 0).regs[0]
}

fn close(asid: AddressSpaceId, cap: u64) {
    assert_eq!(dispatch(asid, call_no::MAILBOX_CLOSE, cap).regs[0], 0);
}

fn account(asid: AddressSpaceId) -> alloc::sync::Arc<mailbox_budget::DomainBudget> {
    USER_MAILBOX_CAPS.read().get(&asid).unwrap().budget.clone()
}

pub(crate) fn test_admission() {
    crate::capability::admission_tests::test_admission();
    crate::capability::test_identity_exhaustion();
    mailbox_budget::test_node_admission();
    let recv = receiver(CLIENT);
    assert_ne!(recv, 0);
    assert_eq!(
        dispatch(CLIENT, call_no::MAILBOX_OPEN_SEND, 1u64 << 32).regs[0],
        0,
        "invalid high LP bits must fail even with spare capacity"
    );
    let budget = account(CLIENT);
    let mut senders = Vec::new();
    for _ in 1..DOMAIN_LIMIT {
        let cap = sender(CLIENT);
        assert_ne!(cap, 0);
        senders.push(cap);
    }
    assert_eq!(budget.used(), DOMAIN_LIMIT);
    assert_eq!(dispatch(CLIENT, call_no::MAILBOX_OPEN_SEND, 1u64 << 32).regs[0], 0);
    assert_eq!(sender(CLIENT), 0, "full mailbox namespace must report open failure");
    assert_eq!(receiver(CLIENT), recv, "receiver reuse needs no spare entry");
    assert_eq!(budget.used(), DOMAIN_LIMIT);
    let last = senders.pop().unwrap();
    close(CLIENT, last);
    let replacement = sender(CLIENT);
    assert_eq!(replacement, last + 1, "rejected open must not consume identity");
    close(CLIENT, replacement);
    for cap in senders {
        close(CLIENT, cap);
    }
    assert_eq!(budget.used(), 1);
    close(CLIENT, recv);
    assert_eq!(budget.used(), 0);
    for _ in 0..1024 {
        close(CLIENT, sender(CLIENT));
    }
    assert_eq!(budget.used(), 0);

    let survivor = sender(CLIENT);
    crate::capability::exhaust_identity_for_test(CLIENT);
    assert_eq!(sender(CLIENT), 0, "serial exhaustion must not panic or publish");
    assert_eq!(budget.used(), 1, "failed mint must refund staged admission");
    assert!(crate::capability::contains(CLIENT, survivor, crate::capability::ObjectKind::Mailbox));
    close(CLIENT, survivor);
    close_mailbox_address_space_for_test(CLIENT);
    assert_eq!(budget.used(), 0);
    assert!(matches!(mailbox_budget::reserve(&budget, false), Err(Error::Retired)));
    crate::capability::close_address_space(CLIENT);

    // Retain a staged reservation over namespace replacement. Its Drop must
    // credit only the old account, even when numeric capability ids repeat.
    let old_cap = sender(CLIENT);
    let old = account(CLIENT);
    let staged = mailbox_budget::reserve(&old, false).unwrap();
    close_mailbox_address_space_for_test(CLIENT);
    assert_eq!(old.used(), 1);
    crate::capability::close_address_space(CLIENT);
    let new_cap = sender(CLIENT);
    assert_eq!(old_cap, new_cap);
    let new = account(CLIENT);
    assert!(matches!(mailbox_budget::reserve(&old, true), Err(Error::Retired)));
    drop(staged);
    assert_eq!(old.used(), 0);
    assert_eq!(new.used(), 1);
    close(CLIENT, new_cap);
    close_mailbox_address_space_for_test(CLIENT);
    crate::capability::close_address_space(CLIENT);
    test_generation_fence();
    test_shared_admission();
    mailbox_publication::tests::run();
    logln!("[mailbox] record quota, churn, rollback, retirement and generation fencing passed");
}

fn test_shared_admission() {
    const OWNER: AddressSpaceId = 0x5e41;
    crate::capability::admission_tests::test_fill_namespace(OWNER);
    assert_eq!(sender(OWNER), 0, "aggregate count must reject below the mailbox family limit");
    let local = account(OWNER);
    assert_eq!(local.used(), 0, "aggregate rejection must refund mailbox admission");
    assert!(crate::capability::remove(OWNER, 1, crate::capability::ObjectKind::Memory));
    let cap = sender(OWNER);
    assert_ne!(cap, 0);
    assert_eq!(local.used(), 1);
    close(OWNER, cap);
    assert_eq!(local.used(), 0);
    close_mailbox_address_space_for_test(OWNER);
    crate::capability::close_address_space(OWNER);
}

fn test_generation_fence() {
    use crate::{
        memory,
        service::loader,
    };

    let old = loader::create_user_address_space_handle();
    let captured = capture_mailbox_identity(old.id());
    let cap = open_mailbox_endpoint(old.id(), Some(0), captured).unwrap();
    assert_ne!(cap, 0);
    let old_budget = account(old.id());
    memory::budget::retire(old);
    assert_eq!(open_mailbox_endpoint(old.id(), Some(0), captured), Err(Error::Retired));
    assert_eq!(old_budget.used(), 1);
    close_mailbox_address_space_for_test(old.id());
    assert_eq!(open_mailbox_endpoint(old.id(), None, captured), Err(Error::Retired));
    assert!(!USER_MAILBOX_CAPS.read().contains_key(&old.id()));
    memory::close_user_address_space_handle(old).unwrap();

    let new = loader::create_user_address_space_handle();
    assert_eq!(old.id(), new.id());
    assert_ne!(old.generation(), new.generation());
    let new_identity = capture_mailbox_identity(new.id());
    let new_cap = open_mailbox_endpoint(new.id(), Some(0), new_identity).unwrap();
    let new_budget = account(new.id());
    assert_eq!(open_mailbox_endpoint(old.id(), Some(0), captured), Err(Error::Retired));
    assert_eq!(new_budget.used(), 1);
    assert!(crate::capability::contains(new.id(), new_cap, crate::capability::ObjectKind::Mailbox));
    memory::close_user_address_space_handle(new).unwrap();
    assert_eq!(new_budget.used(), 0);
}
