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
    pub unsafe fn spawn(entry_vaddr: usize, target_lp: u32) -> Self {
        let (tid, generation) =
            unsafe { catten_syscall::spawn_thread_with_generation(entry_vaddr, target_lp) };
        Self {
            identity: ThreadIdentity::new(tid, generation),
        }
    }

    pub const fn id(&self) -> u64 {
        self.identity.tid()
    }

    pub fn join(self) -> Result<i64, ThreadError> {
        let cap = catten_syscall::observe_thread_exit_generation(
            self.identity.tid(),
            self.identity.generation(),
        );
        if cap == catten_syscall::COMPLETION_SUBMIT_FAILED {
            return Err(ThreadError::ObserverRegistrationFailed);
        }
        Completion::from_kernel(cap).and_then(Completion::wait).map_err(ThreadError::Completion)
    }
}

/// An owned completion capability.
///
/// Dropping a pending completion requests cancellation, waits for the terminal
/// state, and only then closes the capability. This is the appropriate wrapper
/// for timers, connection-close watches, and other buffer-free operations.
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
