//! Serialized record-storage probes. Outer caller qualification is separate.
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
static REJECT: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static DISPOSED: AtomicUsize = AtomicUsize::new(0);

pub(super) fn reject() -> bool {
    REJECT.swap(false, Ordering::AcqRel)
}
pub(super) fn boundary(dispose: bool, entry_irq: bool) {
    if !ACTIVE.load(Ordering::Acquire) {
        return;
    }
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), entry_irq);
    available(|| CAPABILITIES.try_lock().is_some(), "record metadata capability registry");
    available(
        || memory::allocators::global_allocator::PRIMARY_ALLOCATOR.try_lock().is_some(),
        "record metadata heap",
    );
    if dispose {
        DISPOSED.fetch_add(1, Ordering::Relaxed);
    } else {
        ALLOCATED.fetch_add(1, Ordering::Relaxed);
    }
}
pub(super) fn boundary_after(entry_irq: bool) {
    if ACTIVE.load(Ordering::Acquire) {
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), entry_irq);
    }
}
fn available(mut probe: impl FnMut() -> bool, label: &'static str) {
    let deadline = crate::self_test::results::Deadline::after_millis(1000);
    while !probe() {
        deadline.assert_pending(label);
        core::hint::spin_loop();
    }
}
pub(crate) fn begin_real() {
    assert!(!ACTIVE.swap(true, Ordering::AcqRel));
    ALLOCATED.store(0, Ordering::Relaxed);
    DISPOSED.store(0, Ordering::Relaxed);
}
pub(crate) fn finish_real() {
    ACTIVE.store(false, Ordering::Release);
    crate::logln!(
        "[capability record phases] {} prepared-node and {} disposal boundaries outside local \
         capability/heap guards; entry IRQ state preserved",
        ALLOCATED.load(Ordering::Acquire),
        DISPOSED.load(Ordering::Acquire)
    );
}
fn account(root: memory::AddressSpaceHandle) -> Arc<budget::DomainBudget> {
    CAPABILITIES.lock().get(&root.id()).unwrap().budget.clone()
}
const KINDS: [ObjectKind; 6] = [
    ObjectKind::Ipc,
    ObjectKind::Memory,
    ObjectKind::Completion,
    ObjectKind::Device,
    ObjectKind::Mailbox,
    ObjectKind::SystemObserver,
];
pub(super) fn run() {
    begin_real();
    let node = budget::node_used();
    let root = loader::create_user_address_space_handle();
    let local = account(root);
    for kind in KINDS {
        REJECT.store(true, Ordering::Release);
        assert!(matches!(reserve(root.id(), kind), Err(AllocationError::AllocationFailed)));
        assert_eq!(CAPABILITIES.lock().get(&root.id()).unwrap().next_serial, 1);
        assert_eq!(local.used(), 0);
        assert_eq!(budget::node_used(), node);
    }
    let preparations = KINDS.map(|_| PreparingRecord::try_new().unwrap());
    let mut retired: [Option<RetiredRecord>; 6] = core::array::from_fn(|_| None);
    for (index, mut preparation) in preparations.into_iter().enumerate() {
        let kind = KINDS[index];
        {
            let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
            let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
            let cap =
                reserve_prepared_captured(root.id(), kind, Some(root), &mut None, &mut preparation)
                    .unwrap()
                    .publish()
                    .unwrap();
            assert!(contains(root.id(), cap, kind));
            let escrow = escrow_captured(root.id(), cap, kind, Some(root)).unwrap();
            assert!(!contains(root.id(), cap, kind));
            assert_eq!(escrow.rollback(), Ok(cap));
            retired[index] = retire_record(root.id(), cap, kind, |state| state == EntryState::Live);
            assert!(!contains(root.id(), cap, kind));
            drop(heap);
            drop(lifecycle);
        }
        preparation.finish();
    }
    assert_eq!(local.used(), 6);
    for record in retired.into_iter().flatten() {
        record.release();
    }
    assert_eq!(local.used(), 0);
    memory::close_user_address_space_handle(root).unwrap();

    let exhausted = loader::create_user_address_space_handle();
    let exhausted_account = account(exhausted);
    exhaust_identity_for_test(exhausted.id());
    assert!(matches!(
        reserve(exhausted.id(), ObjectKind::Memory),
        Err(AllocationError::IdentityExhausted)
    ));
    assert_eq!(exhausted_account.used(), 0);
    assert_eq!(CAPABILITIES.lock().get(&exhausted.id()).unwrap().next_serial, u64::MAX);
    memory::close_user_address_space_handle(exhausted).unwrap();
    batch();
    cancellations();
    abandonment(node);
    finish_real();
    crate::logln!(
        "[capability record ownership] all six kinds reject storage before serial/charge \
         mutation; heap-held captured admission/publication/escrow/detach; mixed batch rejection \
         is atomic, move retains its exact retired source charge until explicit completion; \
         cancellation and late tokens preserve exact accounts; guarded preparation/retirement \
         abandonment retains two original ordinary charges after root reuse"
    );
}
fn batch() {
    let source_root = loader::create_user_address_space_handle();
    let target_root = loader::create_user_address_space_handle();
    let source = account(source_root);
    let target = account(target_root);
    let moved_cap = try_allocate(source_root.id(), ObjectKind::Memory).unwrap();
    let loan_cap = try_allocate(source_root.id(), ObjectKind::Memory).unwrap();
    let mut moved = escrow(source_root.id(), moved_cap, ObjectKind::Memory).unwrap();
    let mut loan = escrow(source_root.id(), loan_cap, ObjectKind::Memory).unwrap();
    let mut destinations =
        [0, 1, 2].map(|_| reserve(target_root.id(), ObjectKind::Memory).unwrap());
    let ids = destinations.each_ref().map(Reservation::identity);
    {
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        let [a, b, c] = &mut destinations;
        publish_batch(&mut [
            Publication {
                destination: a,
                source: Some((&mut moved, SourceDisposition::Revoke)),
            },
            Publication {
                destination: b,
                source: Some((&mut loan, SourceDisposition::Restore)),
            },
            Publication {
                destination: c,
                source: None,
            },
        ])
        .unwrap();
        assert!(!contains(source_root.id(), moved_cap, ObjectKind::Memory));
        assert!(contains(source_root.id(), loan_cap, ObjectKind::Memory));
        for cap in ids {
            assert!(contains(target_root.id(), cap, ObjectKind::Memory));
        }
        assert!(moved.retired.is_some());
        assert!(loan.retired.is_none());
        assert_eq!(source.used(), 2);
        drop(heap);
    }
    moved.finish_retired();
    assert_eq!(source.used(), 1);
    drop(moved);
    drop(loan);
    drop(destinations);
    // Second batch rejects a retired destination before any source or output
    // mutation. Ordinary rollback restores only its existing source authority.
    let mut escrow = escrow(source_root.id(), loan_cap, ObjectKind::Memory).unwrap();
    let mut destination = reserve(target_root.id(), ObjectKind::Memory).unwrap();
    let staged = destination.identity();
    retire_address_space(target_root.id());
    {
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        assert_eq!(
            publish_batch(&mut [Publication {
                destination: &mut destination,
                source: Some((&mut escrow, SourceDisposition::Revoke))
            }]),
            Err(AllocationError::Retired)
        );
        assert!(escrow.active && escrow.retired.is_none());
        assert!(destination.active);
        assert!(!contains(source_root.id(), loan_cap, ObjectKind::Memory));
        assert!(!contains(target_root.id(), staged, ObjectKind::Memory));
        drop(heap);
    }
    assert_eq!(escrow.rollback(), Ok(loan_cap));
    drop(destination);
    assert_eq!(target.used(), 3);
    memory::close_user_address_space_handle(target_root).unwrap();
    memory::close_user_address_space_handle(source_root).unwrap();
    assert_eq!(source.used(), 0);
    assert_eq!(target.used(), 0);
}
fn cancellations() {
    let root = loader::create_user_address_space_handle();
    let local = account(root);
    drop(reserve(root.id(), ObjectKind::Memory).unwrap());
    assert_eq!(local.used(), 0);
    let cap = try_allocate(root.id(), ObjectKind::Memory).unwrap();
    drop(escrow(root.id(), cap, ObjectKind::Memory).unwrap());
    assert_eq!(local.used(), 0);
    let cap = try_allocate(root.id(), ObjectKind::Memory).unwrap();
    let token = escrow(root.id(), cap, ObjectKind::Memory).unwrap();
    assert!(remove_for_teardown(root.id(), cap, ObjectKind::Memory));
    assert_eq!(local.used(), 0);
    drop(token);
    let old = reserve(root.id(), ObjectKind::Memory).unwrap();
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(local.used(), 0);
    let successor = loader::create_user_address_space_handle();
    assert_eq!(successor.id(), root.id());
    let new = reserve(successor.id(), ObjectKind::Memory).unwrap();
    assert_eq!(new.identity(), 1);
    drop(old);
    assert_eq!(account(successor).used(), 1);
    drop(new);
    memory::close_user_address_space_handle(successor).unwrap();
}
fn abandonment(before: (usize, usize)) {
    let root = loader::create_user_address_space_handle();
    let local = account(root);
    let mut preparation = PreparingRecord::try_new().unwrap();
    preparation.test_charge(&local);
    let cap = try_allocate(root.id(), ObjectKind::Memory).unwrap();
    let retired =
        retire_record(root.id(), cap, ObjectKind::Memory, |state| state == EntryState::Live)
            .unwrap();
    mark_platform(root);
    {
        let lifecycle = memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let registry = CAPABILITIES.lock();
        let heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        drop(preparation);
        drop(retired);
        drop(heap);
        drop(registry);
        drop(lifecycle);
    }
    assert_eq!(local.used(), 2);
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(local.used(), 2);
    let successor = loader::create_user_address_space_handle();
    assert_eq!(successor.id(), root.id());
    assert_eq!(account(successor).used(), 0);
    assert_eq!(budget::node_used(), (before.0 + 2, before.1 + 2));
    memory::close_user_address_space_handle(successor).unwrap();
}
