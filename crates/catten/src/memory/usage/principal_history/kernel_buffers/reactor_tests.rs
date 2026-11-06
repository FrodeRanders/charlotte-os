//! Native actual-layout controls; trusted fixtures prebind principals before
//! root construction. Signed-launch verification is qualified separately.
use super::*;
use crate::{
    completion,
    klib::observer::WaitSponsor,
    memory::{
        self,
        AddressSpace,
        AddressSpaceHandle,
    },
    timers::budget::SchedulerSponsor,
};

fn domain(principal: u64) -> AddressSpaceHandle {
    let space =
        AddressSpace::try_new_principal_user(false, prepare_backing(principal, false).unwrap())
            .unwrap();
    memory::register_user_address_space(space).unwrap()
}
fn pools() -> [(u64, u64); 3] {
    [
        crate::klib::observer::account_budget::snapshot(),
        crate::timers::account_budget::snapshot(),
        completion::account_budget::snapshot(),
    ]
}
pub(super) fn run() {
    let principal = charlotte_launch::artifact_principal_id(b"reactor-account-principal");
    let other = charlotte_launch::artifact_principal_id(b"reactor-account-healthy");
    let before = pools();
    let first = domain(principal);
    let mandatory = test_reactor_used(principal);
    let second = domain(principal);
    let healthy = domain(other);
    assert_eq!(test_reactor_used(principal), 2 * mandatory);
    let storage = capture_storage(first).unwrap();
    macro_rules! measure {
        ($construct:expr) => {{
            let before = test_reactor_used(principal);
            let account = $construct.unwrap();
            let bytes = test_reactor_used(principal) - before;
            assert!(bytes > 0);
            drop(account);
            assert_eq!(test_reactor_used(principal), before);
            bytes
        }};
    }
    let waiter = measure!(WaitSponsor::try_new_with_backing_class(false, storage));
    let scheduler = measure!(SchedulerSponsor::try_new_with_backing_class(false, storage));
    let timer = measure!(crate::timers::budget::DomainBudget::try_new(4, storage));
    let record = measure!(completion::budget::DomainBudget::try_new(4, storage));
    let queue = measure!(completion::cq_budget::DomainBudget::try_new(storage));
    let watch = measure!(completion::watch_budget::DomainBudget::try_new(4, storage));
    assert_eq!(mandatory, waiter + scheduler);
    completion::open_address_space_with_cq(first.id(), 4, 2).unwrap();
    let shared = 2 * mandatory + timer + record + queue + watch;
    assert_eq!(test_reactor_used(principal), shared);
    let established = completion::record_admission(first.id()).unwrap();
    let family = pools();
    memory::budget::mark_platform(first);
    assert_eq!(pools(), family);
    test_reactor_limit(principal, shared);
    {
        let _heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        for _ in 0..64 {
            assert!(WaitSponsor::try_new_with_backing_class(false, storage).is_err());
            assert!(SchedulerSponsor::try_new_with_backing_class(false, storage).is_err());
            assert!(crate::timers::budget::DomainBudget::try_new(4, storage).is_err());
            assert!(completion::budget::DomainBudget::try_new(4, storage).is_err());
            assert!(completion::cq_budget::DomainBudget::try_new(storage).is_err());
            assert!(completion::watch_budget::DomainBudget::try_new(4, storage).is_err());
        }
    }
    assert_eq!(pools(), family);
    assert_eq!(test_reactor_used(principal), shared);
    // Real root rejection at waiter and scheduler preparation, including holder
    // admitted/payload rejected prefixes; frames and publication stay unchanged.
    for headroom in [0, waiter - 1, waiter, waiter + scheduler - 1] {
        test_reactor_limit(principal, shared + headroom);
        let frames = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let roots = memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|r| r.is_some()).count();
        let space =
            AddressSpace::try_new_principal_user(false, prepare_backing(principal, false).unwrap())
                .unwrap();
        assert_eq!(
            memory::register_user_address_space(space),
            Err(memory::AddressSpaceRegistrationError::MemorySponsorAllocationFailed)
        );
        assert_eq!(test_reactor_used(principal), shared);
        assert_eq!(pools(), family);
        assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
        assert_eq!(
            memory::ADDRESS_SPACE_TABLE.lock().iter().filter(|r| r.is_some()).count(),
            roots
        );
    }
    // Reject each namespace prefix while preserving the existing accounts/CQ.
    for headroom in [
        0,
        timer - 1,
        timer,
        timer + record - 1,
        timer + record,
        timer + record + queue - 1,
        timer + record + queue,
        timer + record + queue + watch - 1,
    ] {
        test_reactor_limit(principal, shared + headroom);
        assert!(matches!(
            completion::open_address_space_with_cq(first.id(), 4, 2),
            Err(completion::CqOpenError::AllocationFailed)
        ));
        assert_eq!(test_reactor_used(principal), shared);
        assert_eq!(pools(), family);
        assert!(alloc::sync::Arc::ptr_eq(
            &established,
            &completion::record_admission(first.id()).unwrap()
        ));
    }
    completion::open_address_space_with_cq(healthy.id(), 4, 2).unwrap();
    assert_eq!(test_reactor_used(other), mandatory + timer + record + queue + watch);
    drop(established);
    test_reactor_limit(
        principal,
        crate::resource_policy::POLICY.memory.principal_reactor_account_bytes,
    );
    for root in [first, second, healthy] {
        memory::close_user_address_space_handle(root).unwrap();
    }
    assert!(capture_storage(first).is_err());
    assert_eq!(test_reactor_used(principal), 0);
    assert_eq!(test_reactor_used(other), 0);
    assert_eq!(pools(), before);
    // Real record bundles make the principal byte ceiling bind with global
    // ordinary account slots remaining. No counter-only simulated pressure.
    let pressure_root = domain(principal);
    let storage = capture_storage(pressure_root).unwrap();
    let baseline = test_reactor_used(principal);
    let ceiling = crate::resource_policy::POLICY.memory.principal_reactor_account_bytes;
    let mut held = alloc::vec::Vec::with_capacity((ceiling / record) as usize + 1);
    while let Ok(account) = completion::budget::DomainBudget::try_new(1, storage) {
        held.push(account);
        assert!(held.len() <= (ceiling / record) as usize);
    }
    assert!(ceiling - test_reactor_used(principal) < record);
    assert!(completion::account_budget::snapshot().1 < completion::account_budget::ORDINARY_LIMIT);
    let count = held.len();
    drop(held);
    assert_eq!(test_reactor_used(principal), baseline);
    memory::close_user_address_space_handle(pressure_root).unwrap();
    assert_eq!(test_reactor_used(principal), 0);
    assert_eq!(pools(), before);
    crate::logln!(
        "[reactor account principal] waiter/scheduler/timer/record/CQ/watch \
         bundles={}/{}/{}/{}/{}/{} bytes; sibling sharing, 384 heap-held denials, four root and \
         eight namespace prefixes, healthy sponsor, immutable promotion and {} real bundles at \
         {}-byte ceiling passed",
        waiter,
        scheduler,
        timer,
        record,
        queue,
        watch,
        count,
        ceiling
    );
}
