//! Shared fail-closed policy for reference service reactors.

pub use knytos_rt::cq::WaitOutcome;

/// Recheck all sources after Ready or Deadline. These reactors have no admitted
/// recovery owner for a rejected wait, so abort the domain with pending resources
/// retained rather than spin, advance protocol time or replay accepted work.
/// Specialized callers can use knytos_rt::cq::wait and retain their own explicit
/// admission policy, operation owners and original deadlines.
pub fn wait(timeout_ms: Option<u64>, cq: u32) -> WaitOutcome {
    match knytos_rt::cq::wait(timeout_ms, cq) {
        Ok(outcome) => outcome,
        Err(error) => {
            knytos_rt::logln!("[reactor] CQ {} wait rejected: {:?}", cq, error);
            knytos_rt::domain_abort()
        }
    }
}
