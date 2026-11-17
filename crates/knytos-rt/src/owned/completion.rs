//! Thread handles and completions.
//!
//! Child module of [`crate::owned`].

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionError {
    SubmissionFailed,
    Status(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadError {
    SpawnFailed,
    ObserverRegistrationFailed,
    Completion(CompletionError),
}

/// A generation-bound handle to an EL0 thread.
///
/// Dropping the handle detaches the thread. Joining consumes the handle and
/// uses the spawn-time generation, so a recycled TID cannot be mistaken for
/// the original thread.
#[must_use = "dropping a thread handle detaches it"]
#[derive(Debug)]
pub struct ThreadHandle {
    identity: ThreadIdentity,
}

impl ThreadHandle {
    /// Spawn an EL0 thread pinned to `target_lp`.
    ///
    /// # Safety
    /// `entry_vaddr` must identify a valid `extern "C" fn()` entry point in
    /// the current address space and `target_lp` must identify a logical CPU.
    pub unsafe fn spawn(entry_vaddr: usize, target_lp: u32) -> Result<Self, ThreadError> {
        let (tid, generation) = unsafe { kernel::spawn_thread(entry_vaddr, target_lp) };
        if tid == u64::MAX || generation == 0 {
            return Err(ThreadError::SpawnFailed);
        }
        Ok(Self {
            identity: ThreadIdentity::new(tid, generation),
        })
    }

    pub const fn id(&self) -> u64 {
        self.identity.tid()
    }

    pub fn join(self) -> Result<i64, ThreadError> {
        self.join_preserving().map_err(|(_, error)| error)
    }

    /// Registration or observation failure returns the generation-bound owner.
    /// An unsuccessful observation cannot establish that the worker exited.
    pub fn join_preserving(self) -> Result<i64, (Self, ThreadError)> {
        let cap = kernel::observe_thread_exit(self.identity.tid(), self.identity.generation());
        if cap == catten_syscall::COMPLETION_SUBMIT_FAILED {
            return Err((self, ThreadError::ObserverRegistrationFailed));
        }
        Completion::from_kernel(cap)
            .and_then(Completion::wait)
            .map_err(|error| (self, ThreadError::Completion(error)))
    }
}

/// An owned completion capability.
///
/// Dropping a pending completion requests cancellation, waits for the terminal
/// state, and only then closes the capability. This is the appropriate wrapper
/// for timers, connection-close watches, and other buffer-free operations.
/// Timer, endpoint-close and external thread-exit watch cancellation can finish
/// locally without terminating their targets. Worker-backed operations and other producers
/// may still need to acknowledge cancellation before Drop can finish.
#[must_use = "dropping a completion cancels and closes it"]
#[derive(Debug)]
pub struct Completion {
    cap: Option<u64>,
}

impl Completion {
    pub fn submit(op: OpCode) -> Result<Self, CompletionError> {
        Self::from_kernel(kernel::submit(op))
    }

    pub fn timer(timeout_ms: u64) -> Result<Self, CompletionError> {
        Self::from_kernel(kernel::submit_timer(timeout_ms))
    }

    /// Adopt a uniquely owned completion capability.
    ///
    /// # Safety
    /// `cap` must be a live completion capability owned by the caller and no
    /// other value may close, poll, wait for, or cancel it after adoption.
    pub unsafe fn from_raw(cap: u64) -> Result<Self, CompletionError> {
        Self::from_kernel(cap)
    }

    pub(super) fn from_kernel(cap: u64) -> Result<Self, CompletionError> {
        if cap == catten_syscall::COMPLETION_SUBMIT_FAILED {
            return Err(CompletionError::SubmissionFailed);
        }
        Ok(Self {
            cap: Some(cap),
        })
    }

    pub(super) fn raw_handle(&self) -> u64 {
        self.cap.expect("completion capability already consumed")
    }

    fn finish_result(&mut self, status: u64, result: u64) -> Result<Option<i64>, CompletionError> {
        match status {
            catten_syscall::completion_status::READY => {
                let cap = self.cap.take().expect("completion capability already consumed");
                kernel::close(cap);
                Ok(Some(result as i64))
            }
            catten_syscall::completion_status::PENDING_OR_TIMEOUT => Ok(None),
            catten_syscall::completion_status::WAIT_ADMISSION_FAILED => {
                // Wait admission failed; no terminal result was observed. Keep ownership
                // to retry the wait or let Drop cancel, wait for termination, and close.
                Err(CompletionError::Status(status))
            }
            other => {
                let cap = self.cap.take().expect("completion capability already consumed");
                kernel::close(cap);
                Err(CompletionError::Status(other))
            }
        }
    }

    pub fn poll(&mut self) -> Result<Option<i64>, CompletionError> {
        let (status, result) = kernel::poll(self.raw_handle());
        self.finish_result(status, result)
    }

    /// Match a CQ readiness hint without consuming this operation. A matching
    /// entry is not its terminal result: observe through `poll` before acting.
    /// An old entry cannot match an already consumed completion.
    pub fn matches_cq(&self, entry: &catten_syscall::CqEntry) -> bool {
        self.cap.is_some_and(|cap| cap == entry.cookie)
    }

    pub fn wait_timeout(&mut self, timeout_ms: u64) -> Result<Option<i64>, CompletionError> {
        let (status, result) = kernel::wait_timeout(self.raw_handle(), timeout_ms);
        self.finish_result(status, result)
    }

    pub fn wait(mut self) -> Result<i64, CompletionError> {
        let cap = self.raw_handle();
        kernel::wait(cap);
        let (status, result) = kernel::poll(cap);
        match self.finish_result(status, result)? {
            Some(result) => Ok(result),
            None => {
                Err(CompletionError::Status(catten_syscall::completion_status::PENDING_OR_TIMEOUT))
            }
        }
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        if let Some(cap) = self.cap.take() {
            kernel::cancel(cap);
            kernel::wait(cap);
            kernel::close(cap);
        }
    }
}

/// Bounded custody of generation-bound native worker handles. Reserve before
/// spawning. Stop workers through their application lifecycle, then join them.
/// Dropping a nonempty group aborts rather than silently detaching live work.
pub struct Workers {
    handles: Vec<ThreadHandle>,
    limit: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerCapacity;

impl Workers {
    pub fn new(limit: usize) -> Result<Self, WorkerCapacity> {
        if !(1..=64).contains(&limit) {
            return Err(WorkerCapacity);
        }
        let mut handles = Vec::new();
        handles.try_reserve_exact(limit).map_err(|_| WorkerCapacity)?;
        Ok(Self {
            handles,
            limit,
        })
    }

    pub fn remaining_capacity(&self) -> usize {
        self.limit - self.handles.len()
    }

    /// On rejection the caller retains its actual worker, not just its TID.
    pub fn insert(&mut self, handle: ThreadHandle) -> Result<(), ThreadHandle> {
        if self.remaining_capacity() == 0 {
            return Err(handle);
        }
        self.handles.push(handle);
        Ok(())
    }

    /// Join confirms exit. A failed observation restores the exact handle in
    /// its preallocated slot; neither the group nor the worker is discarded.
    pub fn join_all(&mut self) -> Result<(), ThreadError> {
        while let Some(handle) = self.handles.pop() {
            if let Err((handle, error)) = handle.join_preserving() {
                self.handles.push(handle);
                return Err(error);
            }
        }
        Ok(())
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        if !self.handles.is_empty() {
            kernel::abort_unjoined_workers();
        }
    }
}

#[cfg(test)]
mod worker_tests {
    extern crate std;
    use super::*;
    use crate::owned::kernel::{
        self,
        Event,
    };
    #[test]
    fn rejected_join_retains_original_generation_and_group_capacity() {
        let _guard = kernel::serial();
        kernel::reset();
        let mut workers = Workers::new(1).unwrap();
        let handle = unsafe { ThreadHandle::spawn(0x20000, 0) }.unwrap();
        workers.insert(handle).unwrap();
        kernel::update(|s| s.thread_observer = catten_syscall::COMPLETION_SUBMIT_FAILED);
        assert_eq!(workers.join_all(), Err(ThreadError::ObserverRegistrationFailed));
        assert_eq!(workers.remaining_capacity(), 0);
        kernel::update(|s| {
            s.thread = (7, 4);
            s.thread_observer = 20;
        });
        workers.join_all().unwrap();
        assert_eq!(workers.remaining_capacity(), 1);
        let observations = kernel::events()
            .into_iter()
            .filter(|e| matches!(e, Event::ObserveThread(_, _)))
            .collect::<std::vec::Vec<_>>();
        assert_eq!(observations, [Event::ObserveThread(7, 3), Event::ObserveThread(7, 3)]);
    }
    #[test]
    fn abandoned_group_aborts_instead_of_claiming_worker_exit() {
        let _guard = kernel::serial();
        kernel::reset();
        let mut workers = Workers::new(1).unwrap();
        workers.insert(unsafe { ThreadHandle::spawn(0x20000, 0) }.unwrap()).unwrap();
        assert!(std::panic::catch_unwind(|| drop(workers)).is_err());
        assert!(kernel::events().contains(&Event::AbortUnjoinedWorkers));
        assert!(!kernel::events().iter().any(|e| matches!(e, Event::ObserveThread(_, _))));
    }
}
