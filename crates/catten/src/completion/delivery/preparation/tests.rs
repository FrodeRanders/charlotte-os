//! Private owners force registry/heap orderings without replacing live budgets.

use super::*;
use crate::{
    cpu::isa::interface::memory::address::PhysicalAddress,
    klib::observer::{
        CallOnNotify,
        ObserverRef,
        WaitSponsor,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

const CLIENT: usize = 0x5e33;

fn allocation_and_replacement() {
    open_address_space_with_cq(CLIENT, 8, 8).unwrap();
    let account = cq_admission(CLIENT).unwrap();
    let original_charge = account.used();
    for reject_list in [true, false] {
        let result = Prepared::new_with(
            Target::capture(CLIENT).unwrap(),
            8,
            None,
            |platform| {
                {
                    let _registry = COMPLETIONS.write();
                }
                if reject_list {
                    Err(RegistrationError::AllocationFailed)
                } else {
                    ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
                }
            },
            |_| {
                let _registry = COMPLETIONS.write();
                Err(cq::CqRingError::AllocationFailed)
            },
            entry_budget::prepare,
        );
        assert!(matches!(
            result,
            Err(CqOpenError::AllocationFailed
                | CqOpenError::Ring(cq::CqRingError::AllocationFailed))
        ));
        assert_eq!(account.used(), original_charge);
    }
    let mut first = Prepared::new_with(
        Target::capture(CLIENT).unwrap(),
        8,
        None,
        |platform| {
            {
                let _registry = COMPLETIONS.write();
            }
            ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
        },
        |entries| {
            {
                let _registry = COMPLETIONS.write();
            }
            cq::CompletionQueueRing::new_page(entries)
        },
        entry_budget::prepare,
    )
    .unwrap();
    let mut second = Prepared::new(Target::capture(CLIENT).unwrap(), 8, None).unwrap();
    let sponsor = WaitSponsor::new(false);
    let observer: ObserverRef =
        CallOnNotify::for_test(|| panic!("replacement fabricated readiness"));
    let old = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
    let token = sponsor.register(old.waiter_source(), ObserverRef::downgrade(&observer)).unwrap();
    let retired1;
    let retired2;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        retired1 = first.publish(DEFAULT_CQ).unwrap().unwrap();
        retired2 = second.publish(DEFAULT_CQ).unwrap().unwrap();
        assert_eq!(account.used()[0], 3);
        assert_eq!(sponsor.used(), 1);
        assert!(first.0.state.is_none() && second.0.state.is_none());
    }
    retired1.finish();
    retired2.finish();
    first.finish();
    second.finish();
    assert_eq!(account.used(), original_charge);
    assert_eq!(sponsor.used(), 0);
    drop(token);
    close_address_space(CLIENT);
    assert_eq!(account.used(), [0, 0]);
}

fn stale_and_retired() {
    open_address_space_with_cq(CLIENT, 8, 8).unwrap();
    let account = cq_admission(CLIENT).unwrap();
    let mut stale = Prepared::new(Target::capture(CLIENT).unwrap(), 8, None).unwrap();
    open_address_space_with_cq(CLIENT, 8, 8).unwrap();
    let successor = cq_admission(CLIENT).unwrap();
    let charge = successor.used();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(stale.publish(DEFAULT_CQ), Err(CqOpenError::UnknownAddressSpace)));
        assert_eq!(account.used()[0], 1);
        assert_eq!(successor.used(), charge);
    }
    stale.finish();
    assert_eq!(account.used(), [0, 0]);
    close_address_space(CLIENT);

    let root = crate::service::loader::create_user_address_space_handle();
    open_address_space_with_cq(root.id(), 8, 8).unwrap();
    let account = cq_admission(root.id()).unwrap();
    let mut queue = Prepared::new(Target::capture(root.id()).unwrap(), 8, None).unwrap();
    let mut namespace = PreparedNamespace::new(root.id(), 8, 8, None).unwrap();
    let new_account = namespace.0.queue.0.target.account.clone();
    crate::memory::budget::retire(root);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(queue.publish(DEFAULT_CQ), Err(CqOpenError::RetiringAddressSpace)));
        assert!(matches!(namespace.publish(), Err(CqOpenError::RetiringAddressSpace)));
        assert_eq!(account.used()[0], 2);
        assert_eq!(new_account.used()[0], 1);
    }
    queue.finish();
    namespace.finish();
    assert_eq!(account.used()[0], 1);
    assert_eq!(new_account.used(), [0, 0]);
    crate::memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(account.used(), [0, 0]);
}

