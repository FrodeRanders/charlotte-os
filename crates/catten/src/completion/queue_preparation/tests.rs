//! Private owners force registry/heap orderings without replacing live budgets.

use super::*;
use crate::{
    cpu::isa::interface::memory::address::PhysicalAddress,
    klib::observer::{
        CallOnNotify,
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
    )
    .unwrap();
    let mut second = Prepared::new(Target::capture(CLIENT).unwrap(), 8, None).unwrap();
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> =
        CallOnNotify::new(|| panic!("replacement fabricated readiness"));
    let old = capture_cq_interest(CLIENT, DEFAULT_CQ).unwrap();
    let token = sponsor.register(&old.identity, Arc::downgrade(&observer)).unwrap();
    let retired1;
    let retired2;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        retired1 = first.publish(DEFAULT_CQ).unwrap().unwrap();
        retired2 = second.publish(DEFAULT_CQ).unwrap().unwrap();
        assert_eq!(account.used()[0], 3);
        assert_eq!(sponsor.used(), 1);
        assert!(first.state.is_none() && second.state.is_none());
    }
    drop((retired1, retired2, first, second));
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
    drop(stale);
    assert_eq!(account.used(), [0, 0]);
    close_address_space(CLIENT);

    let root = crate::service::loader::create_user_address_space_handle();
    open_address_space_with_cq(root.id(), 8, 8).unwrap();
    let account = cq_admission(root.id()).unwrap();
    let mut queue = Prepared::new(Target::capture(root.id()).unwrap(), 8, None).unwrap();
    let mut namespace = PreparedNamespace::new(root.id(), 8, 8, None).unwrap();
    let new_account = namespace.0.queue.target.account.clone();
    crate::memory::budget::retire(root);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(queue.publish(DEFAULT_CQ), Err(CqOpenError::RetiringAddressSpace)));
        assert!(matches!(namespace.publish(), Err(CqOpenError::RetiringAddressSpace)));
        assert_eq!(account.used()[0], 2);
        assert_eq!(new_account.used()[0], 1);
    }
    drop(queue);
    namespace.finish();
    assert_eq!(account.used()[0], 1);
    assert_eq!(new_account.used(), [0, 0]);
    crate::memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(account.used(), [0, 0]);
}

fn physical_publication() {
    let frame = crate::memory::PHYSICAL_FRAME_ALLOCATOR.lock().allocate_frame().unwrap();
    // Fixture owns this frame until both namespaces and every preparation leave.
    let page = unsafe { frame.into_hhdm_mut::<u8>() };
    unsafe {
        core::ptr::write_bytes(page, 0xa5, 4096);
    }
    open_address_space(CLIENT, 8);
    let account = cq_admission(CLIENT).unwrap();
    let mut prepared = Prepared::new(Target::capture(CLIENT).unwrap(), 8, Some(frame)).unwrap();
    assert!(unsafe { core::slice::from_raw_parts(page, 4096) }.iter().all(|byte| *byte == 0xa5));
    // A competing queue takes the frame after capture. Both attachment and
    // whole-namespace publication reject without touching its pending result.
    open_address_space_with_cq_phys(CLIENT + 1, 8, frame, 8).unwrap();
    let cap = submit(CLIENT + 1, OpCode::Nop, None).unwrap();
    complete(CLIENT + 1, cap, OpResult::Ok(91)).unwrap();
    let mut namespace = PreparedNamespace::new(CLIENT, 8, 8, Some(frame)).unwrap();
    let new_account = namespace.0.queue.target.account.clone();
    let snapshot = Vec::from(unsafe { core::slice::from_raw_parts(page, 4096) });
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(matches!(prepared.publish(DEFAULT_CQ), Err(CqOpenError::RingInUse)));
        assert!(matches!(namespace.publish(), Err(CqOpenError::RingInUse)));
        assert_eq!(account.used()[0], 1);
        assert_eq!(new_account.used()[0], 1);
        assert_eq!(unsafe { core::slice::from_raw_parts(page, 4096) }, snapshot.as_slice());
    }
    namespace.finish();
    assert_eq!(new_account.used(), [0, 0]);
    close_address_space(CLIENT + 1);
    // No consumer remains. The original preparation may now publish once.
    // Preinsert a heap queue so the actual map replacement needs no new node.
    open_cq(CLIENT, DEFAULT_CQ, 8).unwrap();
    let retired;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        retired = prepared.publish(DEFAULT_CQ).unwrap();
        assert!(prepared.state.is_none());
        assert_eq!(account.used()[0], 2);
        assert_eq!(unsafe { (*cq_ring_of(CLIENT, DEFAULT_CQ).unwrap()).capacity }, 8);
    }
    drop(retired);
    assert_eq!(account.used()[0], 1);
    close_address_space(CLIENT);
    assert_eq!(account.used(), [0, 0]);
    drop(prepared);
    crate::memory::PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame).unwrap();
}

fn namespace_publication() {
    open_address_space_with_cq(CLIENT, 8, 8).unwrap();
    let old_account = cq_admission(CLIENT).unwrap();
    let mut prepared = PreparedNamespace::new(CLIENT, 8, 8, None).unwrap();
    let account = prepared.0.queue.target.account.clone();
    let retired;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        retired = prepared.publish().unwrap().unwrap();
        assert!(prepared.0.entries.is_none());
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
    // Private unpublished raw namespace: bounded deliberate retention, with no
    // live root or replacement sponsor and no physical backing authority.
    let prepared = PreparedNamespace::new(CLIENT + 20, 8, 8, None).unwrap();
    let account = prepared.0.queue.target.account.clone();
    let before = account.used();
    assert_eq!(before[0], 1);
    {
        let _registry = COMPLETIONS.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(prepared);
    }
    assert_eq!(account.used(), before);
    crate::logln!(
        "[CQ namespace retention] heap/registry-held unpublished namespace abandonment retained \
         complete payload and original CQ charge"
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
