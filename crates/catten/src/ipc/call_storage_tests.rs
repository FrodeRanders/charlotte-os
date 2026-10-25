//! Serialized qualification of pending-node storage and containing call owners.
use super::*;
use crate::memory::{
    self,
    ADDRESS_SPACE_LIFECYCLE,
    ADDRESS_SPACE_TABLE,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
    object,
};

pub(super) fn run() {
    let caller = crate::service::loader::create_user_address_space_handle();
    let baseline = record_budget::node_used();
    let authority = crate::capability::admission_tests::test_namespace_used(caller.id());
    for _ in 0..64 {
        assert!(matches!(
            PreparedCall::prepare_checked(
                caller.id(),
                Some(0),
                || { panic!("invalid call handle reached pending-node allocation") },
                || panic!("invalid call handle reached reply-node allocation")
            ),
            Err(IpcError::UnknownCapability)
        ));
        assert_eq!(record_budget::node_used(), baseline);
        assert_eq!(crate::capability::admission_tests::test_namespace_used(caller.id()), authority);
    }
    for _ in 0..64 {
        assert!(
            PreparedCall::prepare(
                caller.id(),
                || {
                    assert!(IPC.try_write().is_some());
                    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
                    Err(IpcError::ResourceLimit)
                },
                || panic!("rejected pending storage reached reply preparation")
            )
            .is_err()
        );
        assert_eq!(record_budget::node_used(), baseline);
        assert_eq!(crate::capability::admission_tests::test_namespace_used(caller.id()), authority);
    }
    let mut local = IpcRegistry::new();
    for _ in 0..64 {
        let irq = crate::cpu::isa::lp::ops::get_int_state();
        let mut call = PreparedCall::try_new(caller.id()).unwrap();
        call.stage(&mut IPC.write(), caller.id()).unwrap();
        let node = call.storage.take().unwrap();
        let payload = call.pending.take().unwrap();
        let charged = record_budget::node_used();
        assert_eq!(charged.0[1], baseline.0[1] + 1);
        let retired = {
            let _heap = PRIMARY_ALLOCATOR.lock();
            local.pending_calls.insert(node, 1, payload);
            let retired = local.pending_calls.take(&1).unwrap();
            assert_eq!(record_budget::node_used(), charged);
            retired
        };
        assert!(IPC.try_write().is_some());
        retired.release();
        assert_eq!(record_budget::node_used().0[1], baseline.0[1]);
        call.finish();
        assert_eq!(record_budget::node_used(), baseline);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }

    // Deliberate terminal fixtures: charged storage, admitted authority/waiters,
    // and copied backing all remain in the same containing owner on Drop.
    let empty = PreparedCall::try_new(caller.id()).unwrap();
    {
        let _ipc = IPC.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(empty);
    }
    for copied in [false, true] {
        let caller = crate::service::loader::create_user_address_space_handle();
        let server = crate::service::loader::create_user_address_space_handle();
        let mut call = PreparedCall::try_new(caller.id()).unwrap();
        call.stage(&mut IPC.write(), caller.id()).unwrap();
        if copied {
            let source = object::allocate(caller.id(), 1).unwrap();
            call.attach(object::prepare_copy(caller.id(), source, server.id()).unwrap()).unwrap();
        }
        let records = record_budget::node_used();
        let backing = memory::budget::used(caller);
        let capabilities = crate::capability::admission_tests::test_namespace_used(caller.id());
        {
            let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
            let _table = ADDRESS_SPACE_TABLE.lock();
            let _ipc = IPC.write();
            let _heap = PRIMARY_ALLOCATOR.lock();
            drop(call);
        }
        assert_eq!(record_budget::node_used(), records);
        assert_eq!(memory::budget::used(caller), backing);
        assert_eq!(
            crate::capability::admission_tests::test_namespace_used(caller.id()),
            capabilities
        );
    }
    crate::logln!(
        "[IPC pending storage] 64 invalid-handle and 64 preparation rejections, 64 heap-held \
         publications/detaches, original charge through node disposal and IRQ preservation \
         passed; three abandoned call owners retain storage, three call/reply reservations, five \
         roots and copied backing"
    );
}
