//! Serialized real retirement-capacity admission; empty pressure buffers own
//! no physical backing or authority. Public paths qualify complete IPC custody.
use super::*;
use crate::{
    ipc::node_budget,
    memory,
};

fn caps(platform: bool, n: usize) -> Result<Buffer<MemoryObjectCap>, IpcError> {
    let mut result = buffer(platform);
    result.try_reserve_exact(n).map_err(|_| IpcError::ResourceLimit)?;
    result.resize(n, 0);
    Ok(result)
}
fn buffers(platform: bool) -> Vec<MemoryAttachments> {
    let max_bytes = (CAP_VECTOR_MAX * core::mem::size_of::<RetiredMemory>()) as u64;
    let mut owners = Vec::with_capacity(
        (crate::resource_policy::POLICY.ipc_nodes.total[0] / max_bytes) as usize + CAP_VECTOR_MAX,
    );
    for n in [CAP_VECTOR_MAX, 1] {
        while let Ok(owner) = caps(platform, n).and_then(MemoryAttachments::try_new) {
            owners.push(owner);
        }
    }
    owners
}
pub(in crate::ipc) fn run() {
    let before = node_budget::snapshot();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    for _ in 0..64 {
        assert!(
            MemoryAttachments::try_new_with(caps(false, 3).unwrap(), |retired, n| {
                retired.try_reserve_exact(n).unwrap();
                Err(IpcError::ResourceLimit)
            })
            .is_err()
        );
        assert_eq!(node_budget::snapshot(), before);
    }
    let ordinary = buffers(false);
    let full = node_budget::snapshot();
    assert_eq!(full.1[1], before.1[1]);
    let trusted = MemoryAttachments::single(true, 0).unwrap();
    assert_eq!(node_budget::snapshot().1, full.1);
    trusted.finish();
    let trusted = buffers(true);
    let counts = (ordinary.len(), trusted.len());
    for owner in ordinary.into_iter().chain(trusted) {
        owner.finish();
    }
    assert_eq!(node_budget::snapshot(), before);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[IPC attachment capacity] ordinary={} trusted={} real empty buffers; byte-only \
         ordinary/total exhaustion, 64 allocated-prefix rejections and exact recovery; retired={} \
         max={} bytes; node slots unchanged",
        counts.0,
        counts.1,
        core::mem::size_of::<RetiredMemory>(),
        CAP_VECTOR_MAX * core::mem::size_of::<RetiredMemory>()
    );
    public_paths();
    super::super::message::test_received_storage_abandonment();
}

fn public_paths() {
    for outcome in 0..3 {
        let before = node_budget::snapshot();
        let caller = crate::service::loader::create_user_address_space_handle();
        let server = crate::service::loader::create_user_address_space_handle();
        let endpoint = endpoint_create(server.id(), 1, 1, 4).unwrap();
        let connection =
            connection_delegate(server.id(), endpoint, caller.id(), ConnectionRights::ALL).unwrap();
        let source = memory::object::allocate(caller.id(), 1).unwrap();
        let baseline = node_budget::snapshot();
        let physical = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
        let mut call = PreparedCall::try_new(caller.id()).unwrap();
        call.attach(memory::object::prepare_copy(caller.id(), source, server.id()).unwrap())
            .unwrap();
        let destination = call.memory[0];
        let staged = node_budget::snapshot();
        let transfer_bytes = (call.transfers.capacity()
            * core::mem::size_of::<memory::object::PreparedTransfer>())
            as u64;
        memory::budget::mark_platform(caller);
        let eid = match IPC.read().cap(caller.id(), connection).unwrap() {
            Capability::Connection {
                endpoint,
                ..
            } => endpoint,
            _ => unreachable!(),
        };
        if outcome == 0 {
            let pressure = Charge::reserve(
                false,
                [crate::resource_policy::POLICY.ipc_nodes.ordinary[0] - staged.1[0], 0],
            )
            .unwrap();
            assert!(matches!(
                call.commit(&mut IPC.write(), eid, 1, 0),
                Err(IpcError::ResourceLimit)
            ));
            drop(pressure);
            call.finish();
        } else {
            let (cap, delivery) = call.commit(&mut IPC.write(), eid, 1, 0).unwrap();
            call.finish();
            delivery.notify();
            let queued = node_budget::snapshot();
            let bytes = core::mem::size_of::<RetiredMemory>() as u64;
            assert_eq!(queued.1, [staged.1[0] - transfer_bytes + bytes, staged.1[1]]);
            if outcome == 2 {
                let message = receive(server.id(), endpoint).unwrap();
                let received = node_budget::snapshot();
                let payload =
                    node_budget::Storage::<(CapabilityId, AdmittedCapability)>::allocation_bytes()
                        as u64;
                assert_eq!(
                    received.1,
                    [
                        queued.1[0] - bytes - core::mem::size_of::<MemoryObjectCap>() as u64
                            + payload,
                        queued.1[1] + 1
                    ]
                );
                reply(server.id(), message.reply.unwrap(), 7).unwrap();
                close_cap(caller.id(), cap).unwrap();
                memory::object::close_cap(server.id(), destination).unwrap();
            } else {
                close_cap(caller.id(), cap).unwrap();
            }
        }
        assert_eq!(node_budget::snapshot(), baseline);
        assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), physical);
        memory::object::close_cap(caller.id(), source).unwrap();
        close_cap(caller.id(), connection).unwrap();
        close_cap(server.id(), endpoint).unwrap();
        memory::close_user_address_space_handle(caller).unwrap();
        memory::close_user_address_space_handle(server).unwrap();
        assert_eq!(node_budget::snapshot(), before);
    }
    crate::logln!(
        "[IPC attachment ownership] real copy call quota rollback, queued cancellation and \
         receive/reply/close recover backing, counts and physical frames; promotion retains \
         original ordinary buffer class; delivered storage disposed after IPC unlock"
    );
}
