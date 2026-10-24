//! Serialized endpoint/queue ownership controls before AP schedulers start.
use super::*;
use crate::memory::{
    self,
    ADDRESS_SPACE_LIFECYCLE,
    ADDRESS_SPACE_TABLE,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn create_root() -> AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}
fn domain(root: AddressSpaceHandle) -> Arc<budget::DomainBudget> {
    IPC.read().caps[&root.id()].endpoint_budget.clone()
}
fn available(root: AddressSpaceHandle) {
    assert!(IPC.try_write().is_some());
    assert!(ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
    assert!(ADDRESS_SPACE_TABLE.try_lock().is_some());
    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
}

pub(super) fn run() {
    preparation();
    publication();
    growth();
    hidden_references();
    abandonment();
    crate::logln!(
        "[IPC endpoint storage] 640 preparation and 96 growth rejections; 64 complete heap-held \
         endpoint publications plus 64 heap-held publications/detaches, heap-held FIFO \
         replacement, concurrent resize/close, hidden connection final-reference cleanup and \
         guarded retention passed; two roots/leases, three endpoint records and twenty slots \
         deliberately retained"
    );
}

fn preparation() {
    let root = create_root();
    let account = domain(root);
    let baseline = budget::node_used();
    let caps = crate::capability::admission_tests::test_namespace_used(root.id());
    for failure in 1..=10 {
        for _ in 0..64 {
            assert!(
                PreparingEndpoint::prepare(root.id(), 1, |stage| {
                    available(root);
                    if stage == failure {
                        Err(IpcError::ResourceLimit)
                    } else {
                        Ok(())
                    }
                })
                .is_err()
            );
            assert_eq!(account.used(), [0, 0]);
            assert_eq!(budget::node_used(), baseline);
            assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), caps);
        }
    }
    let mut local = IpcRegistry::new();
    for id in 1..=64 {
        let mut prepared = PreparingEndpoint::prepare(root.id(), 1, |_| Ok(())).unwrap();
        let storage = prepared.0.storage.take().unwrap();
        let mut payload = prepared.take_payload(1, 1, 1);
        payload.closed = true;
        let mut retired = RetiredMetadata::new();
        {
            let _ipc = IPC.write();
            let _heap = PRIMARY_ALLOCATOR.lock();
            local.endpoints.insert(storage, id, payload);
            retired.push(local.detach_endpoint(id).unwrap());
        }
        assert_eq!(account.used(), [1, 4]);
        retired.release();
        prepared.finish();
        assert_eq!(account.used(), [0, 0]);
        assert_eq!(budget::node_used(), baseline);
    }
    let mut prepared = PreparingEndpoint::prepare(root.id(), 1, |_| Ok(())).unwrap();
    crate::capability::retire_address_space(root.id());
    assert_eq!(prepared.publish(&mut IPC.write(), 1, 1, 1), Err(IpcError::PermissionDenied));
    prepared.finish();
    assert_eq!(account.used(), [0, 0]);
    assert_eq!(budget::node_used(), baseline);
    memory::close_user_address_space_handle(root).unwrap();
}

fn publication() {
    let root = create_root();
    let account = domain(root);
    let baseline = budget::node_used();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        let mut owner = PreparingEndpoint::prepare(root.id(), 1, |_| {
            available(root);
            Ok(())
        })
        .unwrap();
        let cap = {
            let mut ipc = IPC.write();
            let _heap = PRIMARY_ALLOCATOR.lock();
            owner.publish(&mut ipc, 1, 1, 1).unwrap()
        };
        owner.finish();
        close_cap(root.id(), cap).unwrap();
        assert_eq!(account.used(), [0, 0]);
        assert_eq!(budget::node_used(), baseline);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    assert!(matches!(
        PreparingEndpoint::prepare(root.id(), 1, |stage| {
            available(root);
            if stage == 8 {
                crate::capability::retire_address_space(root.id());
            }
            Ok(())
        }),
        Err(IpcError::PermissionDenied)
    ));
    assert_eq!(account.used(), [0, 0]);
    assert_eq!(budget::node_used(), baseline);
    memory::close_user_address_space_handle(root).unwrap();
}

