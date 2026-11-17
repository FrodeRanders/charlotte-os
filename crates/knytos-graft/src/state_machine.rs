use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    Unsupported,
    Malformed,
}

pub trait StateMachine: Send + Sync {
    fn apply(&self, term: u64, command: &[u8]);

    fn apply_with_result(&self, term: u64, command: &[u8]) -> Vec<u8> {
        self.apply(term, command);
        Vec::new()
    }

    fn snapshot(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Restore all application state, or reject without changing any state.
    /// Raft must not publish snapshot progress before this succeeds.
    fn restore(&self, snapshot_data: &[u8]) -> Result<(), SnapshotError> {
        if snapshot_data.is_empty() {
            Ok(())
        } else {
            Err(SnapshotError::Unsupported)
        }
    }

    /// Discard state belonging to a different consensus domain.
    ///
    /// This is distinct from restoring a snapshot: an empty byte string may
    /// be an invalid snapshot and implementations must reject it explicitly.
    /// Admission calls this only after durably fencing the node as a joiner.
    fn reset(&self);

    fn as_queryable(&self) -> Option<&dyn QueryableStateMachine> {
        None
    }
}

pub trait QueryableStateMachine: StateMachine {
    fn query(&self, query: &[u8]) -> Vec<u8>;
}
