//! Real allocated capacity, growth rollback and published loan ownership.
use super::*;
use crate::{
    klib::admission::{
        IpcNodes,
        Reservation,
    },
    memory::{
        self,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};
pub(super) fn run() {
    let before = node_budget::snapshot();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let policy = crate::resource_policy::POLICY.ipc_nodes;
    let mut ordinary = buffer::<MemoryObjectCap>(false);
    let n = ((policy.ordinary[0] - before.1[0]) / 8) as usize;
    ordinary.try_reserve_exact(n).unwrap();
    ordinary.resize(n, 0);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(ordinary.try_reserve_exact(1).is_err());
    }
    let full = node_budget::snapshot();
    assert_eq!(full.1[1], before.1[1]);
    let mut trusted = buffer::<MemoryObjectCap>(true);
    let n = ((policy.total[0] - full.0[0]) / 8) as usize;
    trusted.try_reserve_exact(n).unwrap();
    trusted.resize(n, 0);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(ordinary.try_reserve_exact(1).is_err());
        assert!(trusted.try_reserve_exact(1).is_err());
    }
    drop((ordinary, trusted));
    assert_eq!(node_budget::snapshot(), before);
    let mut borrows = buffer(false);
    let item = MemoryBorrow {
        owner: 0,
        owner_cap: 0,
        borrower: 0,
        borrower_cap: 0,
    };
    borrows.try_reserve_exact(1).unwrap();
    borrows.push(item);
    let pointer = borrows.as_ptr();
    let pressure = Reservation::<IpcNodes, 2>::reserve(
        false,
        [policy.ordinary[0] - node_budget::snapshot().1[0] - 32, 0],
    )
    .unwrap();
    let full = node_budget::snapshot();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(borrows.try_reserve_exact(1).is_err());
    }
    assert_eq!(borrows.as_ptr(), pointer);
    assert_eq!(borrows.as_slice(), [item]);
    assert_eq!(node_budget::snapshot(), full);
    assert_eq!(borrows.pop(), Some(item));
    assert_eq!(node_budget::snapshot(), full);
    drop(pressure);
    drop(borrows);
    assert_eq!(node_budget::snapshot(), before);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[IPC vector capacity] real ordinary/total byte pressure, heap-held growth denial, full \
         old/candidate rejection, original pointer/prefix and popped capacity retained; exact \
         recovery; cap={} transfer={} loan={} allocator={} bytes",
        core::mem::size_of::<MemoryObjectCap>(),
        core::mem::size_of::<memory::object::PreparedTransfer>(),
        core::mem::size_of::<MemoryBorrow>(),
        core::mem::size_of::<crate::klib::admission::ByteAllocator<IpcNodes>>()
    );
    loan_lifetime();
    rejected_attachment();
}
fn loan_lifetime() {
    let before = node_budget::snapshot();
    let caller = crate::service::loader::create_user_address_space_handle();
    let server = crate::service::loader::create_user_address_space_handle();
    let endpoint = endpoint_create(server.id(), 1, 1, 4).unwrap();
    let connection =
        connection_delegate(server.id(), endpoint, caller.id(), ConnectionRights::ALL).unwrap();
    let source = memory::object::allocate(caller.id(), 1).unwrap();
    let baseline = node_budget::snapshot();
    let call = scalar_call_with_memory_borrow_read(caller.id(), connection, 1, 0, source).unwrap();
    let queued = node_budget::snapshot();
    memory::budget::mark_platform(caller);
    assert_eq!(node_budget::snapshot(), queued);
    let message = receive(server.id(), endpoint).unwrap();
    let loan = message.memory.unwrap();
    {
        let ipc = IPC.read();
        let token = match ipc.cap(server.id(), message.reply.unwrap()).unwrap() {
            Capability::ReplyToken {
                token,
            } => token,
            _ => unreachable!(),
        };
        assert_eq!(ipc.reply_tokens[&token].borrows.capacity(), 1);
        assert!(!ipc.reply_tokens[&token].borrows.allocator().platform());
    }
    memory::object::map_any(server.id(), loan, false).unwrap();
    assert!(memory::object::info(caller.id(), source).unwrap().lent);
    reply(server.id(), message.reply.unwrap(), 7).unwrap();
    assert!(!memory::object::info(caller.id(), source).unwrap().lent);
    let pending_bytes = (node_budget::Storage::<(PendingCallId, PendingCall)>::allocation_bytes()
        + node_budget::Storage::<(CapabilityId, AdmittedCapability)>::allocation_bytes())
        as u64;
    assert_eq!(
        node_budget::snapshot(),
        (
            [baseline.0[0] + pending_bytes, baseline.0[1] + 2],
            [baseline.1[0] + pending_bytes, baseline.1[1] + 2]
        )
    );
    close_cap(caller.id(), call).unwrap();
    assert_eq!(node_budget::snapshot(), baseline);
    memory::object::close_cap(caller.id(), source).unwrap();
    close_cap(caller.id(), connection).unwrap();
    close_cap(server.id(), endpoint).unwrap();
    memory::close_user_address_space_handle(caller).unwrap();
    memory::close_user_address_space_handle(server).unwrap();
    assert_eq!(node_budget::snapshot(), before);
    crate::logln!(
        "[IPC loan capacity] real mapped read loan, caller promotion, receive/reply revocation \
         and call close recover exact backing; token keeps original loan-vector allocation until \
         actual disposal"
    );
}

fn rejected_attachment() {
    let before = node_budget::snapshot();
    let caller = crate::service::loader::create_user_address_space_handle();
    let server = crate::service::loader::create_user_address_space_handle();
    let source = memory::object::allocate(caller.id(), 1).unwrap();
    let physical = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let mut call = PreparedCall::try_new(caller.id()).unwrap();
    let transfer = memory::object::prepare_copy(caller.id(), source, server.id()).unwrap();
    let size = core::mem::size_of::<memory::object::PreparedTransfer>() as u64;
    let pressure = Reservation::<IpcNodes, 2>::reserve(
        false,
        [
            crate::resource_policy::POLICY.ipc_nodes.ordinary[0]
                - node_budget::snapshot().1[0]
                - size,
            0,
        ],
    )
    .unwrap();
    assert_eq!(call.attach(transfer), Err(IpcError::ResourceLimit));
    assert_eq!(call.transfers.capacity(), 1);
    assert!(call.transfers.is_empty());
    drop(pressure);
    call.finish();
    assert_eq!(node_budget::snapshot(), before);
    assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), physical);
    memory::object::write_bytes(caller.id(), source, &[7]).unwrap();
    memory::object::close_cap(caller.id(), source).unwrap();
    memory::close_user_address_space_handle(caller).unwrap();
    memory::close_user_address_space_handle(server).unwrap();
    assert_eq!(node_budget::snapshot(), before);
    crate::logln!(
        "[IPC vector rollback] transfer capacity admitted before cap-ID quota rejection; complete \
         copied transfer retained until post-guard finish; original counts, authority, buffer \
         bytes, source and physical frames recovered"
    );
}
