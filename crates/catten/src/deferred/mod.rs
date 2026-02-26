//! # Deferred Work Manager

use concurrent_queue::ConcurrentQueue;

pub enum DeferredTask {}
pub struct SubmissionEntry(u64, DeferredTask);

pub enum DeferredTaskResult {}

pub struct CompletionEntry(u64, DeferredTaskResult);

#[derive(Debug)]
pub struct DeferredWorkManager {
    id_gen: u64,
    submission_ring: ConcurrentQueue<SubmissionEntry>,
    completion_ring: ConcurrentQueue<CompletionEntry>,
}
