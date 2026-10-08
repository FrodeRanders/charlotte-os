//! Real detached roots, bounded admission, and controlled unlocked faults.
use super::*;
use crate::{
    memory::{
        self,
        PHYSICAL_FRAME_ALLOCATOR,
        backing_budget::{
            self,
            Kind,
        },
    },
    service::loader,
};

fn root() -> RetiredAddressSpace {
    let handle = loader::create_user_address_space_handle();
    assert!(memory::commit_user_heap_page_handle(handle, charlotte_launch::HEAP_VADDR));
    super::super::tests::stage(handle)
}
fn release(frame: PAddr) -> Result<(), physical::Error> {
    PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame)
}
fn status(registry: &Registry, ticket: Ticket) -> Status {
    registry.snapshot().entries[ticket.index].unwrap()
}

pub(crate) fn run() {
    test_retry_and_reuse();
    test_limits_and_abandonment();
    test_serial_exhaustion();
    test_physical_rejection();
    test_status_interruption();
    test_production_adapter();
    assert_eq!(status_words(false), [0; 8]);
    let words = status_words(true);
    assert_eq!(words[7], CAPACITY as u64);
    assert!(words[..6].iter().sum::<u64>() <= CAPACITY as u64);
    crate::logln!(
        "[root recovery registry] bounded custody, exact tickets, unlocked retry, attempt/slot \
         limits, abandonment and terminal physical rejection passed"
    );
}

fn test_retry_and_reuse() {
    let before = backing_budget::test_used_pages(Kind::Heap);
    let registry = Registry::new();
    let owner = root();
    let handle = owner.handle;
    let ticket = registry.retain(owner).unwrap_or_else(|_| panic!("empty registry rejected"));
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let first = registry
        .retry_with(
            ticket,
            |_, captured| {
                assert_eq!(captured, handle);
                assert!(registry.inner.try_lock().is_some());
                super::super::tests::assert_detached(handle);
                assert_eq!(registry.claim(ticket).err(), Some(RetryError::Busy));
                assert_eq!(status(&registry, ticket).state, State::Running);
                false
            },
            &mut |_| panic!("failed invalidation released backing"),
        )
        .unwrap();
    assert_eq!(first.state, State::AwaitingRetry);
    assert_eq!(first.attempts, 1);
    assert_eq!(first.heap_pages, 1);
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    let intervening = loader::create_user_address_space_handle();
    assert_ne!(intervening.id(), handle.id());
    let result = registry
        .retry_with(ticket, super::super::invalidate, &mut |frame| {
            assert!(registry.inner.try_lock().is_some());
            assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
            assert!(memory::ADDRESS_SPACE_TABLE.try_lock().is_some());
            release(frame)
        })
        .unwrap();
    assert_eq!(result.state, State::Recovered);
    assert_eq!(result.rejected_frames, 0);
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before);
    let successor = loader::create_user_address_space_handle();
    assert_eq!(successor.id(), handle.id());
    assert_ne!(successor, handle);
    assert_eq!(registry.claim(ticket).err(), Some(RetryError::Terminal));
    memory::close_user_address_space_handle(successor).unwrap();
    memory::close_user_address_space_handle(intervening).unwrap();
    let next = registry.retain(root()).unwrap_or_else(|_| panic!("completed slot not reusable"));
    assert_eq!(next.index, ticket.index);
    assert_ne!(next.serial, ticket.serial);
    assert_eq!(registry.claim(ticket).err(), Some(RetryError::StaleTicket));
    assert_eq!(status(&registry, next).state, State::AwaitingRetry);
    assert_eq!(
        registry.retry_with(next, super::super::invalidate, &mut release).unwrap().state,
        State::Recovered
    );
}

