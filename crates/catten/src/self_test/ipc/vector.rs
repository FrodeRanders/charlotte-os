//! Mixed-mode transactions through the real kernel IPC entry points.

use alloc::vec::Vec;

use super::create_ipc_memory_test_address_space as create;
use crate::{
    capability::admission_tests,
    ipc::{
        self,
        ConnectionRights,
        IpcError,
    },
    logln,
    memory::{
        AddressSpaceHandle,
        budget,
        current_address_space_handle,
        object,
    },
    self_test::close_test_address_space,
};

struct Round {
    server: usize,
    client: usize,
    handle: AddressSpaceHandle,
    endpoint: u64,
    connection: u64,
    sources: [u64; 4],
    result: u64,
}

impl Round {
    fn new() -> Self {
        let server = create("mixed vector server");
        let client = create("mixed vector client");
        let endpoint = ipc::endpoint_create(server, 0x5645_4354, 1, 4).unwrap();
        let connection =
            ipc::connection_delegate(server, endpoint, client, ConnectionRights::CALL).unwrap();
        let sources = core::array::from_fn(|index| {
            let cap = object::allocate(client, 1).unwrap();
            object::write_bytes(client, cap, &[(index + 1) as u8]).unwrap();
            cap
        });
        let result = object::allocate(server, 1).unwrap();
        Self {
            server,
            client,
            handle: current_address_space_handle(client).unwrap(),
            endpoint,
            connection,
            sources,
            result,
        }
    }