fn physical_publication() {
    let (root, principal) = entry_budget::test_principal_root(b"completion-physical-ring", false);
    let asid = root.id();
    let other = CLIENT + 1;
    let used = crate::memory::kernel_buffers::test_metadata_used;
    let frame = crate::memory::PHYSICAL_FRAME_ALLOCATOR.lock().allocate_frame().unwrap();
    // Fixture owns this frame until both namespaces and every preparation leave.
    let page = unsafe { frame.into_hhdm_mut::<u8>() };
    unsafe {
        core::ptr::write_bytes(page, 0xa5, 4096);
    }
    open_address_space(asid, 8);
    let account = cq_admission(asid).unwrap();
    let base = used(principal);
    let mut prepared = Prepared::new(Target::capture(asid).unwrap(), 8, Some(frame)).unwrap();
    let node = PreparedEntry::<(CqId, CqState)>::allocation_bytes() as u64;
    let slots = charlotte_lifecycle::resources::queue_backing_slots(8).unwrap();
    let backlog = (slots * core::mem::size_of::<BacklogEntry>()) as u64;
    assert_eq!(used(principal), base + node + backlog);
    assert!(unsafe { core::slice::from_raw_parts(page, 4096) }.iter().all(|byte| *byte == 0xa5));
    // A competing queue takes the frame after capture. Both attachment and
    // whole-namespace publication reject without touching its pending result.
    open_address_space_with_cq_phys(other, 8, frame, 8).unwrap();
    let cap = submit(other, OpCode::Nop, None).unwrap();
    complete(other, cap, OpResult::Ok(91)).unwrap();
    let mut namespace = PreparedNamespace::new(asid, 8, 8, Some(frame)).unwrap();
    let new_account = namespace.0.queue.0.target.account.clone();
    let live = used(principal);
    let snapshot = Vec::from(unsafe { core::slice::from_raw_parts(page, 4096) });
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(prepared.publish(DEFAULT_CQ), Err(CqOpenError::RingInUse)));
        assert!(matches!(namespace.publish(), Err(CqOpenError::RingInUse)));
        assert_eq!(account.used()[0], 1);
        assert_eq!(new_account.used()[0], 1);
        assert_eq!(unsafe { core::slice::from_raw_parts(page, 4096) }, snapshot.as_slice());
    }
    assert_eq!(used(principal), live);
    namespace.finish();
    assert_eq!(used(principal), base + node + backlog);
    assert_eq!(new_account.used(), [0, 0]);
    close_address_space(other);
    // No consumer remains. The original preparation may now publish once.
    // Fresh physical publication now relinks its prepared map node too.
    let retired;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        retired = prepared.publish(DEFAULT_CQ).unwrap();
        assert!(prepared.0.state.is_none() && prepared.0.node.is_none());
        assert!(retired.is_none());
        assert_eq!(account.used()[0], 1);
        assert_eq!(unsafe { (*cq_ring_of(asid, DEFAULT_CQ).unwrap()).capacity() }, 8);
    }
    if let Some(retired) = retired {
        retired.finish();
    }
    assert_eq!(account.used()[0], 1);
    close_address_space(asid);
    assert_eq!(account.used(), [0, 0]);
    prepared.finish();
    crate::memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(used(principal), 0);
    crate::logln!(
        "[completion physical principal] backlog={} bytes, borrowed frame excluded, alias \
         rejection retained both preparations",
        backlog
    );
    crate::memory::PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame).unwrap();
}

