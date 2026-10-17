//! Private local-ledger controls do not publish roots or physical authority.
use super::*;
use crate::memory::{
    ADDRESS_SPACE_LIFECYCLE,
    ADDRESS_SPACE_TABLE,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

pub(super) fn run() {
    for reject in 1..=4 {
        let mut visits = 0;
        assert!(
            PreparingAccount::prepare(false, |stage| {
                visits += 1;
                assert!(LEDGER.try_lock().is_some());
                assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
                assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
                if stage == reject {
                    Err(AllocError)
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        assert_eq!(visits, reject);
    }
    let mut ledger = Ledger::new();
    for generation in 0..64 {
        let prepared = PreparingAccount::try_new().unwrap();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            prepared.publish_into(&mut ledger, (1, generation));
        }
        let retired = ledger.domains.take(&(1, generation)).unwrap();
        assert!(!retired.value().1.platform);
        assert_eq!(retired.value().1.waiters.used(), 0);
        assert_eq!(retired.value().1.timers.used(), 0);
        retired.release();
    }
    assert!(ledger.domains.iter().next().is_none());
    // Captured-only, node-only, node+waiter and complete preparation fallback.
    // This deliberately retains three empty nodes, two waiter sponsors and
    // one timer sponsor, without a root or any entry/event/memory charge.
    for stage in 0..4 {
        let mut owner = PreparingAccount(ManuallyDrop::new(Resources {
            limit: LEDGER.lock().default_limit(),
            platform: false,
            node: None,
            waiters: None,
            timers: None,
        }));
        if stage >= 1 {
            owner.0.node = Some(PreparedEntry::try_new().unwrap());
        }
        if stage >= 2 {
            owner.0.waiters = Some(WaitSponsor::try_new(false).unwrap());
        }
        if stage >= 3 {
            owner.0.timers = Some(SchedulerSponsor::try_new(false).unwrap());
        }
        let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
        let _table = ADDRESS_SPACE_TABLE.lock();
        let _ledger = LEDGER.lock();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(owner);
    }
    crate::logln!(
        "[memory sponsor preparation] four rejection phases, 64 heap-held publications and four \
         guarded abandonments passed; retains three nodes, two waiter sponsors and one timer \
         sponsor, no root or resource charge"
    );
}
