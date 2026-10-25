//! Complete scalar call commit with already-prepared storage and heap held.
use super::*;
use crate::memory::{
    self,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

pub(super) fn run() {
    let handle = crate::service::loader::create_user_address_space_handle();
    let endpoint = endpoint_create(handle.id(), 1, 1, 1).unwrap();
    let connection = connection_mint(handle.id(), endpoint, ConnectionRights::CALL).unwrap();
    let endpoint_id = match IPC.read().cap(handle.id(), connection).unwrap() {
        Capability::Connection {
            endpoint,
            ..
        } => endpoint,
        _ => unreachable!(),
    };
    let records = record_budget::node_used();
    let caps = crate::capability::admission_tests::test_namespace_used(handle.id());
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        let mut prepared = PreparedCall::try_new(handle.id()).unwrap();
        let (call, delivery) = {
            let mut ipc = IPC.write();
            let _heap = PRIMARY_ALLOCATOR.lock();
            prepared.stage(&mut ipc, handle.id()).unwrap();
            prepared.commit(&mut ipc, endpoint_id, 7, 8).unwrap()
        };
        prepared.finish();
        delivery.notify();
        let incoming = receive(handle.id(), endpoint).unwrap();
        assert_eq!((incoming.opcode, incoming.arg0), (7, 8));
        reply(handle.id(), incoming.reply.unwrap(), 42).unwrap();
        assert_eq!(poll_reply(handle.id(), call).unwrap().unwrap().result, 42);
        close_cap(handle.id(), call).unwrap();
        assert_eq!(record_budget::node_used(), records);
        assert_eq!(crate::capability::admission_tests::test_namespace_used(handle.id()), caps);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    close_cap(handle.id(), connection).unwrap();
    close_cap(handle.id(), endpoint).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
    crate::logln!(
        "[IPC scalar publication] 64 complete prepared call commits with IPC/heap held, \
         enqueue/reply/observation, original count reconciliation and IRQ preservation passed"
    );
}
