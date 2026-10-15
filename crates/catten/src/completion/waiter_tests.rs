//! Exact-CQ preparation probes. Raw namespaces are private serialized fixtures;
//! their replacement is source-identity evidence, never physical root proof.

use super::*;
use crate::{
    klib::observer::{
        CallOnNotify,
        WaitSponsor,
        registration::RegistrationError,
        waiter_budget,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

pub(super) fn run() {
    const CLIENT: usize = 0x5e21;
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| {
        let _registry = COMPLETIONS.write();
    });
    open_address_space_with_cq(CLIENT, 8, 8).unwrap();
    let source = capture_cq_interest(CLIENT, 0).unwrap();
    let mut prepared = sponsor.prepare(&source.identity, Arc::downgrade(&observer)).unwrap();
    let token;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        token = source.publish_waiter(&mut prepared).unwrap();
    }
    assert!(token.is_owned());
    assert_eq!(sponsor.used(), 1);
    drop((token, prepared));
    assert_eq!(sponsor.used(), 0);

    let mut tokens = Vec::new();
    for _ in 0..waiter_budget::SOURCE_LIMIT {
        tokens.push(source.try_register_waiter(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    let mut full = sponsor.prepare(&source.identity, Arc::downgrade(&observer)).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(source.publish_waiter(&mut full).unwrap_err(), RegistrationError::ResourceLimit);
    }
    assert_eq!(sponsor.used(), waiter_budget::SOURCE_LIMIT + 1);
    drop(full);
    assert_eq!(sponsor.used(), waiter_budget::SOURCE_LIMIT);
    drop(tokens);
    assert_eq!(sponsor.used(), 0);

    // A producer wins after preparation, before publication. Recheck under
    // the registry returns ready without linking or disposing unused storage.
    let mut ready = sponsor.prepare(&source.identity, Arc::downgrade(&observer)).unwrap();
    source.wake();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(source.publish_waiter(&mut ready).unwrap().is_ready());
    }
    assert_eq!(source.identity.registered(), 0);
    assert_eq!(sponsor.used(), 1);
    drop(ready);
    assert_eq!(sponsor.used(), 0);

    let mut stale = sponsor.prepare(&source.identity, Arc::downgrade(&observer)).unwrap();
    close_address_space(CLIENT);
    open_address_space_with_cq(CLIENT, 8, 8).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(source.publish_waiter(&mut stale).unwrap_err(), RegistrationError::Closed);
    }
    assert_eq!(cq_waiter_count(CLIENT, 0), 0);
    assert_eq!(sponsor.used(), 1);
    drop(stale);
    assert_eq!(sponsor.used(), 0);
    close_address_space(CLIENT);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[CQ waiter preparation] heap-held success/full/ready/reused-source publication, retained \
         rejection charge and post-guard cancellation passed"
    );
}
