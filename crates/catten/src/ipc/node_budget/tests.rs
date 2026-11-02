//! Serialized real-layout controls; no payload/root/authority is leaked here.
use super::*;
use crate::{
    klib::admission::Reservation,
    memory::{
        self,
        allocators::global_allocator::PRIMARY_ALLOCATOR,
    },
};

fn rejected<T>() {
    let before = snapshot();
    let bytes = Storage::<T>::allocation_bytes() as u64;
    for platform in [false, true] {
        for _ in 0..64 {
            assert!(
                prepare_with::<T>(platform, |_| {
                    assert!(IPC.try_write().is_some());
                    assert_eq!(snapshot().0, [before.0[0] + bytes, before.0[1] + 1]);
                    if platform {
                        assert_eq!(snapshot().1, before.1);
                    } else {
                        assert_eq!(snapshot().1, [before.1[0] + bytes, before.1[1] + 1]);
                    }
                    Err(core::alloc::AllocError)
                })
                .is_err()
            );
            assert_eq!(snapshot(), before);
        }
    }
}

fn real_pressure<T>() {
    let policy = crate::resource_policy::POLICY.ipc_nodes;
    let before = snapshot();
    let bytes = Storage::<T>::allocation_bytes() as u64;
    let mut ordinary = Vec::with_capacity(policy.ordinary[1] as usize);
    while let Ok(node) = prepare::<T>(false) {
        ordinary.push(node);
    }
    let full = snapshot();
    assert!(full.1[1] == policy.ordinary[1] || policy.ordinary[0] - full.1[0] < bytes);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(prepare::<T>(false).is_err());
    }
    let trusted = prepare::<T>(true).unwrap();
    assert_eq!(snapshot().1, full.1);
    drop(trusted);
    let mut trusted = Vec::with_capacity(policy.total[1] as usize);
    while let Ok(node) = prepare::<T>(true) {
        trusted.push(node);
    }
    let total = snapshot();
    assert!(total.0[1] == policy.total[1] || policy.total[0] - total.0[0] < bytes);
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(prepare::<T>(true).is_err());
        assert!(prepare::<T>(false).is_err());
    }
    let counts = (ordinary.len(), trusted.len());
    drop((ordinary, trusted));
    assert_eq!(snapshot(), before);
    // Independently saturate just bytes, using an explicit counter adapter.
    let pressure =
        Reservation::<IpcNodes, 2>::reserve(false, [policy.ordinary[0] - before.1[0], 0]).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(prepare::<T>(false).is_err());
    }
    let node = prepare::<T>(true).unwrap();
    drop((node, pressure));
    let pressure =
        Reservation::<IpcNodes, 2>::reserve(true, [policy.total[0] - before.0[0], 0]).unwrap();
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert!(prepare::<T>(true).is_err());
    }
    drop(pressure);
    assert_eq!(snapshot(), before);
    crate::logln!(
        "[IPC node pressure] {} bytes; {} ordinary + {} trusted real empty nodes; heap-held quota \
         rejection, separate trusted reserve, independent counter-only byte exhaustion and exact \
         recovery",
        bytes,
        counts.0,
        counts.1
    );
}

fn second_node_rejection() {
    let root = crate::service::loader::create_user_address_space_handle();
    let before = snapshot();
    let records = record_budget::node_used();
    let shared = crate::capability::admission_tests::test_namespace_used(root.id());
    let bytes = PendingStorage::allocation_bytes() as u64;
    for _ in 0..64 {
        let pressure = Reservation::<IpcNodes, 2>::reserve(
            false,
            [crate::resource_policy::POLICY.ipc_nodes.ordinary[0] - before.1[0] - bytes, 0],
        )
        .unwrap();
        assert!(matches!(
            PreparedCall::prepare(root.id(), prepare, |platform| {
                assert!(!platform);
                assert_eq!(snapshot().1[1], before.1[1] + 1);
                let _heap = PRIMARY_ALLOCATOR.lock();
                prepare(platform)
            }),
            Err(IpcError::ResourceLimit)
        ));
        drop(pressure);
        assert_eq!(snapshot(), before);
        assert_eq!(record_budget::node_used(), records);
        assert_eq!(crate::capability::admission_tests::test_namespace_used(root.id()), shared);
    }
    memory::close_user_address_space_handle(root).unwrap();
    crate::logln!(
        "[IPC node rollback] 64 admitted pending-prefix / rejected reply-node preparations; \
         original resource counts and backing bytes/slots recovered before authority or \
         attachment publication"
    );
}

pub(in crate::ipc) fn run() {
    let irq = crate::cpu::isa::lp::ops::get_int_state();
    rejected::<(PendingCallId, PendingCall)>();
    rejected::<(ReplyTokenId, ReplyToken)>();
    real_pressure::<(PendingCallId, PendingCall)>();
    real_pressure::<(ReplyTokenId, ReplyToken)>();
    second_node_rejection();
    assert_eq!(crate::cpu::isa::lp::ops::get_int_state(), irq);
    crate::logln!(
        "[IPC node admission] 256 allocator rejections; actual pending/reply layouts {}/{} bytes; \
         complete node keeps backing and logical record reservations through real disposal",
        PendingStorage::allocation_bytes(),
        ReplyStorage::allocation_bytes()
    );
}