    fn descriptor(&self, entries: &[(u64, u32)]) -> u64 {
        let descriptor = object::allocate(self.client, 1).unwrap();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for &(cap, mode) in entries {
            bytes.extend_from_slice(&cap.to_le_bytes());
            bytes.extend_from_slice(&mode.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
        }
        object::write_bytes(self.client, descriptor, &bytes).unwrap();
        descriptor
    }

    fn call(&self) -> u64 {
        let modes = [1, 0, 2, 3];
        let entries: Vec<_> = self.sources.iter().copied().zip(modes).collect();
        let descriptor = self.descriptor(&entries);
        let call = ipc::vector_call(self.client, self.connection, 1, 0, descriptor).unwrap();
        assert_eq!(
            object::info(self.client, descriptor),
            Err(object::MemoryObjectError::UnknownCapability)
        );
        assert!(object::info(self.client, self.sources[2]).unwrap().lent);
        assert!(object::info(self.client, self.sources[3]).unwrap().lent);
        for cap in &self.sources[2..] {
            assert_eq!(
                object::write_bytes(self.client, *cap, &[0xff]),
                Err(object::MemoryObjectError::LendingActive)
            );
            assert!(matches!(
                object::pin_for_dma(self.client, *cap, false, true, false),
                Err(object::MemoryObjectError::LendingActive)
            ));
        }
        assert!(matches!(
            object::pin_for_dma(self.client, self.sources[3], true, false, false),
            Err(object::MemoryObjectError::LendingActive)
        ));
        assert_eq!(
            object::snapshot_bytes(self.client, self.sources[3], 1),
            Err(object::MemoryObjectError::LendingActive)
        );
        call
    }

    fn receive(&self) -> (u64, [u64; 4]) {
        let message = ipc::receive_vec(self.server, self.endpoint, self.result).unwrap();
        let bytes = object::snapshot_bytes(self.server, self.result, 2 + 4 * 8).unwrap();
        assert_eq!(&bytes[..2], &4u16.to_le_bytes());
        let caps = core::array::from_fn(|index| {
            u64::from_le_bytes(bytes[2 + index * 8..2 + (index + 1) * 8].try_into().unwrap())
        });
        assert_eq!(message.memory, Some(caps[0]));
        for (index, &cap) in caps.iter().enumerate() {
            assert_eq!(object::snapshot_bytes(self.server, cap, 1).unwrap(), [(index + 1) as u8]);
        }
        object::map_any(self.server, caps[2], false).unwrap();
        object::map_any(self.server, caps[3], true).unwrap();
        object::write_bytes(self.server, caps[3], &[0x5a]).unwrap();
        (message.reply.unwrap(), caps)
    }

    fn assert_loans_ended(&self) {
        for cap in &self.sources[2..] {
            assert!(!object::info(self.client, *cap).unwrap().lent);
            object::write_bytes(self.client, *cap, &[0xa5]).unwrap();
        }
    }
}

impl Drop for Round {
    fn drop(&mut self) {
        close_test_address_space(self.client).unwrap();
        close_test_address_space(self.server).unwrap();
        assert_eq!(budget::used(self.handle), budget::Amount::default());
    }
}

pub(super) fn test_mixed_vectors() {
    for mode in [0, 1, 2, 3] {
        let round = Round::new();
        let descriptor = round.descriptor(&[(round.sources[0], mode), (u64::MAX, 0)]);
        let before = budget::used(round.handle);
        let target_records = admission_tests::test_namespace_used(round.server);
        admission_tests::test_fill_remaining_namespace(round.client);
        assert_eq!(
            ipc::vector_call(round.client, round.connection, 1, 0, descriptor),
            Err(IpcError::MemoryTransferFailed)
        );
        assert_eq!(ipc::receive(round.server, round.endpoint), Err(IpcError::NoMessage));
        assert_eq!(budget::used(round.handle), before);
        assert_eq!(admission_tests::test_namespace_used(round.server), target_records);
        assert_eq!(
            admission_tests::test_namespace_used(round.client),
            admission_tests::TEST_NAMESPACE_LIMIT
        );
        for cap in round.sources {
            assert!(!object::info(round.client, cap).unwrap().lent);
            object::write_bytes(round.client, cap, &[0x5a]).unwrap();
        }
        assert!(object::info(round.client, descriptor).is_ok());
    }
    // Reject private copy backing after a read loan has already been staged.
    {
        let round = Round::new();
        let descriptor = round.descriptor(&[(round.sources[2], 2), (round.sources[1], 0)]);
        let before = budget::used(round.handle);
        budget::set_limit(round.handle, before).unwrap();
        assert_eq!(
            ipc::vector_call(round.client, round.connection, 1, 0, descriptor),
            Err(IpcError::MemoryTransferFailed)
        );
        assert_eq!(budget::used(round.handle), before);
        assert!(!object::info(round.client, round.sources[2]).unwrap().lent);
        object::write_bytes(round.client, round.sources[2], &[0xa5]).unwrap();
        assert_eq!(ipc::receive(round.server, round.endpoint), Err(IpcError::NoMessage));
    }

    for outcome in 0..5 {
        let round = Round::new();
        let call = round.call();
        if outcome == 1 || outcome == 4 {
            if outcome == 1 {
                ipc::close_cap(round.client, call).unwrap();
                assert_eq!(ipc::receive(round.server, round.endpoint), Err(IpcError::NoMessage));
            } else {
                ipc::close_cap(round.server, round.endpoint).unwrap();
                assert_eq!(
                    ipc::poll_reply(round.client, call).unwrap().unwrap().result,
                    ipc::REPLY_ENDPOINT_CLOSED
                );
            }
            round.assert_loans_ended();
            assert_eq!(
                budget::used(round.handle),
                budget::Amount {
                    pages: 3,
                    objects: 3
                },
                "queued cancellation must release copies/moves as well as both loans"
            );
            continue;
        }
        let (reply, caps) = round.receive();
        match outcome {
            0 => {
                ipc::reply(round.server, reply, 42).unwrap();
                assert_eq!(ipc::poll_reply(round.client, call).unwrap().unwrap().result, 42);
            }
            2 => {
                ipc::close_cap(round.client, call).unwrap();
                assert_eq!(ipc::reply(round.server, reply, 42), Err(IpcError::UnknownCapability));
            }
            3 => {
                ipc::close_cap(round.server, reply).unwrap();
                assert_eq!(
                    ipc::poll_reply(round.client, call).unwrap().unwrap().result,
                    ipc::REPLY_CANCELLED
                );
            }
            _ => unreachable!(),
        }
        round.assert_loans_ended();
        for cap in &caps[2..] {
            assert_eq!(
                object::info(round.server, *cap),
                Err(object::MemoryObjectError::UnknownCapability)
            );
        }
        for cap in &caps[..2] {
            assert!(object::info(round.server, *cap).is_ok());
        }
    }
    logln!(
        "[ipc vector staging] all four modes cancel privately; mixed reply, queued/delivered \
         cancellation and endpoint close revoke all loans"
    );
}
