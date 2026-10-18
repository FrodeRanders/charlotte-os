//! Serialized probes of the final metadata release boundary. Wider caller contexts remain
//! unqualified.
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use super::*;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static IRQ: AtomicBool = AtomicBool::new(false);
static MAILBOX: AtomicUsize = AtomicUsize::new(0);
static AUTHORITY: AtomicUsize = AtomicUsize::new(0);
static POLICY: AtomicUsize = AtomicUsize::new(0);

fn available(mut probe: impl FnMut() -> bool, label: &'static str) {
    let deadline = crate::self_test::results::Deadline::after_millis(1000);
    while !probe() {
        deadline.assert_pending(label);
        core::hint::spin_loop();
    }
}
fn check_context() {
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), IRQ.load(Ordering::Acquire));
    available(
        || super::super::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(),
        "root metadata lifecycle",
    );
    available(|| ADDRESS_SPACE_TABLE.try_lock().is_some(), "root metadata root table");
    available(|| super::super::KERNEL_AS.try_lock().is_some(), "root metadata kernel table");
    available(
        || super::super::PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some(),
        "root metadata physical allocator",
    );
    crate::syscall::mailbox_retirement::tests::assert_guards_available();
    crate::capability::record_tests::assert_local_available();
    available(|| super::super::DOMAIN_LIMITS.try_lock().is_some(), "root limits metadata");
    available(|| super::super::usage::DOMAIN_USAGE.try_lock().is_some(), "root usage metadata");
    available(
        || super::super::DOMAIN_AUTHORITIES.try_lock().is_some(),
        "root signed authority metadata",
    );
    available(super::super::usage::principal_history_available, "principal history metadata");
}
pub(crate) fn boundary(authority: bool) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    check_context();
    if authority {
        AUTHORITY.fetch_add(1, Ordering::Relaxed);
    } else {
        MAILBOX.fetch_add(1, Ordering::Relaxed);
    }
}
pub(crate) fn policy_boundary() {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    check_context();
    POLICY.fetch_add(1, Ordering::Relaxed);
}
pub(super) fn begin() {
    assert!(!ACTIVE.swap(true, Ordering::AcqRel));
    IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Release);
    MAILBOX.store(0, Ordering::Relaxed);
    AUTHORITY.store(0, Ordering::Relaxed);
    POLICY.store(0, Ordering::Relaxed);
}
pub(super) fn finish() {
    ACTIVE.store(false, Ordering::Release);
    assert_eq!(MAILBOX.load(Ordering::Acquire), AUTHORITY.load(Ordering::Acquire));
    assert_eq!(MAILBOX.load(Ordering::Acquire), POLICY.load(Ordering::Acquire));
    assert!(MAILBOX.load(Ordering::Acquire) > 0);
    crate::logln!(
        "[root metadata phases] {} mailbox, {} authority and {} policy/usage/signed-authority \
         release entries outside local \
         lifecycle/mailbox/queue/table/physical/heap/capability/policy/usage/signed-authority \
         guards; entry IRQ state preserved",
        MAILBOX.load(Ordering::Acquire),
        AUTHORITY.load(Ordering::Acquire),
        POLICY.load(Ordering::Acquire)
    );
}
