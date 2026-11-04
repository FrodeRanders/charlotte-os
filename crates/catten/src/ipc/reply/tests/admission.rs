//! Real work allocations retain original token class and full capacity.
use super::*;
use crate::{
    ipc::loan_work_tests::bytes,
    klib::admission::{
        IpcNodes,
        Reservation,
    },
    memory::allocators::global_allocator::PRIMARY_ALLOCATOR,
};

pub(super) fn run() {
    for platform in [false, true] {
        let before = node_budget::snapshot();
        let fixture = Fixture::new_classified(2, platform);
        let baseline = node_budget::snapshot();
        // A later lookup of this caller would incorrectly grant trusted backing
        // to the ordinary token. Work must inherit the accepted token's class.
        memory::budget::mark_platform(fixture.caller);
        let policy = crate::resource_policy::POLICY.ipc_nodes;
        let available = if platform {
            policy.total[0] - baseline.0[0]
        } else {
            policy.ordinary[0] - baseline.1[0]
        };
        let pressure =
            Reservation::<IpcNodes, 2>::reserve(platform, [available - bytes(2) + 1, 0]).unwrap();
        let full = node_budget::snapshot();
        {
            let _heap = PRIMARY_ALLOCATOR.lock();
            assert!(matches!(
                PreparedReply::prepare(fixture.server.id(), fixture.reply),
                Err(IpcError::ResourceLimit)
            ));
        }
        assert_eq!(node_budget::snapshot(), full);
        {
            let ipc = IPC.read();
            let (token, _, _) = validate(&ipc, fixture.server.id(), fixture.reply).unwrap();
            assert_eq!(ipc.reply_tokens[&token].claim_epoch, 0);
            assert_eq!(ipc.reply_tokens[&token].borrows, fixture.borrows);
        }
        for borrow in &fixture.borrows {
            assert!(object::info(borrow.borrower, borrow.borrower_cap).unwrap().mapped);
        }
        drop(pressure);
        assert_eq!(node_budget::snapshot(), baseline);
        let operation = fixture.prepare();
        assert_eq!((operation.loans.len(), operation.loans.capacity()), (2, 2));
        assert_eq!(operation.loans.allocator().platform(), platform);
        let charged = (
            [baseline.0[0] + bytes(2), baseline.0[1]],
            [
                baseline.1[0]
                    + if platform {
                        0
                    } else {
                        bytes(2)
                    },
                baseline.1[1],
            ],
        );
        assert_eq!(node_budget::snapshot(), charged);
        let mut finished = 0;
        operation
            .finish_with_publication(
                7,
                |loan| {
                    unlocked();
                    assert_eq!(node_budget::snapshot(), charged);
                    finished += 1;
                    loan.finish_observed(unlocked)
                },
                |memory, authority| {
                    // Every work entry has been popped, but the allocation is live.
                    assert_eq!(node_budget::snapshot(), charged);
                    object::commit_undelivered_transfers_with_authority(memory, authority)
                        .map_err(|_| IpcError::MemoryTransferFailed)
                },
            )
            .unwrap();
        assert_eq!(finished, 2);
        assert_eq!(poll_reply(fixture.caller.id(), fixture.call).unwrap().unwrap().result, 7);
        fixture.close();
        assert_eq!(node_budget::snapshot(), before);
    }
    let before = node_budget::snapshot();
    let fixture = Fixture::new(0);
    let baseline = node_budget::snapshot();
    let policy = crate::resource_policy::POLICY.ipc_nodes;
    let pressure =
        Reservation::<IpcNodes, 2>::reserve(false, [policy.ordinary[0] - baseline.1[0], 0])
            .unwrap();
    let full = node_budget::snapshot();
    let operation = {
        let _heap = PRIMARY_ALLOCATOR.lock();
        fixture.prepare()
    };
    assert_eq!(operation.loans.capacity(), 0);
    assert_eq!(node_budget::snapshot(), full);
    drop(pressure);
    operation.finish_with(1, |_| panic!("empty reply revoked a loan")).unwrap();
    fixture.close();
    assert_eq!(node_budget::snapshot(), before);
    crate::logln!(
        "[IPC reply work] parent={} entry={} max={} bytes; ordinary/total quota denial under heap \
         guard before claim, original class after promotion, full capacity through final \
         pop/publication, zero-work admission at quota and exact recovery",
        core::mem::size_of::<PreparedReply>(),
        bytes(1),
        bytes(CAP_VECTOR_MAX)
    );
}
