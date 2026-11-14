//! Real accepted claims keep the original sponsor when a different actor waits.
use super::*;
use crate::memory::kernel_buffers::{
    self,
    test_metadata_limit as limit,
    test_metadata_used as used,
};

fn exercise(
    asid: AddressSpaceId,
    cap: CapabilityId,
    principal: u64,
    other: u64,
    storage: CapturedStorage,
    complete: impl FnOnce(),
) -> Source {
    let bytes = deferred::PreparedDisposal::allocation_bytes() as u64;
    let before = deferred::admission::snapshot();
    let base = used(principal);
    let other_base = used(other);
    deferred::admission::funded_allocation_failure(storage, principal);

    // Real list rejection follows successful funded node allocation. Both
    // budgets must refund the unused candidate outside IPC.
    let lists_before = crate::klib::observer::list_budget::node_used();
    let mut lists = Vec::new();
    while let Ok(list) = ObserverList::<waiter_budget::Charge>::try_new(1, false) {
        lists.push(list);
    }
    for _ in 0..64 {
        assert!(matches!(Source::prepare(storage), Err(IpcError::ResourceLimit)));
        assert_eq!(used(principal), base);
        assert_eq!(deferred::admission::snapshot(), before);
    }
    drop(lists);
    assert_eq!(crate::klib::observer::list_budget::node_used(), lists_before);

    limit(principal, base + bytes - 1);
    let mut waiter = Wait::new();
    let result = {
        let _heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
        waiter.wait_with(asid, cap, |_| panic!("unfunded claim waited"), Source::prepare)
    };
    assert_eq!(result, Err(IpcError::ResourceLimit));
    assert_eq!(waiter.finish(result), result);
    assert_eq!(used(principal), base);
    assert_eq!(used(other), other_base);
    assert_eq!(deferred::admission::snapshot(), before);

    limit(principal, base + bytes);
    let mut waiter = Wait::new();
    let mut late = None;
    let result = waiter.wait_with(
        asid,
        cap,
        |interest| {
            assert_eq!(used(principal), base + bytes);
            assert_eq!(used(other), other_base, "cleanup actor acquired the source charge");
            let _heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
            assert!(matches!(Source::prepare(storage), Err(IpcError::ResourceLimit)));
            late = Some(Interest(interest.0.clone()));
            Err(IpcError::ResourceLimit)
        },
        Source::prepare,
    );
    assert_eq!(result, Err(IpcError::ResourceLimit));
    assert_eq!(waiter.finish(result), result);
    assert_eq!(
        deferred::admission::snapshot().1,
        [before.1[0] + bytes, before.1[1] + 1],
        "promotion reclassified accepted storage"
    );
    complete();
    assert!(late.as_ref().unwrap().completed());
    assert_eq!(
        deferred::admission::snapshot().1,
        [before.1[0] + bytes, before.1[1] + 1],
        "claim completion refunded queued storage"
    );
    let queued = used(principal);
    deferred::drain();
    assert_eq!(used(principal), queued - bytes);
    assert_eq!(deferred::admission::snapshot(), before);
    let late_list = crate::klib::observer::list_budget::node_used();
    drop(late);
    assert_eq!(crate::klib::observer::list_budget::node_used(), (late_list.0 - 1, late_list.1 - 1));
    limit(principal, crate::resource_policy::POLICY.memory.principal_metadata_bytes);
    Source::prepare(storage).unwrap()
}

pub(super) fn run() {
    deferred::drain();
    let nodes_before = deferred::admission::snapshot();
    let bytes = deferred::PreparedDisposal::allocation_bytes() as u64;
    for reply in [true, false] {
        let name: &[u8] = if reply {
            b"ipc-reply-source"
        } else {
            b"ipc-endpoint-source"
        };
        let (caller, caller_principal) = buffer_tests::signed_root(name);
        let (server, server_principal) = buffer_tests::signed_root(b"ipc-source-server");
        let endpoint = endpoint_create(server.id(), 0x5052_494e, 1, 4).unwrap();
        let (principal, retained) = if reply {
            let connection =
                connection_delegate(server.id(), endpoint, caller.id(), ConnectionRights::ALL)
                    .unwrap();
            let call = scalar_call(caller.id(), connection, 1, 0).unwrap();
            let token_cap = receive(server.id(), endpoint).unwrap().reply.unwrap();
            let operation =
                cancellation::PreparedCancellation::prepare(server.id(), token_cap).unwrap();
            let storage = {
                let mut ipc = IPC.write();
                let Capability::ReplyToken {
                    token,
                } = ipc.cap(server.id(), token_cap).unwrap()
                else {
                    unreachable!()
                };
                let epoch = ipc.reply_tokens[&token].claim_epoch;
                Claim::Reply(token, epoch).source(&mut ipc).unwrap().1
            };
            memory::budget::mark_platform(caller);
            // The server waits on its reply capability, but the scalar call's
            // original empty loan allocator still identifies the caller.
            let retained = exercise(
                server.id(),
                token_cap,
                caller_principal,
                server_principal,
                storage,
                || {
                    operation.finish_with(|_| panic!("scalar claim has loans")).unwrap();
                },
            );
            assert!(poll_reply(caller.id(), call).unwrap().is_some());
            (caller_principal, retained)
        } else {
            let operation =
                endpoint_close::PreparedEndpointClose::prepare(server.id(), endpoint).unwrap();
            let storage = {
                let mut ipc = IPC.write();
                let Capability::Endpoint {
                    endpoint,
                    ..
                } = ipc.cap(server.id(), endpoint).unwrap()
                else {
                    unreachable!()
                };
                let epoch = ipc.endpoints[&endpoint].claim_epoch;
                Claim::Endpoint(endpoint, epoch).source(&mut ipc).unwrap().1
            };
            memory::budget::mark_platform(server);
            let retained = exercise(
                server.id(),
                endpoint,
                server_principal,
                caller_principal,
                storage,
                || {
                    operation
                        .finish_with(
                            |_| panic!("empty endpoint has loans"),
                            || panic!("empty endpoint waited"),
                        )
                        .unwrap();
                },
            );
            (server_principal, retained)
        };
        let old = if reply {
            caller
        } else {
            server
        };
        let other = if reply {
            server
        } else {
            caller
        };
        memory::close_user_address_space_handle(old).unwrap();
        assert_eq!(used(principal), bytes);
        let (successor, successor_principal) = buffer_tests::signed_root(b"ipc-source-successor");
        assert_eq!(successor.id(), old.id());
        assert_ne!(successor, old);
        let successor_base = used(successor_principal);
        {
            let _ipc = IPC.write();
            let _heap = memory::allocators::global_allocator::PRIMARY_ALLOCATOR.lock();
            kernel_buffers::test_with_history_locked(|| {
                deferred::admission::with_locked(|| retained.finish())
            });
        }
        assert_eq!(used(principal), bytes);
        deferred::drain();
        assert_eq!(used(principal), 0);
        assert_eq!(used(successor_principal), successor_base);
        memory::close_user_address_space_handle(other).unwrap();
        memory::close_user_address_space_handle(successor).unwrap();
        assert_eq!(used(successor_principal), 0);
        assert_eq!(deferred::admission::snapshot(), nodes_before);
    }
    crate::logln!(
        "[IPC claim principal] reply caller and endpoint owner funded; allocator/list rejection, \
         heap-held quota denial, different actor, promotion, accepted wait rejection, queued \
         retention and original refunds across retired generations passed"
    );
}
