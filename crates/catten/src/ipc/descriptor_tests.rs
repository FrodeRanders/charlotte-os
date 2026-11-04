//! Kernel ABI fixtures inspect byte admission without adopting userspace owners.
use super::*;
use crate::{
    klib::admission::{
        ByteAllocator,
        IpcNodes,
        Reservation,
    },
    memory::{
        self,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
        object,
    },
};

fn descriptor(caller: AddressSpaceId, source: MemoryObjectCap) -> MemoryObjectCap {
    let cap = object::allocate(caller, 1).unwrap();
    let mut bytes = [0; 18];
    bytes[..2].copy_from_slice(&1u16.to_le_bytes());
    bytes[2..10].copy_from_slice(&source.to_le_bytes());
    object::write_bytes(caller, cap, &bytes).unwrap();
    cap
}

pub(super) fn run() {
    let before = node_budget::snapshot();
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    let caller = crate::service::loader::create_user_address_space_handle();
    let server = crate::service::loader::create_user_address_space_handle();
    let endpoint = endpoint_create(server.id(), 1, 1, 4).unwrap();
    let connection =
        connection_delegate(server.id(), endpoint, caller.id(), ConnectionRights::ALL).unwrap();
    let source = object::allocate(caller.id(), 1).unwrap();
    object::write_bytes(caller.id(), source, &[0x5a]).unwrap();
    let page = descriptor(caller.id(), source);
    let baseline = node_budget::snapshot();
    let frames = memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let policy = crate::resource_policy::POLICY.ipc_nodes;
    let bytes = CAP_VECTOR_BYTES as u64;
    assert_eq!(bytes, 4082);
    let mut caps = buffer(false);
    let mut transfers = buffer(false);
    for platform in [false, true] {
        let available = if platform {
            policy.total[0] - baseline.0[0]
        } else {
            policy.ordinary[0] - baseline.1[0]
        };
        let pressure =
            Reservation::<IpcNodes, 2>::reserve(platform, [available - bytes + 1, 0]).unwrap();
        let full = node_budget::snapshot();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            assert_eq!(
                read_vector_page(
                    caller.id(),
                    page,
                    server.id(),
                    false,
                    platform,
                    &mut caps,
                    &mut transfers
                ),
                Err(IpcError::ResourceLimit)
            );
            for len in [0, 4097] {
                assert_eq!(
                    object::snapshot_bytes_in(
                        caller.id(),
                        page,
                        len,
                        ByteAllocator::<IpcNodes>::new(platform)
                    ),
                    Err(object::MemoryObjectError::InvalidLength)
                );
            }
            assert_eq!(
                read_vector_page(
                    caller.id(),
                    u64::MAX,
                    server.id(),
                    false,
                    platform,
                    &mut caps,
                    &mut transfers
                ),
                Err(IpcError::UnknownCapability)
            );
        }
        assert_eq!(node_budget::snapshot(), full);
        assert_eq!((caps.capacity(), transfers.capacity()), (0, 0));
        if !platform {
            // Public rejection keeps the descriptor/source and releases both
            // submission roots; no transfer or message has been published.
            assert_eq!(
                vector_send(caller.id(), connection, 1, 0, page),
                Err(IpcError::ResourceLimit)
            );
            assert_eq!(node_budget::snapshot(), full);
            assert_eq!(endpoint_status(server.id(), endpoint).unwrap().1, 0);
            assert!(!object::info(caller.id(), source).unwrap().lent);
            assert_eq!(object::snapshot_bytes(caller.id(), page, 2).unwrap(), [1, 0]);
            assert_eq!(memory::PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), frames);
            let trusted = object::snapshot_bytes_in(
                caller.id(),
                page,
                CAP_VECTOR_BYTES,
                ByteAllocator::<IpcNodes>::new(true),
            )
            .unwrap();
            assert_eq!(node_budget::snapshot(), ([full.0[0] + bytes, full.0[1]], full.1));
            drop(trusted);
            assert_eq!(node_budget::snapshot(), full);
        }
        drop(pressure);
        assert_eq!(node_budget::snapshot(), baseline);
    }
    object::write_bytes(caller.id(), page, &[0, 0]).unwrap();
    assert_eq!(
        read_vector_page(caller.id(), page, server.id(), false, false, &mut caps, &mut transfers),
        Err(IpcError::MemoryTransferFailed)
    );
    assert_eq!(node_budget::snapshot(), baseline);
    object::write_bytes(caller.id(), page, &[1, 0]).unwrap();
    // Retry the original public operation after real quota recovery.
    vector_send(caller.id(), connection, 1, 0, page).unwrap();
    assert_eq!(object::info(caller.id(), page), Err(object::MemoryObjectError::UnknownCapability));
    let message = receive(server.id(), endpoint).unwrap();
    assert!(message.reply.is_none());
    let copied = message.memory.unwrap();
    assert_eq!(object::snapshot_bytes(server.id(), copied, 1).unwrap(), [0x5a]);
    object::close_cap(server.id(), copied).unwrap();
    assert_eq!(node_budget::snapshot(), baseline);
    let page = descriptor(caller.id(), source);
    let snapshot = object::snapshot_bytes_in(
        caller.id(),
        page,
        CAP_VECTOR_BYTES,
        ByteAllocator::<IpcNodes>::new(false),
    )
    .unwrap();
    assert_eq!((snapshot.len(), snapshot.capacity()), (CAP_VECTOR_BYTES, CAP_VECTOR_BYTES));
    assert_eq!(
        node_budget::snapshot(),
        ([baseline.0[0] + bytes, baseline.0[1]], [baseline.1[0] + bytes, baseline.1[1]])
    );
    object::write_bytes(caller.id(), page, &[0, 0]).unwrap();
    assert_eq!(&snapshot[..2], &[1, 0]);
    assert_eq!(&snapshot[2..10], &source.to_le_bytes());
    memory::budget::mark_platform(caller);
    assert!(!snapshot.allocator().platform());
    object::close_cap(caller.id(), page).unwrap();
    object::close_cap(caller.id(), source).unwrap();
    close_cap(caller.id(), connection).unwrap();
    close_cap(server.id(), endpoint).unwrap();
    memory::close_user_address_space_handle(caller).unwrap();
    memory::close_user_address_space_handle(server).unwrap();
    assert_eq!(
        node_budget::snapshot(),
        ([before.0[0] + bytes, before.0[1]], [before.1[0] + bytes, before.1[1]])
    );
    drop(snapshot);
    assert_eq!(node_budget::snapshot(), before);
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[IPC descriptor capacity] 4082 requested bytes; ordinary/total quota denial under heap \
         guard, trusted headroom, invalid source/length and malformed rollback; public send \
         rejection preserves source/descriptor/frames, retry delivers; immutable copied bytes and \
         original class survive promotion/root retirement until actual buffer disposal"
    );
}
