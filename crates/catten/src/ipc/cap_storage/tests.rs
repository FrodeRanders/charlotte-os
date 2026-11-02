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
    backing_classification();
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

fn backing_classification() {
    use crate::klib::admission::{
        IpcNodes,
        Reservation,
    };
    let handle = root();
    let before = node_budget::snapshot();
    let authority = used(handle.id());
    for _ in 0..64 {
        let mut owner = PreparingCapability::new();
        owner.prepare_authority_storage(handle.id(), Some(handle)).unwrap();
        owner.reserve(&mut IPC.write()).unwrap();
        // Promotion cannot replace the namespace's captured backing class.
        memory::budget::mark_platform(handle);
        let pressure = Reservation::<IpcNodes, 2>::reserve(
            false,
            [crate::resource_policy::POLICY.ipc_nodes.ordinary[0] - before.1[0], 0],
        )
        .unwrap();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            assert_eq!(owner.prepare_payload(), Err(IpcError::ResourceLimit));
        }
        assert_eq!(used(handle.id()), authority + 1);
        drop(pressure);
        owner.prepare_payload().unwrap();
        let bytes = Storage::allocation_bytes() as u64;
        assert_eq!(
            node_budget::snapshot(),
            ([before.0[0] + bytes, before.0[1] + 1], [before.1[0] + bytes, before.1[1] + 1])
        );
        owner.finish();
        assert_eq!(node_budget::snapshot(), before);
        assert_eq!(used(handle.id()), authority);
    }
    memory::close_user_address_space_handle(handle).unwrap();

    // Recipient backing and grantor resource sponsorship are independent.
    let grantor = root();
    let endpoint = endpoint_create(grantor.id(), 1, 1, 1).unwrap();
    let before = node_budget::snapshot();
    let records = record_budget::node_used();
    let pressure = Reservation::<IpcNodes, 2>::reserve(
        false,
        [crate::resource_policy::POLICY.ipc_nodes.ordinary[0] - before.1[0], 0],
    )
    .unwrap();
    let full = node_budget::snapshot();
    let cap =
        connection_delegate(grantor.id(), endpoint, memory::KERNEL_ASID, ConnectionRights::SEND)
            .unwrap();
    assert_eq!(node_budget::snapshot().1, full.1);
    assert_eq!(
        node_budget::snapshot().0,
        [full.0[0] + Storage::allocation_bytes() as u64, full.0[1] + 1]
    );
    assert_eq!(record_budget::node_used().1[0], records.1[0] + 1);
    close_cap(memory::KERNEL_ASID, cap).unwrap();
    assert_eq!(node_budget::snapshot(), full);
    assert_eq!(record_budget::node_used(), records);
    drop(pressure);
    close_cap(grantor.id(), endpoint).unwrap();
    memory::close_user_address_space_handle(grantor).unwrap();
    crate::logln!(
        "[IPC capability backing] 64 heap-held quota denials after authority reservation, \
         namespace-captured class survives promotion and exact cleanup; ordinary grantor uses \
         trusted recipient backing reserve without acquiring trusted logical admission"
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
    assert!(matches!(
        PreparedCall::prepare_checked_with(
            handle.id(),
            None,
            node_budget::prepare,
            node_budget::prepare,
            |stage| {
                assert_eq!(
                    memory::close_user_address_space_handle(handle),
                    Err(memory::AddressSpaceCloseError::OperationsInFlight)
                );
                if stage == 3 {
                    Err(IpcError::ResourceLimit)
                } else {
                    Ok(())
                }
            }
        ),
        Err(IpcError::ResourceLimit)
    ));
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
    call.stage(&mut IPC.write(), handle.id()).unwrap();
    let records = IPC.read().caps[&handle.id()].record_budget.clone();
    {
        let _ipc = IPC.write();
        let _heap = PRIMARY_ALLOCATOR.lock();
        node_budget::with_locked(|| drop(call));
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
        node_budget::with_locked(|| drop(retired));
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
