//! Serialized publication/retirement controls before AP schedulers start.
use super::*;
use crate::memory::{
    self,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

fn root() -> memory::AddressSpaceHandle {
    crate::service::loader::create_user_address_space_handle()
}
fn used(owner: AddressSpaceId) -> usize {
    crate::capability::admission_tests::test_namespace_used(owner)
}

pub(super) fn run() {
    preparation();
    publication();
    abandonment();
    crate::logln!(
        "[IPC capability storage] 128 stage rejections, 64 heap-held joint publications/detaches, \
         post-guard payload/authority refunds, containing-call rejection and guarded retention \
         passed; one call root/lease with two family records and one staged capability, plus one \
         detached connection payload/authority pair deliberately retained"
    );
}

fn preparation() {
    let handle = root();
    let baseline = used(handle.id());
    for reject in 1..=2 {
        for _ in 0..64 {
            let mut prepared = PreparingCapability::new();
            let result = {
                let mut ipc = IPC.write();
                prepared.prepare_with(&mut ipc, handle.id(), |stage| {
                    assert!(PRIMARY_ALLOCATOR.try_lock().is_some());
                    assert!(memory::ADDRESS_SPACE_LIFECYCLE.try_lock().is_some());
                    assert!(memory::ADDRESS_SPACE_TABLE.try_lock().is_some());
                    assert_eq!(used(handle.id()), baseline + 1);
                    if stage == reject {
                        Err(IpcError::ResourceLimit)
                    } else {
                        Ok(())
                    }
                })
            };
            assert_eq!(result, Err(IpcError::ResourceLimit));
            assert_eq!(
                used(handle.id()),
                baseline + 1,
                "unused authority escaped its containing owner"
            );
            prepared.finish();
            assert_eq!(used(handle.id()), baseline);
        }
    }
    let records = IPC.read().caps[&handle.id()].record_budget.clone();
    let mut call = PreparedCall::try_new(handle.id()).unwrap();
    call.authority = Some(PreparingCapability::new());
    let result =
        call.authority
            .as_mut()
            .unwrap()
            .prepare_with(&mut IPC.write(), handle.id(), |_| Err(IpcError::ResourceLimit));
    assert_eq!(result, Err(IpcError::ResourceLimit));
    assert_eq!(records.used(), [0, 1, 1]);
    assert_eq!(
        memory::close_user_address_space_handle(handle),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );
    call.finish();
    assert_eq!(records.used(), [0, 0, 0]);
    assert_eq!(used(handle.id()), baseline);
    let mut prepared = PreparingCapability::new();
    {
        let mut ipc = IPC.write();
        prepared.prepare(&mut ipc, handle.id()).unwrap();
        crate::capability::retire_address_space(handle.id());
        assert_eq!(prepared.publish(), Err(IpcError::PermissionDenied));
        assert!(ipc.caps[&handle.id()].caps.first_key_value().is_none());
    }
    prepared.finish();
    assert_eq!(used(handle.id()), baseline);
    memory::close_user_address_space_handle(handle).unwrap();
}

fn publication() {
    let handle = root();
    let endpoint = endpoint_create(handle.id(), 1, 1, 1).unwrap();
    let (id, records) = {
        let ipc = IPC.read();
        let Capability::Endpoint {
            endpoint,
            ..
        } = ipc.cap(handle.id(), endpoint).unwrap()
        else {
            unreachable!()
        };
        (endpoint, ipc.caps[&handle.id()].record_budget.clone())
    };
    let baseline = used(handle.id());
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        let mut prepared = PreparedConnection::empty(handle.id(), id, ConnectionRights::SEND);
        prepared.prepare(&mut IPC.write(), handle.id()).unwrap();
        let mut retired = RetiredMetadata::new();
        {
            let mut ipc = IPC.write();
            let _heap = PRIMARY_ALLOCATOR.lock();
            prepared.authority.publish().unwrap();
            let cap = prepared.install(&mut ipc);
            assert!(ipc.cap(handle.id(), cap).is_err(), "hidden connection became visible");
            ipc.remove_cap(handle.id(), cap, &mut retired).unwrap();
        }
        assert_eq!(used(handle.id()), baseline + 1);
        assert_eq!(records.used(), [1, 0, 0]);
        prepared.finish();
        assert_eq!(used(handle.id()), baseline + 1);
        assert_eq!(records.used(), [1, 0, 0]);
        retired.release();
        assert_eq!(used(handle.id()), baseline);
        assert_eq!(records.used(), [0, 0, 0]);
        assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    }
    close_cap(handle.id(), endpoint).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
}

fn abandonment() {
    let handle = root();
    let mut call = PreparedCall::try_new(handle.id()).unwrap();
    IPC.write().stage_call(handle.id(), &mut call).unwrap();
    let records = IPC.read().caps[&handle.id()].record_budget.clone();
    {
        let _ipc = IPC.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(call);
    }
    assert_eq!(records.used(), [0, 1, 1]);
    assert_eq!(used(handle.id()), 1);
    assert_eq!(
        memory::close_user_address_space_handle(handle),
        Err(memory::AddressSpaceCloseError::OperationsInFlight)
    );

    let baseline = crate::capability::node_admission_used();
    let handle = root();
    let endpoint = endpoint_create(handle.id(), 1, 1, 1).unwrap();
    let cap = connection_mint(handle.id(), endpoint, ConnectionRights::SEND).unwrap();
    let records = IPC.read().caps[&handle.id()].record_budget.clone();
    let mut retired = RetiredMetadata::new();
    {
        let mut ipc = IPC.write();
        ipc.remove_cap(handle.id(), cap, &mut retired).unwrap();
        let _heap = PRIMARY_ALLOCATOR.lock();
        drop(retired);
    }
    close_cap(handle.id(), endpoint).unwrap();
    memory::close_user_address_space_handle(handle).unwrap();
    assert_eq!(records.used(), [1, 0, 0]);
    assert_eq!(crate::capability::node_admission_used(), (baseline.0 + 1, baseline.1 + 1));
    let fresh = root();
    assert_eq!(fresh.id(), handle.id());
    assert_ne!(fresh, handle);
    let endpoint = endpoint_create(fresh.id(), 1, 1, 1).unwrap();
    close_cap(fresh.id(), endpoint).unwrap();
    assert_eq!(records.used(), [1, 0, 0]);
    assert_eq!(IPC.read().caps[&fresh.id()].record_budget.used(), [0, 0, 0]);
    memory::close_user_address_space_handle(fresh).unwrap();
}
