//! Serialized real IPC publication with prepared subscription ownership.
//! Raw fixture namespaces do not authorize production ASID-based cleanup.

use super::*;
use crate::{
    klib::observer::CallOnNotify,
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

const SERVER: usize = 0x000e_a127;
const CLIENT: usize = 0x000e_a128;

pub(super) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let sponsor = WaitSponsor::new(false);
    let observer: Arc<dyn Observer> = CallOnNotify::new(|| {
        let _ipc = IPC.write();
    });
    let endpoint = endpoint_create(SERVER, 1, 1, 4).unwrap();
    let connection = connection_delegate(SERVER, endpoint, CLIENT, ConnectionRights::ALL).unwrap();
    let id = receive_endpoint_id(&IPC.read(), SERVER, endpoint).unwrap();
    let source = Source::Endpoint(id);
    let (identity, ready) = source.capture().unwrap();
    assert!(!ready);
    let mut prepared = sponsor.prepare(&identity, Arc::downgrade(&observer)).unwrap();
    let token;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        token = source.publish(&mut prepared).unwrap();
    }
    scalar_send(CLIENT, connection, 1, 2).unwrap();
    assert_eq!(sponsor.used(), 0);
    drop((token, prepared));
    receive(SERVER, endpoint).unwrap();

    let mut full_tokens = Vec::new();
    for _ in 0..waiter_budget::SOURCE_LIMIT {
        full_tokens.push(source.register(Arc::downgrade(&observer), &sponsor).unwrap());
    }
    let mut full = sponsor.prepare(&identity, Arc::downgrade(&observer)).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(source.publish(&mut full).unwrap_err(), RegistrationError::ResourceLimit);
    }
    assert_eq!(sponsor.used(), waiter_budget::SOURCE_LIMIT + 1);
    drop(full);
    assert_eq!(sponsor.used(), waiter_budget::SOURCE_LIMIT);
    drop(full_tokens);

    let mut readable = sponsor.prepare(&identity, Arc::downgrade(&observer)).unwrap();
    scalar_send(CLIENT, connection, 3, 4).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(source.publish(&mut readable).unwrap().is_ready());
    }
    assert_eq!(sponsor.used(), 1);
    assert_eq!(identity.registered(), 0);
    drop(readable);
    receive(SERVER, endpoint).unwrap();

    let call = scalar_call(CLIENT, connection, 5, 6).unwrap();
    let call_id = pending_call_id(CLIENT, call).unwrap();
    let reply = Source::Reply(call_id);
    let (reply_identity, ready) = reply.capture().unwrap();
    assert!(!ready);
    let mut prepared = sponsor.prepare(&reply_identity, Arc::downgrade(&observer)).unwrap();
    let token;
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        token = reply.publish(&mut prepared).unwrap();
    }
    let authority = receive(SERVER, endpoint).unwrap().reply.unwrap();
    super::super::reply(SERVER, authority, 42).unwrap();
    assert_eq!(sponsor.used(), 0);
    assert!(!IPC.read().pending_calls[&call_id].observed);
    drop((token, prepared));

    // Reply result wins before entry publication. The complete unused entry
    // remains owned/charged until the caller leaves IPC and disposes it.
    let mut replied = sponsor.prepare(&reply_identity, Arc::downgrade(&observer)).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(reply.publish(&mut replied).unwrap().is_ready());
    }
    assert!(!IPC.read().pending_calls[&call_id].observed);
    assert_eq!(sponsor.used(), 1);
    drop(replied);
    assert_eq!(poll_reply(CLIENT, call).unwrap().unwrap().result, 42);
    close_cap(CLIENT, call).unwrap();

    // A preparation for a different original list cannot attach to a live
    // endpoint even when that endpoint is readable. Identity precedes ready.
    let mut foreign = sponsor.prepare(&reply_identity, Arc::downgrade(&observer)).unwrap();
    scalar_send(CLIENT, connection, 7, 8).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(source.publish(&mut foreign).unwrap_err(), RegistrationError::Closed);
    }
    assert_eq!(identity.registered(), 0);
    drop(foreign);
    receive(SERVER, endpoint).unwrap();

    let mut gone = sponsor.prepare(&identity, Arc::downgrade(&observer)).unwrap();
    close_cap(SERVER, endpoint).unwrap();
    close_cap(CLIENT, connection).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(source.publish(&mut gone).unwrap_err(), RegistrationError::Closed);
    }
    assert_eq!(sponsor.used(), 1);
    drop(gone);
    assert_eq!(sponsor.used(), 0);
    close_address_space_fixture(SERVER).unwrap();
    close_address_space_fixture(CLIENT).unwrap();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[IPC waiter preparation] heap-held receive/reply/full/ready/foreign/closed-source \
         publication and post-guard original-charge disposal passed"
    );
}
