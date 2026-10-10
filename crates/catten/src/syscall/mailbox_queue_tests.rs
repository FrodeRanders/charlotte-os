//! Actual queue admission, bounded delivery and complete-owner retention.
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
static BACKING_REJECT: AtomicBool = AtomicBool::new(false);
static REJECT: AtomicBool = AtomicBool::new(false);
static PREPARED: AtomicUsize = AtomicUsize::new(0);
static FINISHED: AtomicUsize = AtomicUsize::new(0);
pub(super) fn reject_backing() -> bool {
    BACKING_REJECT.swap(false, Ordering::AcqRel)
}
pub(super) fn reject() -> bool {
    REJECT.swap(false, Ordering::AcqRel)
}
pub(super) fn boundary(dispose: bool) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), IRQ.load(Ordering::Acquire));
    mailbox_retirement::tests::assert_guards_available();
    let deadline = crate::self_test::results::Deadline::after_millis(1000);
    while memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_none()
        || memory::ADDRESS_SPACE_TABLE.try_lock().is_none()
        || memory::KERNEL_AS.try_lock().is_none()
        || memory::PHYSICAL_FRAME_ALLOCATOR.try_lock().is_none()
    {
        deadline.assert_pending("queue operation metadata boundary");
        core::hint::spin_loop();
    }
    crate::capability::record_tests::assert_local_available();
    if dispose {
        FINISHED.fetch_add(1, Ordering::Relaxed);
    } else {
        PREPARED.fetch_add(1, Ordering::Relaxed);
    }
}
pub(in crate::syscall) fn run() {
    assert!(!ACTIVE.swap(true, Ordering::AcqRel));
    IRQ.store(crate::cpu::isa::lp::ops::get_int_state(), Ordering::Release);
    let root = loader::create_user_address_space_handle();
    let captured = capture_mailbox_identity(root.id());
    let bytes = mailbox_words::Words::backing_bytes().unwrap() as u64;
    let baseline = mailbox_budget::queue_node_used();
    assert_eq!(mailbox_words::Words::backing_bytes_for(usize::MAX), Err(Error::ResourceLimit));
    assert!(matches!(mailbox_words::Words::prepare(usize::MAX), Err(Error::AllocationFailed)));
    REJECT.store(true, Ordering::Release);
    assert_eq!(send(root.id(), get_lp_count(), 7, captured), Err(7));
    assert!(REJECT.load(Ordering::Acquire), "invalid target allocated storage");
    assert_eq!(send(root.id(), get_lp_id(), 7, captured), Err(7));
    assert!(!USER_MAILBOX.read().contains_key(&root.id()));
    BACKING_REJECT.store(true, Ordering::Release);
    assert_eq!(send(root.id(), get_lp_id(), 8, captured), Err(8));
    let budget = USER_MAILBOX_CAPS.read().get(&root.id()).unwrap().budget.clone();
    assert_eq!(budget.queue_used(), [0, 0]);
    assert!(matches!(
        mailbox_budget::reserve_queue(&budget, false, 1024 * 1024 + 1),
        Err(Error::ResourceLimit)
    ));
    assert_eq!(mailbox_budget::queue_node_used(), baseline);
    assert!(!USER_MAILBOX.read().contains_key(&root.id()));
    // Both exact-root operations prepare before guards. A competing winner
    // neither replaces the first queue nor destroys unused storage under heap.
    let mut first = Operation::new(root.id(), captured).unwrap();
    let mut second = Operation::new(root.id(), captured).unwrap();
    first.prepare().unwrap();
    second.prepare().unwrap();
    assert_eq!(budget.queue_used(), [2, 2 * bytes]);
    let mut third = Operation::new(root.id(), captured).unwrap();
    assert_eq!(third.prepare(), Err(Error::ResourceLimit));
    third.finish().unwrap();
    assert_eq!(budget.queue_used(), [2, 2 * bytes]);
    {
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        assert_eq!(first.publish_send(get_lp_id(), 11), Ok(()));
        assert_eq!(second.publish_send(get_lp_id(), 12), Ok(()));
        drop(heap);
    }
    first.finish().unwrap();
    second.finish().unwrap();
    assert_eq!(budget.queue_used(), [1, bytes]);
    REJECT.store(true, Ordering::Release);
    assert_eq!(send(root.id(), get_lp_id(), 13, captured), Ok(()));
    assert!(REJECT.swap(false, Ordering::AcqRel), "established send prepared storage");
    for word in [11, 12, 13] {
        assert_eq!(receive(root.id(), captured), Some(word));
    }
    let operation = Operation::new(root.id(), captured).unwrap();
    for lap in 0..2 {
        for i in 0..256 {
            assert_eq!(operation.existing(get_lp_id(), lap * 256 + i), Ok(Some(Ok(()))));
        }
        assert_eq!(operation.existing(get_lp_id(), 999), Ok(Some(Err(999))));
        for i in 0..256 {
            assert_eq!(operation.receive(), Some(lap * 256 + i));
        }
        assert_eq!(operation.receive(), None);
    }
    operation.finish().unwrap();
    // Actual final metadata detachment does not destroy its admitted node while
    // the heap/lifecycle are held. Release happens only after both leave.
    memory::budget::retire(root);
    assert_eq!(send(root.id(), get_lp_id(), 99, captured), Err(99));
    assert_eq!(receive(root.id(), captured), None);
    let detached = {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        let detached = mailbox_retirement::detach(root, &lifecycle).unwrap();
        assert!(!USER_MAILBOX.read().contains_key(&root.id()));
        drop(heap);
        drop(lifecycle);
        detached
    };
    assert_eq!(budget.queue_used(), [1, bytes]);
    assert!(matches!(
        mailbox_budget::reserve_queue(&budget, true, bytes as usize),
        Err(Error::Retired)
    ));
    detached.release();
    assert_eq!(budget.queue_used(), [0, 0]);
    assert_eq!(mailbox_budget::queue_node_used(), baseline);
    memory::close_user_address_space_handle(root).unwrap();
    let successor = loader::create_user_address_space_handle();
    assert_eq!(successor.id(), root.id());
    assert_ne!(successor.generation(), root.generation());
    let current = capture_mailbox_identity(successor.id());
    assert_eq!(send(successor.id(), get_lp_id(), 21, current), Ok(()));
    assert_eq!(send(root.id(), get_lp_id(), 22, captured), Err(22));
    assert_eq!(receive(root.id(), captured), None);
    assert_eq!(receive(successor.id(), current), Some(21));
    // Older live operations terminate after a staged close, but cannot publish
    // new storage or deliver/pop queued words through its admission fence.
    let mut pending = Operation::new(successor.id(), current).unwrap();
    pending.prepare().unwrap();
    let successor_budget = USER_MAILBOX_CAPS.read().get(&successor.id()).unwrap().budget.clone();
    assert_eq!(successor_budget.queue_used(), [2, 2 * bytes]);
    assert_eq!(budget.queue_used(), [0, 0]);
    assert_eq!(send(successor.id(), get_lp_id(), 23, current), Ok(()));
    let closing = memory::retirement::ClosingAddressSpace::begin(successor).unwrap();
    assert_eq!(pending.publish_send(get_lp_id(), 24), Err(24));
    assert_eq!(pending.receive(), None);
    assert_eq!(send(successor.id(), get_lp_id(), 25, current), Err(25));
    pending.finish().unwrap();
    assert_eq!(successor_budget.queue_used(), [1, bytes]);
    assert!(matches!(closing.poll().unwrap(), memory::retirement::CloseProgress::Complete));
    assert_eq!(successor_budget.queue_used(), [0, 0]);
    assert_eq!(mailbox_budget::queue_node_used(), baseline);
    mailbox_budget::test_queue_admission();
    abandoned();
    ACTIVE.store(false, Ordering::Release);
    crate::logln!(
        "[mailbox queue backing] {} requested bytes per {}-LP queue set; backing rejection \
         refunds, shared per-generation ceiling rejects third preparation, final detach retains \
         charge until release, successor and guarded abandonment preserve original ordinary \
         classification",
        bytes,
        get_lp_count()
    );
    crate::logln!(
        "[mailbox queue phases] {} preparation-entry and {} completion-entry boundaries outside \
         local lifecycle/mailbox/queue/table/physical/heap/capability guards; entry IRQ state \
         preserved",
        PREPARED.load(Ordering::Acquire),
        FINISHED.load(Ordering::Acquire)
    );
    crate::logln!(
        "[mailbox queue ownership] node rejection, competing creators, 256-word FIFO/backpressure \
         and wrap, heap-held publication/final detach, retired/staged/stale-root rejection; \
         guarded abandonment retains one exact root and its unused queue/node"
    );
}
fn abandoned() {
    let root = loader::create_user_address_space_handle();
    let mut operation = Operation::new(root.id(), capture_mailbox_identity(root.id())).unwrap();
    operation.prepare().unwrap();
    let budget = operation.0.budget.as_ref().unwrap().clone();
    let before = mailbox_budget::queue_node_used();
    let used = budget.queue_used();
    crate::capability::mark_platform(root);
    {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let table = memory::ADDRESS_SPACE_TABLE.lock();
        let kernel = memory::KERNEL_AS.lock();
        let physical = memory::PHYSICAL_FRAME_ALLOCATOR.lock();
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        mailbox_retirement::tests::under_registry_guards(|| drop(operation));
        drop(heap);
        drop(physical);
        drop(kernel);
        drop(table);
        drop(lifecycle);
    }
    assert_eq!(budget.queue_used(), used);
    assert_eq!(mailbox_budget::queue_node_used(), before);
    assert!(!USER_MAILBOX.read().contains_key(&root.id()));
    let closing = memory::retirement::ClosingAddressSpace::begin(root).unwrap();
    assert!(matches!(closing.poll().unwrap(), memory::retirement::CloseProgress::Pending(_)));
}
