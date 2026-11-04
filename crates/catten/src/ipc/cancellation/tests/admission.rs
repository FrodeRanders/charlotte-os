//! Queued/delivered cleanup shares mechanics, retaining local physical owners.
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
    for delivered in [false, true] {
        for platform in [false, true] {
            let before = node_budget::snapshot();
            let fixture = Fixture::classified(2, delivered, true, platform);
            let identity =
                resolve(&IPC.read(), fixture.caller.id(), fixture.call).unwrap().unwrap();
            let borrow_capacity = IPC.read().reply_tokens[&identity.token].borrows.capacity();
            let baseline = node_budget::snapshot();
            memory::budget::mark_platform(fixture.caller);
            let policy = crate::resource_policy::POLICY.ipc_nodes;
            let available = if platform {
                policy.total[0] - baseline.0[0]
            } else {
                policy.ordinary[0] - baseline.1[0]
            };
            let pressure =
                Reservation::<IpcNodes, 2>::reserve(platform, [available - bytes(2) + 1, 0])
                    .unwrap();
            let full = node_budget::snapshot();
            {
                let _heap = PRIMARY_ALLOCATOR.lock();
                assert!(matches!(
                    PreparedCancellation::prepare(fixture.caller.id(), fixture.call),
                    Err(IpcError::ResourceLimit)
                ));
            }
            assert_eq!(node_budget::snapshot(), full);
            {
                let ipc = IPC.read();
                assert!(
                    resolve(&ipc, fixture.caller.id(), fixture.call).unwrap() == Some(identity)
                );
                assert_eq!(ipc.reply_tokens[&identity.token].claim_epoch, 0);
                assert_eq!(ipc.reply_tokens[&identity.token].borrows, fixture.borrows);
            }
            for borrow in &fixture.borrows {
                assert!(object::info(borrow.borrower, borrow.borrower_cap).unwrap().mapped);
            }
            drop(pressure);
            assert_eq!(node_budget::snapshot(), baseline);
            let operation =
                PreparedCancellation::prepare(fixture.caller.id(), fixture.call).unwrap();
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
            let mut released = 0;
            operation.finish_with_cleanup(|loan| {
                unlocked();
                assert_eq!(node_budget::snapshot(), charged);
                finished += 1;
                loan.finish_observed(unlocked)
            }, |owner| {
                unlocked();
                if !platform {
                    // Token, pending call and caller cap were actually disposed;
                    // the empty work buffer and full attachment catalog remain.
                    let retired = (node_budget::Storage::<(ReplyTokenId, ReplyToken)>::allocation_bytes()
                        + node_budget::Storage::<(PendingCallId, PendingCall)>::allocation_bytes()
                        + node_budget::Storage::<(CapabilityId, AdmittedCapability)>::allocation_bytes()
                        + borrow_capacity * core::mem::size_of::<MemoryBorrow>()) as u64;
                    assert_eq!(node_budget::snapshot(), ([charged.0[0] - retired, charged.0[1] - 3], [charged.1[0] - retired, charged.1[1] - 3]));
                }
                released += 1;
                attachments::release_one(owner);
            }).unwrap();
            assert_eq!(finished, 2);
            assert_eq!(
                released,
                if delivered {
                    0
                } else {
                    2
                }
            );
            fixture.close();
            assert_eq!(node_budget::snapshot(), before);
        }
    }
    let before = node_budget::snapshot();
    let fixture = Fixture::create(0, false, false);
    let baseline = node_budget::snapshot();
    let policy = crate::resource_policy::POLICY.ipc_nodes;
    let pressure =
        Reservation::<IpcNodes, 2>::reserve(false, [policy.ordinary[0] - baseline.1[0], 0])
            .unwrap();
    let full = node_budget::snapshot();
    let operation = {
        let _heap = PRIMARY_ALLOCATOR.lock();
        PreparedCancellation::prepare(fixture.caller.id(), fixture.call).unwrap()
    };
    assert_eq!(operation.loans.capacity(), 0);
    assert_eq!(node_budget::snapshot(), full);
    drop(pressure);
    operation.finish_with(|_| panic!("empty cancellation revoked a loan")).unwrap();
    fixture.close();
    assert_eq!(node_budget::snapshot(), before);
    crate::logln!(
        "[IPC cancellation work] parent={} entry={} max={} bytes; queued/delivered ordinary/total \
         quota denial under heap guard before claim, original class after promotion, full \
         capacity through final pop/attachment release, zero-work admission at quota and exact \
         recovery",
        core::mem::size_of::<PreparedCancellation<'_>>(),
        bytes(1),
        bytes(CAP_VECTOR_MAX)
    );
}
