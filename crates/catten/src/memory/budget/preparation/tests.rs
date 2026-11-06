//! Private local-ledger controls do not publish roots or physical authority.
use super::*;
use crate::memory::{
    ADDRESS_SPACE_LIFECYCLE,
    ADDRESS_SPACE_TABLE,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

pub(super) fn run() {
    let baseline = admission::snapshot();
    let accounts = crate::klib::observer::account_budget::snapshot();
    let timers = crate::timers::account_budget::snapshot();
    // Byte admission rejects even with count headroom, before allocator entry.
    assert!(
        admission::prepare_with::<
            [u8; crate::resource_policy::POLICY.root_metadata.ordinary[0] as usize],
        >(false, |_| panic!("oversized ordinary node reached allocator"))
        .is_err()
    );
    assert!(
        admission::prepare_with::<
            [u8; crate::resource_policy::POLICY.root_metadata.total[0] as usize],
        >(true, |_| panic!("oversized trusted node reached allocator"))
        .is_err()
    );
    assert_eq!(admission::snapshot(), baseline);
    for _ in 0..64 {
        assert!(
            admission::prepare_with::<(Identity, DomainAccount)>(false, |_| Err(AllocError))
                .is_err()
        );
        assert_eq!(admission::snapshot(), baseline);
    }
    for reject in 1..=4 {
        let mut visits = 0;
        assert!(
            PreparingAccount::prepare(false, false.into(), |stage| {
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
        assert_eq!(admission::snapshot(), baseline);
        assert_eq!(crate::klib::observer::account_budget::snapshot(), accounts);
        assert_eq!(crate::timers::account_budget::snapshot(), timers);
    }
    let mut ledger = Ledger::new();
    for generation in 0..64 {
        let prepared = PreparingAccount::try_new(false).unwrap();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            admission::with_locked(|| {
                crate::klib::observer::account_budget::with_locked(|| {
                    crate::timers::account_budget::with_locked(|| {
                        prepared.publish_into(&mut ledger, (1, generation));
                    });
                });
            });
        }
        assert_eq!(admission::snapshot().0[1], baseline.0[1] + 1);
        assert_eq!(admission::snapshot().1[1], baseline.1[1] + 1);
        assert_eq!(crate::timers::account_budget::snapshot(), (timers.0 + 1, timers.1 + 1));
        assert_eq!(
            crate::klib::observer::account_budget::snapshot(),
            (accounts.0 + 1, accounts.1 + 1)
        );
        let retired = ledger.domains.take(&(1, generation)).unwrap();
        assert!(!retired.value().1.platform);
        assert_eq!(retired.value().1.waiters.used(), 0);
        assert_eq!(retired.value().1.timers.used(), 0);
        retired.release();
        assert_eq!(admission::snapshot(), baseline);
        assert_eq!(crate::klib::observer::account_budget::snapshot(), accounts);
        assert_eq!(crate::timers::account_budget::snapshot(), timers);
    }
    assert!(ledger.domains.iter().next().is_none());
    // Captured-only, node-only, node+waiter and complete preparation fallback.
    // This deliberately retains three empty nodes, two waiter sponsors and
    // one timer sponsor, without a root or any entry/event/memory charge.
    // All three node charges must survive the inert fallback.
    for stage in 0..4 {
        let mut owner = PreparingAccount(ManuallyDrop::new(Resources {
            limit: LEDGER.lock().default_limit(),
            platform: false,
            node: None,
            waiters: None,
            timers: None,
        }));
        if stage >= 1 {
            owner.0.node = Some(admission::prepare(false).unwrap());
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
        admission::with_locked(|| {
            crate::klib::observer::account_budget::with_locked(|| {
                crate::timers::account_budget::with_locked(|| drop(owner));
            });
        });
    }
    let retained = admission::snapshot();
    let bytes =
        PreparedEntry::<(Identity, DomainAccount), admission::Charge>::allocation_bytes() as u64;
    assert_eq!(retained.0, [baseline.0[0] + 3 * bytes, baseline.0[1] + 3]);
    assert_eq!(retained.1, [baseline.1[0] + 3 * bytes, baseline.1[1] + 3]);
    assert_eq!(crate::klib::observer::account_budget::snapshot(), (accounts.0 + 2, accounts.1 + 2));
    assert_eq!(crate::timers::account_budget::snapshot(), (timers.0 + 1, timers.1 + 1));
    crate::logln!(
        "[memory sponsor preparation] 64 allocator rejections, four rejection phases, 64 \
         heap/pool-held publications and four guarded abandonments passed; retains three charged \
         {}-byte nodes, two waiter sponsors and one timer sponsor, no root or object/event/entry \
         charge",
        bytes
    );
}
