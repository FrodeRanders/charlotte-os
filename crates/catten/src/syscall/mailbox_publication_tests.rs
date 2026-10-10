//! Serialized actual-publication rejection and guarded complete-owner probes.
use core::sync::atomic::{
    AtomicBool,
    AtomicUsize,
    Ordering,
};

use super::*;
use crate::{
    memory,
    service::loader,
};
static ACTIVE: AtomicBool = AtomicBool::new(false);
static IRQ: AtomicBool = AtomicBool::new(false);
static REJECT: AtomicUsize = AtomicUsize::new(0);
static PREPARED: AtomicUsize = AtomicUsize::new(0);
static DISPOSED: AtomicUsize = AtomicUsize::new(0);
pub(super) fn reject(stage: usize) -> bool {
    REJECT.compare_exchange(stage, 0, Ordering::AcqRel, Ordering::Relaxed).is_ok()
}
fn available(mut probe: impl FnMut() -> bool, label: &'static str) {
    let deadline = crate::self_test::results::Deadline::after_millis(1000);
    while !probe() {
        deadline.assert_pending(label);
        core::hint::spin_loop();
    }
}
pub(super) fn metadata_boundary(dispose: bool) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), IRQ.load(Ordering::Acquire));
    available(|| USER_MAILBOX_CAPS.try_write().is_some(), "mailbox authority registry");
    available(|| USER_MAILBOX.try_write().is_some(), "mailbox queues");
    available(
        || memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some(),
        "mailbox authority lifecycle",
    );
    available(|| memory::ADDRESS_SPACE_TABLE.try_lock().is_some(), "mailbox authority root table");
    available(|| memory::KERNEL_AS.try_lock().is_some(), "mailbox authority kernel table");
    available(
        || memory::PHYSICAL_FRAME_ALLOCATOR.try_lock().is_some(),
        "mailbox physical allocator",
    );
    crate::capability::record_tests::assert_local_available();
    if dispose {
        DISPOSED.fetch_add(1, Ordering::Relaxed);
    } else {
        PREPARED.fetch_add(1, Ordering::Relaxed);
    }
}
pub(in crate::syscall) fn run() {
    assert!(!ACTIVE.swap(true, Ordering::AcqRel));
    IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Release);
    PREPARED.store(0, Ordering::Relaxed);
    DISPOSED.store(0, Ordering::Relaxed);
    let root = loader::create_user_address_space_handle();
    let captured = capture_mailbox_identity(root.id());
    let first = open(root.id(), None, captured).unwrap();
    let budget = USER_MAILBOX_CAPS.read().get(&root.id()).unwrap().budget.clone();
    for stage in 1..=4 {
        if stage == 4 {
            crate::capability::record_tests::reject_next();
        } else {
            REJECT.store(stage, Ordering::Release);
        }
        let nodes = crate::capability::node_admission_used();
        assert_eq!(open(root.id(), Some(0), captured), Err(Error::AllocationFailed));
        assert_eq!(budget.used(), 1);
        assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 1);
        assert_eq!(crate::capability::node_admission_used(), nodes);
        // Existing receiver succeeds without consuming the next rejection.
        REJECT.store(1, Ordering::Release);
        assert_eq!(open(root.id(), None, captured), Ok(first));
        assert_eq!(REJECT.swap(0, Ordering::AcqRel), 1);
    }
    let cap = open(root.id(), Some(0), captured).unwrap();
    assert_eq!(cap, first + 1, "storage rejection consumed a serial");
    close(root.id(), cap).unwrap();
    assert_eq!(budget.used(), 1);
    assert!(close(root.id(), cap).is_err());
    cancellation(root, captured);
    heap_held(root, captured);
    staged_close(root, captured);
    assert_eq!(budget.used(), 0);
    abandoned();
    ACTIVE.store(false, Ordering::Release);
    crate::logln!(
        "[mailbox authority phases] {} preparation-entry and {} disposal-entry boundaries outside \
         local lifecycle/mailbox/queue/table/physical/heap/capability guards; entry IRQ state \
         preserved",
        PREPARED.load(Ordering::Acquire),
        DISPOSED.load(Ordering::Acquire)
    );
    crate::logln!(
        "[mailbox authority ownership] four storage rejection phases before charge/serial \
         mutation; receiver reuse; heap-held publication/detach; staged-close rejection; guarded \
         complete-owner abandonment retains two roots and two original mailbox/authority charges"
    );
}
fn cancellation(root: memory::AddressSpaceHandle, captured: MailboxIdentity) {
    let mut preparing = PreparingMailbox::new(root.id(), captured).unwrap();
    let local = USER_MAILBOX_CAPS.read().get(&root.id()).unwrap().budget.clone();
    {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        preparing.0.charge = Some(mailbox_budget::reserve(&local, false).unwrap());
        preparing.0.reservation =
            Some(preparing.0.authority.as_mut().unwrap().reserve_in_lifecycle(&lifecycle).unwrap());
    }
    assert_eq!(local.used(), 2);
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 2);
    preparing.finish().unwrap();
    assert_eq!(local.used(), 1);
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), 1);
}
fn heap_held(root: memory::AddressSpaceHandle, captured: MailboxIdentity) {
    let mut prepared = PreparingMailbox::new(root.id(), captured).unwrap();
    let cap = {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let mut registry = USER_MAILBOX_CAPS.write();
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        let cap = prepared.publish(Some(0), &lifecycle, &mut registry).unwrap();
        assert!(crate::capability::contains(root.id(), cap, ObjectKind::Mailbox));
        drop(heap);
        drop(registry);
        drop(lifecycle);
        cap
    };
    prepared.finish().unwrap();
    let mut retired = RetiredMailbox(ManuallyDrop::new(CloseResources {
        root: root_fn(root.id(), captured),
        payload: None,
        authority: None,
    }));
    {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let mut registry = USER_MAILBOX_CAPS.write();
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        retired.0.payload = registry.get_mut(&root.id()).unwrap().endpoints.take(&cap);
        retired.0.authority = crate::capability::detach(root.id(), cap, ObjectKind::Mailbox);
        assert!(retired.0.payload.is_some() && retired.0.authority.is_some());
        assert!(!crate::capability::contains(root.id(), cap, ObjectKind::Mailbox));
        drop(heap);
        drop(registry);
        drop(lifecycle);
    }
    retired.finish().unwrap();
}
fn root_fn(asid: AddressSpaceId, captured: MailboxIdentity) -> Option<AddressSpaceOperation> {
    super::root(asid, captured).unwrap()
}
fn staged_close(root: memory::AddressSpaceHandle, captured: MailboxIdentity) {
    let mut preparation = PreparingMailbox::new(root.id(), captured).unwrap();
    let closing = memory::retirement::ClosingAddressSpace::begin(root).unwrap();
    let result = {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let mut registry = USER_MAILBOX_CAPS.write();
        preparation.publish(Some(0), &lifecycle, &mut registry)
    };
    assert_eq!(result, Err(Error::Retired));
    preparation.finish().unwrap();
    match closing.poll().unwrap() {
        memory::retirement::CloseProgress::Complete => (),
        memory::retirement::CloseProgress::Pending(_) => {
            panic!("mailbox rejection leaked root lease")
        }
    }
}
fn abandoned() {
    let preparing_root = loader::create_user_address_space_handle();
    let captured = capture_mailbox_identity(preparing_root.id());
    let cap = open(preparing_root.id(), Some(0), captured).unwrap();
    close(preparing_root.id(), cap).unwrap();
    let mut preparing = PreparingMailbox::new(preparing_root.id(), captured).unwrap();
    let local = USER_MAILBOX_CAPS.read().get(&preparing_root.id()).unwrap().budget.clone();
    {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        preparing.0.charge = Some(mailbox_budget::reserve(&local, false).unwrap());
        preparing.0.reservation =
            Some(preparing.0.authority.as_mut().unwrap().reserve_in_lifecycle(&lifecycle).unwrap());
    }
    let retired_root = loader::create_user_address_space_handle();
    let cap =
        open(retired_root.id(), Some(0), capture_mailbox_identity(retired_root.id())).unwrap();
    let retired_local = USER_MAILBOX_CAPS.read().get(&retired_root.id()).unwrap().budget.clone();
    let retired = RetiredMailbox::prepare(retired_root.id(), cap).unwrap();
    // Promotion cannot retroactively reclassify the original ordinary charges.
    crate::capability::mark_platform(preparing_root);
    crate::capability::mark_platform(retired_root);
    {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let registry = USER_MAILBOX_CAPS.write();
        let queues = USER_MAILBOX.write();
        let table = memory::ADDRESS_SPACE_TABLE.lock();
        let kernel = memory::KERNEL_AS.lock();
        let physical = memory::PHYSICAL_FRAME_ALLOCATOR.lock();
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        drop(preparing);
        drop(retired);
        drop(heap);
        drop(physical);
        drop(kernel);
        drop(table);
        drop(queues);
        drop(registry);
        drop(lifecycle);
    }
    assert_eq!(local.used(), 1);
    assert_eq!(retired_local.used(), 1);
    assert_eq!(crate::capability::admission_tests::test_namespace_used(preparing_root.id()), 1);
    assert_eq!(crate::capability::admission_tests::test_namespace_used(retired_root.id()), 1);
    for root in [preparing_root, retired_root] {
        let closing = memory::retirement::ClosingAddressSpace::begin(root).unwrap();
        assert!(
            matches!(closing.poll().unwrap(), memory::retirement::CloseProgress::Pending(_)),
            "abandonment released its exact root"
        );
    }
}
