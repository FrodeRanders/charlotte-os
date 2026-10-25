//! Reply nodes use the same count/owner lifetime as their pending-call peers.
use super::*;
use crate::memory::{
    self,
    ADDRESS_SPACE_LIFECYCLE,
    ADDRESS_SPACE_TABLE,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn token(server: AddressSpaceId, call: PendingCallId, platform: bool) -> ReplyToken {
    ReplyToken {
        server,
        call,
        platform,
        completing: false,
        claim_interest: None,
        claim_epoch: 0,
        cleanup_failed: false,
        connection_source: None,
        borrows: Vec::new(),
    }
}

pub(super) fn run() {
    second_allocation_rejection();
    paired_batch();
    original_classification();
    abandoned_node();
    crate::logln!(
        "[IPC reply storage] 64 second-allocation rejections, 64 heap-held paired \
         publications/detaches and one 64-node retirement batch passed; token charge survives \
         pending-node disposal, original class survives promotion; two abandoned scalar token \
         nodes/counts and one notification entry retained across exact root reuse"
    );
}

fn second_allocation_rejection() {
    let root = crate::service::loader::create_user_address_space_handle();
    let baseline = record_budget::node_used();
    let authority = crate::capability::admission_tests::test_namespace_used(root.id());
    for _ in 0..64 {
        assert!(
            PreparedCall::prepare(
                root.id(),
                || { PendingStorage::try_new_charged(None).map_err(|_| IpcError::ResourceLimit) },
                || {
                    assert!(IPC.try_write().is_some());
                    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
                    assert_eq!(
                        memory::close_user_address_space_handle(root),
                        Err(memory::AddressSpaceCloseError::OperationsInFlight)
                    );
                    assert_eq!(record_budget::node_used().0[1], baseline.0[1] + 1);
                    assert_eq!(record_budget::node_used().0[2], baseline.0[2] + 1);
                    Err(IpcError::ResourceLimit)
                }
            )
            .is_err()
        );
        assert_eq!(record_budget::node_used(), baseline);
        assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), authority);
    }
    memory::close_user_address_space_handle(root).unwrap();
}

fn paired_batch() {
    let root = crate::service::loader::create_user_address_space_handle();
    let baseline = record_budget::node_used();
    let authority = crate::capability::admission_tests::test_namespace_used(root.id());
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let mut owners = Vec::with_capacity(64);
    let mut pending = RetirementList::new();
    let mut replies = RetiredReplies::new();
    let mut local = IpcRegistry::new();
    for id in 1..=64 {
        let mut owner = PreparedCall::try_new(root.id()).unwrap();
        owner.stage(&mut IPC.write(), root.id()).unwrap();
        let call_node = owner.storage.take().unwrap();
        let call = owner.pending.take().unwrap();
        let reply_node = owner.reply_storage.take().unwrap();
        let payload = token(root.id(), id, owner.platform);
        {
            let _ipc = IPC.write();
            let _heap = PRIMARY_ALLOCATOR.lock();
            local.pending_calls.insert(call_node, id, call);
            local.reply_tokens.insert(reply_node, id, payload);
            pending.push(local.pending_calls.take(&id).unwrap());
            replies.push(local.reply_tokens.take(&id).unwrap());
        }
        owners.push(owner);
    }
    let full = record_budget::node_used();
    assert_eq!(full.0[1], baseline.0[1] + 64);
    assert_eq!(full.0[2], baseline.0[2] + 64);
    while let Some(node) = pending.pop() {
        node.release();
    }
    assert_eq!(record_budget::node_used().0[1], baseline.0[1]);
    assert_eq!(record_budget::node_used().0[2], full.0[2]);
    replies.release();
    assert_eq!(record_budget::node_used(), baseline);
    for owner in owners {
        owner.finish();
    }
    assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), authority);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    memory::close_user_address_space_handle(root).unwrap();
}

