//! Raft state-machine adaptation and catalog query helpers.

use alloc::{
    boxed::Box,
    sync::Arc,
    vec::Vec,
};

use catten_graft::{
    node::RaftNode,
    state_machine::{
        QueryableStateMachine,
        StateMachine,
    },
};
use catten_services::{
    dns,
    name_catalog::{
        CatalogEntry,
        NameCatalog,
        decode_query_result,
        encode_lookup_query,
    },
};

/// Query the catalog only after the Raft read barrier admits the request.
pub(super) fn linearizable_entry(
    node: &RaftNode,
    name: &[u8],
) -> Result<Option<CatalogEntry>, i64> {
    node.handle_client_query(encode_lookup_query(name))
        .map(|bytes| decode_query_result(&bytes))
        .map_err(|_| dns::ERR_NOT_LEADER)
}

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

    fn restore(&self, snapshot_data: &[u8]) {
        self.0.restore(snapshot_data);
    }

    fn as_queryable(&self) -> Option<&dyn QueryableStateMachine> {
        Some(self.0.as_ref())
    }
}