fn growth() {
    let root = create_root();
    let account = domain(root);
    let endpoint = create(root.id(), 1, 1, 4).unwrap();
    let connection = connection_mint(root.id(), endpoint, ConnectionRights::SEND).unwrap();
    scalar_send(root.id(), connection, 7, 8).unwrap();
    let baseline = budget::node_used();
    for failure in 1..=3 {
        for _ in 0..32 {
            let mut owner = Growth::capture(root.id()).unwrap();
            assert_eq!(
                owner.apply(endpoint, 8, |stage| {
                    available(root);
                    if stage == failure {
                        Err(IpcError::ResourceLimit)
                    } else {
                        Ok(())
                    }
                }),
                Err(IpcError::ResourceLimit)
            );
            owner.finish();
            assert_eq!(account.used(), [1, 4]);
            assert_eq!(budget::node_used(), baseline);
            assert_eq!(endpoint_status(root.id(), endpoint), Ok((4, 1, 1)));
        }
    }
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    memory::budget::mark_platform(root);
    assert_eq!(budget::node_used(), baseline);
    let mut owner = Growth::capture(root.id()).unwrap();
    let mut heap = None;
    owner
        .apply(endpoint, 8, |stage| {
            if stage == 3 {
                heap = Some(PRIMARY_ALLOCATOR.lock());
            }
            Ok(())
        })
        .unwrap();
    drop(heap);
    assert_eq!(
        budget::node_used().1[1],
        baseline.1[1] + 8,
        "queue growth reclassified the endpoint's original ordinary sponsor"
    );
    assert_eq!(account.used(), [1, 12], "old queue freed before explicit post-guard finish");
    owner.finish();
    assert_eq!(account.used(), [1, 8]);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    let message = receive(root.id(), endpoint).unwrap();
    assert_eq!((message.opcode, message.arg0), (7, 8));
    let mut owner = Growth::capture(root.id()).unwrap();
    owner
        .apply(endpoint, 16, |stage| {
            if stage == 3 {
                resize(root.id(), endpoint, 32).unwrap();
            }
            Ok(())
        })
        .unwrap();
    owner.finish();
    assert_eq!(endpoint_status(root.id(), endpoint), Ok((16, 0, 1)));
    assert_eq!(account.used(), [1, 32]);
    let mut owner = Growth::capture(root.id()).unwrap();
    assert_eq!(
        owner.apply(endpoint, 64, |stage| {
            if stage == 3 {
                close_cap(root.id(), endpoint).unwrap();
            }
            Ok(())
        }),
        Err(IpcError::UnknownCapability)
    );
    owner.finish();
    close_cap(root.id(), connection).unwrap();
    assert_eq!(account.used(), [0, 0]);
    memory::close_user_address_space_handle(root).unwrap();
}

fn hidden_references() {
    let caller = create_root();
    let server = create_root();
    let caller_budget = domain(caller);
    let server_budget = domain(server);
    let service = create(server.id(), 1, 1, 4).unwrap();
    let connection =
        connection_delegate(server.id(), service, caller.id(), ConnectionRights::CALL).unwrap();
    let source = create(caller.id(), 2, 1, 4).unwrap();
    let call =
        scalar_call_with_connection(caller.id(), connection, 1, 0, source, ConnectionRights::SEND)
            .unwrap();
    close_cap(caller.id(), source).unwrap();
    assert_eq!(caller_budget.used(), [1, 0]);
    close_cap(caller.id(), call).unwrap();
    assert_eq!(caller_budget.used(), [0, 0]);
    let source = create(server.id(), 2, 1, 4).unwrap();
    let call = scalar_call(caller.id(), connection, 1, 0).unwrap();
    let request = receive(server.id(), service).unwrap();
    reply_with_connection(server.id(), request.reply.unwrap(), source, ConnectionRights::SEND, 42)
        .unwrap();
    close_cap(server.id(), source).unwrap();
    assert_eq!(server_budget.used(), [2, 4]);
    close_cap(caller.id(), call).unwrap();
    assert_eq!(server_budget.used(), [1, 4]);
    memory::close_user_address_space_handle(caller).unwrap();
    memory::close_user_address_space_handle(server).unwrap();
}

fn abandonment() {
    let root = create_root();
    let account = domain(root);
    let prepared = PreparingEndpoint::prepare(root.id(), 1, |_| Ok(())).unwrap();
    {
        let _ipc = IPC.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(prepared);
    }
    assert_eq!(account.used(), [1, 4]);
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    let root = create_root();
    let account = domain(root);
    let endpoint = create(root.id(), 1, 1, 4).unwrap();
    let mut growth = Growth::capture(root.id()).unwrap();
    assert_eq!(
        growth.apply(endpoint, 8, |stage| {
            if stage == 3 {
                Err(IpcError::ResourceLimit)
            } else {
                Ok(())
            }
        }),
        Err(IpcError::ResourceLimit)
    );
    {
        let _ipc = IPC.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(growth);
    }
    assert_eq!(account.used(), [1, 12]);
    assert_eq!(
        memory::close_user_address_space_handle(root),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    let root = create_root();
    let account = domain(root);
    let mut prepared = PreparingEndpoint::prepare(root.id(), 1, |_| Ok(())).unwrap();
    let storage = prepared.0.storage.take().unwrap();
    let mut payload = prepared.take_payload(1, 1, 1);
    payload.closed = true;
    let mut local = IpcRegistry::new();
    local.endpoints.insert(storage, 1, payload);
    let mut retired = RetiredMetadata::new();
    retired.push(local.detach_endpoint(1).unwrap());
    {
        let _ipc = IPC.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(retired);
    }
    prepared.finish();
    memory::close_user_address_space_handle(root).unwrap();
    let fresh = create_root();
    assert_eq!(fresh.id(), root.id());
    assert_ne!(fresh, root);
    let endpoint = create(fresh.id(), 1, 1, 4).unwrap();
    close_cap(fresh.id(), endpoint).unwrap();
    assert_eq!(account.used(), [1, 4]);
    assert_eq!(domain(fresh).used(), [0, 0]);
    memory::close_user_address_space_handle(fresh).unwrap();
}