fn test_limits_and_abandonment() {
    let before = backing_budget::test_used_pages(Kind::Heap);
    let registry = Registry::new();
    let mut tickets = [Ticket {
        index: 0,
        serial: 0,
    }; CAPACITY];
    for ticket in &mut tickets {
        *ticket = registry.retain(root()).unwrap_or_else(|_| panic!("bounded slot rejected"));
    }
    let extra = root();
    let handle = extra.handle;
    let extra = registry.retain(extra).expect_err("full registry admitted another receipt");
    assert_eq!(registry.snapshot().rejected_admissions, 1);
    // Rejection returned ownership outside the hold; no root/slot is lost.
    assert!(extra.release_retry_with(super::super::invalidate).is_ok());
    let successor = loader::create_user_address_space_handle();
    assert_eq!(successor.id(), handle.id());
    memory::close_user_address_space_handle(successor).unwrap();
    let first = tickets[0];
    for count in 1..=MAX_ATTEMPTS {
        let result =
            registry.retry_with(first, |_, _| false, &mut |_| panic!("unconfirmed free")).unwrap();
        assert_eq!(result.attempts, count);
        assert_eq!(
            result.state,
            if count == MAX_ATTEMPTS {
                State::RetryLimit
            } else {
                State::AwaitingRetry
            }
        );
    }
    assert_eq!(registry.claim(first).err(), Some(RetryError::RetryLimit));
    let attempt = registry.claim(tickets[1]).unwrap();
    let retained_root = attempt.owner.as_ref().unwrap().handle;
    // Dropping beneath the registry lock must acquire no lock or physically
    // destroy its root. The stable cell preserves the terminal diagnostic.
    {
        let _guard = registry.inner.lock();
        drop(attempt);
    }
    assert_eq!(status(&registry, tickets[1]).state, State::Abandoned);
    assert_eq!(registry.claim(tickets[1]).err(), Some(RetryError::Terminal));
    assert!(memory::current_address_space_handle(retained_root.id()).is_none());
    for &ticket in &tickets[2..] {
        assert_eq!(
            registry.retry_with(ticket, super::super::invalidate, &mut release).unwrap().state,
            State::Recovered
        );
    }
    // Local registry Drop retains the exhausted owner, too; no abandonment
    // cancellation, account refund, slot return or recovery bypass exists.
    drop(registry);
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 2);
}

fn test_serial_exhaustion() {
    let registry = Registry::new();
    registry.inner.lock().next_serial = u64::MAX;
    let owner = registry.retain(root()).expect_err("receipt serial wrapped");
    assert!(registry.snapshot().entries.iter().all(Option::is_none));
    assert_eq!(registry.snapshot().rejected_admissions, 1);
    assert!(owner.release_retry_with(super::super::invalidate).is_ok());
    assert_eq!(
        registry
            .claim(Ticket {
                index: CAPACITY,
                serial: 1
            })
            .err(),
        Some(RetryError::StaleTicket)
    );
}

fn test_physical_rejection() {
    let registry = Registry::new();
    let before = backing_budget::test_used_pages(Kind::Heap);
    let owner = root();
    let handle = owner.handle;
    let ticket = registry.retain(owner).unwrap_or_else(|_| panic!("admission"));
    let mut calls = 0;
    let result = registry
        .retry_with(ticket, super::super::invalidate, &mut |frame| {
            calls += 1;
            assert!(registry.inner.try_lock().is_some());
            if calls == 1 {
                Err(physical::Error::CannotDeallocateUnallocatedFrame)
            } else {
                release(frame)
            }
        })
        .unwrap();
    assert!(calls > 1, "fault did not exercise a partial owning walk");
    assert_eq!(result.state, State::Quarantined);
    assert_eq!(result.rejected_frames, 1);
    assert_eq!(registry.claim(ticket).err(), Some(RetryError::Terminal));
    assert_eq!(backing_budget::test_used_pages(Kind::Heap), before + 1);
    // Confirmed invalidation permits slot reuse; rejected backing remains
    // charged and can never be walked again through this terminal receipt.
    let successor = loader::create_user_address_space_handle();
    assert_eq!(successor.id(), handle.id());
    assert_ne!(successor, handle);
    memory::close_user_address_space_handle(successor).unwrap();
}

fn test_status_interruption() {
    let registry = Registry::new();
    let ticket = registry.retain(root()).unwrap_or_else(|_| panic!("admission"));
    let mut attempt = registry.claim(ticket).unwrap();
    let owner = attempt.owner.take().unwrap();
    assert!(owner.release_retry_with(super::super::invalidate).is_ok());
    // Completion was lost after the physical phase. No authority remains to
    // replay it, even though this controlled walk happened to finish safely.
    drop(attempt);
    assert_eq!(status(&registry, ticket).state, State::Abandoned);
    assert_eq!(registry.claim(ticket).err(), Some(RetryError::Terminal));
}

fn test_production_adapter() {
    let owner = root();
    let handle = owner.handle;
    assert_eq!(
        owner.release_with(|_, _| false),
        Err(memory::AddressSpaceCloseError::QuiescenceFailed)
    );
    let ticket =
        snapshot().entries.into_iter().flatten().find(|entry| entry.root == handle).unwrap().ticket;
    if crate::cpu::isa::lp::ops::get_int_state() {
        assert_eq!(retry(ticket).unwrap().state, State::Recovered);
        assert_eq!(retry(ticket), Err(RetryError::Terminal));
    } else {
        assert_eq!(retry(ticket), Err(RetryError::Masked));
        assert_eq!(status(&ROOT_RECOVERY, ticket).attempts, 0);
        // Serialized boot fixtures use the private fault adapter. Production
        // rejects this enclosing IRQ state before claiming a receipt.
        assert_eq!(
            ROOT_RECOVERY.retry_with(ticket, super::super::invalidate, &mut release).unwrap().state,
            State::Recovered
        );
    }
}
