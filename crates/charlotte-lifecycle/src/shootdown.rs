//! Per-target epoch acknowledgements. Request ownership serializes publication;
//! abandoning one attempt does not let a late acknowledgement satisfy a retry.
use core::sync::atomic::{
    AtomicBool,
    AtomicU64,
    Ordering,
};

#[derive(Debug, PartialEq, Eq)]
pub enum BeginError {
    Busy,
    Exhausted,
}

pub struct Shootdown {
    owned: AtomicBool,
    requested: AtomicU64,
}
impl Default for Shootdown {
    fn default() -> Self {
        Self::new()
    }
}
impl Shootdown {
    pub const fn new() -> Self {
        Self {
            owned: AtomicBool::new(false),
            requested: AtomicU64::new(0),
        }
    }

    pub fn try_begin(&self) -> Result<Attempt<'_>, BeginError> {
        self.owned
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| BeginError::Busy)?;
        let Some(epoch) = self.requested.load(Ordering::Relaxed).checked_add(1) else {
            self.owned.store(false, Ordering::Release);
            return Err(BeginError::Exhausted);
        };
        // Page-table writes must precede publication; handlers acquire this
        // exact request before flushing, never re-read it when acknowledging.
        self.requested.store(epoch, Ordering::Release);
        Ok(Attempt {
            coordinator: self,
            epoch,
        })
    }

    pub fn requested(&self) -> u64 {
        self.requested.load(Ordering::Acquire)
    }
}

pub struct Acknowledgement(AtomicU64);
impl Default for Acknowledgement {
    fn default() -> Self {
        Self::new()
    }
}
impl Acknowledgement {
    pub const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// Call only after the captured request's physical invalidation completes.
    pub fn complete(&self, epoch: u64) {
        self.0.fetch_max(epoch, Ordering::Release);
    }
}

pub struct Attempt<'a> {
    coordinator: &'a Shootdown,
    epoch: u64,
}
impl Attempt<'_> {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn completed(&self, targets: &[Acknowledgement]) -> bool {
        targets.iter().all(|ack| ack.0.load(Ordering::Acquire) >= self.epoch)
    }
}
impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        // This releases coordinator ownership, never backing ownership. The
        // caller must retain/quarantine its receipt unless completed was true.
        self.coordinator.owned.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_send_and_late_ack_do_not_complete_retry() {
        let coordinator = Shootdown::new();
        let acks = [Acknowledgement::new(), Acknowledgement::new(), Acknowledgement::new()];
        let first = coordinator.try_begin().unwrap();
        let captured = coordinator.requested();
        acks[0].complete(first.epoch());
        acks[1].complete(first.epoch());
        assert!(!first.completed(&acks)); // failed target is never acknowledged
        drop(first);
        let retry = coordinator.try_begin().unwrap();
        acks[2].complete(captured); // old handler finishes after new publication
        assert!(!retry.completed(&acks));
        for ack in &acks {
            ack.complete(retry.epoch());
        }
        assert!(retry.completed(&acks));
    }
    #[test]
    fn duplicate_ack_is_not_another_target() {
        let coordinator = Shootdown::new();
        let acks = [Acknowledgement::new(), Acknowledgement::new()];
        let attempt = coordinator.try_begin().unwrap();
        acks[0].complete(attempt.epoch());
        acks[0].complete(attempt.epoch());
        assert!(!attempt.completed(&acks));
    }
    #[test]
    fn request_is_serialized_until_release() {
        let coordinator = Shootdown::new();
        let first = coordinator.try_begin().unwrap();
        assert!(matches!(coordinator.try_begin(), Err(BeginError::Busy)));
        let old = first.epoch();
        drop(first);
        assert!(coordinator.try_begin().unwrap().epoch() > old);
    }
    #[test]
    fn old_completion_cannot_regress_new_ack() {
        let coordinator = Shootdown::new();
        let first = coordinator.try_begin().unwrap();
        let old = first.epoch();
        drop(first);
        let next = coordinator.try_begin().unwrap();
        let acks = [Acknowledgement::new()];
        acks[0].complete(next.epoch());
        acks[0].complete(old);
        assert!(next.completed(&acks));
    }
    #[test]
    fn epoch_exhaustion_never_reuses_an_acknowledgement() {
        let coordinator = Shootdown::new();
        coordinator.requested.store(u64::MAX, Ordering::Relaxed);
        assert!(matches!(coordinator.try_begin(), Err(BeginError::Exhausted)));
        assert!(matches!(coordinator.try_begin(), Err(BeginError::Exhausted)));
        assert_eq!(coordinator.requested(), u64::MAX);
    }
}
