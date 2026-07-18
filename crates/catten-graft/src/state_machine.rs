use alloc::vec::Vec;

pub trait StateMachine: Send + Sync {
    fn apply(&self, term: u64, command: &[u8]);

    fn apply_with_result(&self, term: u64, command: &[u8]) -> Vec<u8> {
        self.apply(term, command);
        Vec::new()
    }

    fn snapshot(&self) -> Vec<u8> {
        Vec::new()
    }

    fn restore(&self, _snapshot_data: &[u8]) {}

    fn as_queryable(&self) -> Option<&dyn QueryableStateMachine> {
        None
    }
}

pub trait QueryableStateMachine: StateMachine {
    fn query(&self, query: &[u8]) -> Vec<u8>;
}