fn original_classification() {
    let root = crate::service::loader::create_user_address_space_handle();
    let baseline = record_budget::node_used();
    let endpoint = endpoint_create(root.id(), 1, 1, 4).unwrap();
    let connection = connection_mint(root.id(), endpoint, ConnectionRights::CALL).unwrap();
    let mut owner = PreparedCall::try_new(root.id()).unwrap();
    assert!(!owner.platform);
    let charged = record_budget::node_used();
    memory::budget::mark_platform(root);
    assert_eq!(record_budget::node_used(), charged);
    let (call, delivery) = {
        let mut ipc = IPC.write();
        owner.stage(&mut ipc, root.id()).unwrap();
        let endpoint_id = match ipc.cap(root.id(), connection).unwrap() {
            Capability::Connection {
                endpoint,
                ..
            } => endpoint,
            _ => unreachable!(),
        };
        let result = owner.commit(&mut ipc, endpoint_id, 1, 0).unwrap();
        let id = ipc.endpoints[&endpoint_id].queue.front().unwrap().reply.unwrap();
        assert!(!ipc.reply_tokens[&id].platform);
        result
    };
    owner.finish();
    delivery.notify();
    assert_eq!(record_budget::node_used(), charged);
    let reply_cap = receive(root.id(), endpoint).unwrap().reply.unwrap();
    reply(root.id(), reply_cap, 42).unwrap();
    let replied = record_budget::node_used();
    assert_eq!(replied.0[2], baseline.0[2]);
    assert_eq!(replied.1[2], baseline.1[2]);
    assert_eq!(poll_reply(root.id(), call).unwrap().unwrap().result, 42);
    close_cap(root.id(), call).unwrap();
    close_cap(root.id(), connection).unwrap();
    close_cap(root.id(), endpoint).unwrap();
    assert_eq!(record_budget::node_used(), baseline);
    memory::close_user_address_space_handle(root).unwrap();
}

fn abandoned_node() {
    let root = crate::service::loader::create_user_address_space_handle();
    let baseline = record_budget::node_used();
    let account = IPC.read().caps[&root.id()].record_budget.clone();
    let mut owner = PreparedCall::try_new(root.id()).unwrap();
    let node =
        owner.reply_storage.take().unwrap().publish((1, token(root.id(), 1, owner.platform)));
    let mut retired = RetiredReplies::new();
    retired.push(node);
    {
        let _ipc = IPC.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(retired);
    }
    owner.finish();
    let sponsor = WaitSponsor::new(false);
    let source = crate::klib::observer::waiter_source::WaiterSource::new();
    let observer: Arc<dyn Observer> = crate::klib::observer::CallOnNotify::new(|| {
        panic!("abandoned reply completion invoked its callback")
    });
    let registered = source.register(Arc::downgrade(&observer), &sponsor).unwrap();
    let mut owner = PreparedCall::try_new(root.id()).unwrap();
    let node =
        owner.reply_storage.take().unwrap().publish((2, token(root.id(), 2, owner.platform)));
    let completed = ReplyCompletion::new(node, source.drain());
    assert_eq!(sponsor.used(), 1);
    {
        let _ipc = IPC.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(completed);
    }
    owner.finish();
    assert_eq!(sponsor.used(), 1);
    drop(registered);
    assert_eq!(account.used(), [0, 0, 2]);
    memory::close_user_address_space_handle(root).unwrap();
    let fresh = crate::service::loader::create_user_address_space_handle();
    assert_eq!(fresh.id(), root.id());
    assert_ne!(fresh, root);
    let fresh_account = IPC.read().caps[&fresh.id()].record_budget.clone();
    PreparedCall::try_new(fresh.id()).unwrap().finish();
    assert_eq!(fresh_account.used(), [0, 0, 0]);
    assert_eq!(account.used(), [0, 0, 2]);
    memory::close_user_address_space_handle(fresh).unwrap();
    let after = record_budget::node_used();
    assert_eq!(after.0[2], baseline.0[2] + 2);
    assert_eq!(after.1[2], baseline.1[2] + 2);
}
