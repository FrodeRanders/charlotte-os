//! Raft state-machine adaptation and catalog query helpers.

use alloc::{
    boxed::Box,
    sync::Arc,
    vec::Vec,
};

use catten_graft::state_machine::{
    QueryableStateMachine,
    StateMachine,
};
use catten_services::{
    dns,
    name_catalog::NameCatalog,
};

pub(super) fn persistent_namespace(cluster_id: &[u8], node_id: &[u8]) -> u64 {
    // Stable FNV-1a over the cluster/node tuple. This selects an object-store
    // namespace; it is not used as a security boundary.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in
        cluster_id.iter().copied().chain(core::iter::once(0xff)).chain(node_id.iter().copied())
    {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Give Raft and the service shared ownership of the same catalog.
pub(super) fn state_machine(catalog: Arc<NameCatalog>) -> Box<dyn StateMachine> {
    Box::new(CatalogMachine(catalog))
}

struct CatalogMachine(Arc<NameCatalog>);

impl StateMachine for CatalogMachine {
    fn apply(&self, term: u64, command: &[u8]) {
        self.0.apply(term, command);
    }

    fn apply_with_result(&self, term: u64, command: &[u8]) -> Vec<u8> {
        self.0.apply_with_result(term, command)
    }

    fn snapshot(&self) -> Vec<u8> {
        self.0.snapshot()
    }

    fn restore(
        &self,
        snapshot_data: &[u8],
    ) -> Result<(), catten_graft::state_machine::SnapshotError> {
        self.0.restore(snapshot_data)
    }

    fn reset(&self) {
        self.0.reset();
    }

    fn as_queryable(&self) -> Option<&dyn QueryableStateMachine> {
        Some(self.0.as_ref())
    }
}

enum CatalogBacking {
    Empty,
    Memory(catten_rt::owned::OwnedMemory),
    Mapping(catten_rt::owned::MappedMemory<catten_rt::owned::Writable>),
}

/// One owner for every transient field from preparation through publication.
/// The output is destroyed before unused reply authority on abandonment.
struct CatalogReply {
    backing: CatalogBacking,
    bytes: Vec<u8>,
    reply: Option<catten_rt::owned::ReplyToken>,
}

impl CatalogReply {
    fn reject(mut self, error: i64) {
        drop(core::mem::replace(&mut self.backing, CatalogBacking::Empty));
        if let Some(reply) = self.reply.take() {
            let _ = reply.reply(error);
        }
    }

    fn prepare(&mut self) -> Result<(), i64> {
        use catten_rt::owned::OwnedMemory;
        self.backing = CatalogBacking::Memory(
            OwnedMemory::allocate(self.bytes.len().div_ceil(4096))
                .map_err(|_| dns::ERR_RESOURCE_LIMIT)?,
        );
        let CatalogBacking::Memory(memory) =
            core::mem::replace(&mut self.backing, CatalogBacking::Empty)
        else {
            unreachable!()
        };
        self.backing = CatalogBacking::Mapping(match memory.map_writable() {
            Ok(mapping) => mapping,
            Err((memory, _)) => {
                self.backing = CatalogBacking::Memory(memory);
                return Err(dns::ERR_RESOURCE_LIMIT);
            }
        });
        let CatalogBacking::Mapping(mapping) = &mut self.backing else {
            unreachable!()
        };
        mapping.as_mut_slice()[..self.bytes.len()].copy_from_slice(&self.bytes);
        let CatalogBacking::Mapping(mapping) =
            core::mem::replace(&mut self.backing, CatalogBacking::Empty)
        else {
            unreachable!()
        };
        self.backing = CatalogBacking::Memory(match mapping.unmap() {
            Ok(memory) => memory,
            Err((mapping, _)) => {
                self.backing = CatalogBacking::Mapping(mapping);
                return Err(dns::ERR_RESOURCE_LIMIT);
            }
        });
        Ok(())
    }

    fn publish(mut self) {
        let CatalogBacking::Memory(memory) =
            core::mem::replace(&mut self.backing, CatalogBacking::Empty)
        else {
            unreachable!()
        };
        // Rejected move returns the memory owner; dropping its result releases
        // it through the existing runtime policy exactly once, without replay.
        let _ = self
            .reply
            .take()
            .expect("catalog reply authority")
            .reply_move(memory, self.bytes.len() as i64);
    }
}

/// Consume the catalog observation's received transaction before any scalar
/// attachment fallback can touch it. No new cleanup/retry policy is added.
pub(super) fn reply_catalog(mut request: catten_rt::owned::IncomingMessage, catalog: &NameCatalog) {
    let invalid = request.arg0 != 0 || request.memory.is_some() || request.connection.is_some();
    let mut operation = CatalogReply {
        backing: CatalogBacking::Empty,
        bytes: Vec::new(),
        reply: request.reply.take(),
    };
    drop(request);
    if operation.reply.is_none() {
        return;
    }
    if invalid {
        operation.reject(dns::ERR_BAD_OPCODE);
        return;
    }
    operation.bytes = match catalog.catalog_result() {
        Ok(bytes) => bytes,
        Err(error) => {
            operation.reject(match error {
                catten_services::dns_catalog::EncodeError::TooLarge => dns::ERR_TOO_LARGE,
                catten_services::dns_catalog::EncodeError::InvalidRecord => dns::ERR_BAD_OPCODE,
                catten_services::dns_catalog::EncodeError::AllocationFailed => {
                    dns::ERR_RESOURCE_LIMIT
                }
            });
            return;
        }
    };
    if let Err(error) = operation.prepare() {
        operation.reject(error);
        return;
    }
    operation.publish();
}