fn namespace_publication() {
    open_address_space_with_cq(CLIENT, 8, 8).unwrap();
    let old_account = cq_admission(CLIENT).unwrap();
    let mut prepared = PreparedNamespace::new(CLIENT, 8, 8, None).unwrap();
    let account = prepared.0.queue.0.target.account.clone();
    let retired;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        retired = prepared.publish().unwrap().unwrap();
        assert!(prepared.0.namespace.is_published());
        assert_eq!(old_account.used()[0], 1);
        assert_eq!(account.used()[0], 1);
    }
    retired.finish();
    prepared.finish();
    assert_eq!(old_account.used(), [0, 0]);
    // Authority/map metadata remains a separate boundary. Exercise actual
    // capability revocation through the public adapter outside the heap guard.
    let cap = submit(CLIENT, OpCode::Nop, None).unwrap();
    open_address_space_with_cq(CLIENT, 8, 8).unwrap();
    assert!(!crate::capability::contains(CLIENT, cap, crate::capability::ObjectKind::Completion));
    assert_eq!(account.used(), [0, 0]);
    close_address_space(CLIENT);
}

// Dropping this ManuallyDrop wrapper is the negative control: it must do nothing.
#[allow(clippy::drop_non_drop)]
fn guarded_namespace_abandonment() {
    // Private unpublished namespace with funded backing. This retained owner
    // does not hold a root lease or physical ring authority.
    let backing = crate::timers::account_budget::snapshot();
    let accounts = account_budget::snapshot();
    let (root, principal) =
        entry_budget::test_principal_root(b"completion-namespace-abandoned", false);
    let used = crate::memory::kernel_buffers::test_metadata_used;
    let root_bytes = used(principal);
    let prepared = PreparedNamespace::new(root.id(), 8, 8, None).unwrap();
    let retained = used(principal);
    assert!(retained > root_bytes);
    let account = prepared.0.queue.0.target.account.clone();
    let timers = Arc::downgrade(&prepared.0.namespace.entries().timer_budget);
    let records = Arc::downgrade(&prepared.0.namespace.entries().record_budget);
    let watches = Arc::downgrade(&prepared.0.namespace.entries().watch_budget);
    let before = account.used();
    assert_eq!(before[0], 1);
    {
        let _registry = COMPLETIONS.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        crate::timers::account_budget::with_locked(|| {
            account_budget::with_locked(|| {
                node_budget::with_locked(|| {
                    entry_budget::with_locked(|| {
                        crate::memory::kernel_buffers::test_with_history_locked(|| drop(prepared));
                    })
                });
            });
        });
    }
    assert_eq!(used(principal), retained);
    crate::memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(used(principal), retained - root_bytes);
    assert_eq!(account.used(), before);
    assert!(timers.upgrade().is_some());
    assert!(records.upgrade().is_some());
    assert!(watches.upgrade().is_some());
    assert_eq!(crate::timers::account_budget::snapshot(), (backing.0 + 1, backing.1 + 1));
    assert_eq!(account_budget::snapshot(), (accounts.0 + 3, accounts.1 + 3));
    crate::logln!(
        "[CQ namespace retention] heap/registry-held unpublished namespace abandonment retained \
         global node, all original accounts, complete payload and {} original-principal bytes",
        retained - root_bytes
    );
}

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    allocation_and_replacement();
    stale_and_retired();
    physical_publication();
    namespace_publication();
    guarded_namespace_abandonment();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[CQ payload preparation] unlocked list/ring allocation and partial rejection, heap-held \
         competing/stale/retired/aliased publication, unchanged physical bytes, exact original \
         charges and whole namespace replacement passed"
    );
}
