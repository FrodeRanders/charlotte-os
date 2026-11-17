//! Typed waits on launch-owned completion queues. Readiness is a hint: it
//! does not observe results, adopt attachments or discharge operation owners.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitOutcome {
    Ready,
    Deadline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaitError {
    AdmissionRejected,
    InvalidQueueOrRequest,
    InvalidStatus(u64),
}

/// Wait for a new CQ work generation or one total timeout. None is event-only.
/// Keep every pending resource and borrow alive through either error outcome.
/// The kernel owns and cancels its transient source/watchdog registrations.
pub fn wait(timeout_ms: Option<u64>, cq: u32) -> Result<WaitOutcome, WaitError> {
    let (_, status) = catten_syscall::cq_wait_checked(timeout_ms, cq);
    decode(status, timeout_ms.is_some())
}

fn decode(status: u64, timed: bool) -> Result<WaitOutcome, WaitError> {
    use catten_syscall::completion_status as status_code;
    match status {
        status_code::READY => Ok(WaitOutcome::Ready),
        status_code::PENDING_OR_TIMEOUT if timed => Ok(WaitOutcome::Deadline),
        status_code::WAIT_ADMISSION_FAILED => Err(WaitError::AdmissionRejected),
        status_code::INVALID_CAPABILITY => Err(WaitError::InvalidQueueOrRequest),
        other => Err(WaitError::InvalidStatus(other)),
    }
}

#[cfg(test)]
mod tests {
    use catten_syscall::completion_status as status;

    use super::*;

    #[test]
    fn admission_rejection_never_becomes_readiness_or_deadline() {
        for timed in [false, true] {
            assert_eq!(
                decode(status::WAIT_ADMISSION_FAILED, timed),
                Err(WaitError::AdmissionRejected)
            );
            assert_eq!(decode(status::READY, timed), Ok(WaitOutcome::Ready));
        }
        assert_eq!(decode(status::PENDING_OR_TIMEOUT, true), Ok(WaitOutcome::Deadline));
        assert_eq!(decode(status::PENDING_OR_TIMEOUT, false), Err(WaitError::InvalidStatus(1)));
    }

    #[test]
    fn invalid_queue_and_unknown_status_fail_closed() {
        assert_eq!(decode(status::INVALID_CAPABILITY, true), Err(WaitError::InvalidQueueOrRequest));
        assert_eq!(decode(u64::MAX, true), Err(WaitError::InvalidStatus(u64::MAX)));
    }
}
