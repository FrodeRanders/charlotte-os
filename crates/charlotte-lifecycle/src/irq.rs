//! Allocation-free deferred IRQ readiness. Each route has independent storage;
//! a busy or retired route cannot consume another route's notification slot.

use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

/// One bit marks readiness; the remaining bits name a monotonic route lifetime.
pub const MAX_ROUTE_GENERATION: u64 = u64::MAX >> 1;

/// Reserve a later binding identity and leave one generation for retirement.
/// The caller serializes route management and never resets its generation.
pub const fn next_binding_generation(current: u64) -> Option<u64> {
    if current < MAX_ROUTE_GENERATION - 1 {
        Some(current + 1)
    } else {
        None
    }
}

/// Retirement always remains possible, including after binding exhaustion.
pub const fn retired_generation(current: u64) -> u64 {
    if current < MAX_ROUTE_GENERATION {
        current + 1
    } else {
        current
    }
}

/// Single coalescing mailbox, not a FIFO. The high watermark survives claiming
/// readiness, preventing an older in-flight publisher from replacing a newer
/// pending wake or resurrecting an already retired lifetime. Multiple consumers
/// may claim concurrently; at most one receives each published readiness bit.
pub struct DeferredIrqWake {
    state: AtomicU64,
}

impl DeferredIrqWake {
    pub const fn new() -> Self {
        Self {
            state: AtomicU64::new(0),
        }
    }

    /// Publish readiness for a captured binding. Returns false for malformed
    /// identities; valid but stale publications are harmlessly coalesced away.
    pub fn publish(&self, generation: u64) -> bool {
        if generation == 0 || generation >= MAX_ROUTE_GENERATION {
            return false;
        }
        self.state.fetch_max((generation << 1) | 1, Ordering::AcqRel);
        true
    }

    /// Establish a strictly newer binding/retirement watermark. Management
    /// must advance generations; this is not an acknowledgement of the same
    /// generation's pending readiness.
    pub fn advance(&self, generation: u64) {
        assert!(generation > 0 && generation <= MAX_ROUTE_GENERATION);
        self.state.fetch_max(generation << 1, Ordering::AcqRel);
    }

    pub fn claim(&self) -> Option<u64> {
        // Avoid writing all route cache lines on every idle/yield scan.
        if self.state.load(Ordering::Acquire) & 1 == 0 {
            return None;
        }
        let previous = self.state.fetch_and(!1, Ordering::AcqRel);
        if previous & 1 == 0 {
            None
        } else {
            Some(previous >> 1)
        }
    }
}

impl Default for DeferredIrqWake {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn flood_coalesces_without_using_other_routes() {
        let slots: [_; 16] = core::array::from_fn(|_| DeferredIrqWake::new());
        for slot in &slots {
            slot.advance(1);
        }
        for _ in 0..4096 {
            assert!(slots[0].publish(1));
        }
        for slot in &slots[1..] {
            assert!(slot.publish(1));
        }
        for slot in &slots {
            assert_eq!(slot.claim(), Some(1));
            assert_eq!(slot.claim(), None);
        }
    }

    #[test]
    fn retire_rebind_and_late_old_publisher_preserve_new_wake() {
        let slot = DeferredIrqWake::new();
        slot.advance(1);
        slot.publish(1);
        slot.advance(2);
        assert_eq!(slot.claim(), None);
        slot.publish(1);
        assert_eq!(slot.claim(), None);
        slot.advance(3);
        slot.publish(3);
        slot.publish(1);
        assert_eq!(slot.claim(), Some(3));
        slot.publish(1);
        assert_eq!(slot.claim(), None);
        slot.publish(3);
        assert_eq!(slot.claim(), Some(3));
    }

    #[test]
    fn claim_then_publish_keeps_next_wake() {
        let slot = DeferredIrqWake::new();
        slot.publish(1);
        assert_eq!(slot.claim(), Some(1));
        slot.publish(1);
        assert_eq!(slot.claim(), Some(1));
        slot.advance(2);
        slot.publish(3);
        assert_eq!(slot.claim(), Some(3));
        assert_eq!(slot.claim(), None);
    }

    #[test]
    fn generation_exhaustion_never_reuses_an_identity() {
        assert_eq!(next_binding_generation(0), Some(1));
        assert_eq!(
            next_binding_generation(MAX_ROUTE_GENERATION - 2),
            Some(MAX_ROUTE_GENERATION - 1)
        );
        assert_eq!(next_binding_generation(MAX_ROUTE_GENERATION - 1), None);
        assert_eq!(next_binding_generation(MAX_ROUTE_GENERATION), None);
        assert_eq!(retired_generation(MAX_ROUTE_GENERATION - 1), MAX_ROUTE_GENERATION);
        assert_eq!(retired_generation(MAX_ROUTE_GENERATION), MAX_ROUTE_GENERATION);
        let slot = DeferredIrqWake::new();
        assert!(!slot.publish(0));
        assert!(!slot.publish(MAX_ROUTE_GENERATION));
        assert!(!slot.publish(u64::MAX));
        slot.publish(MAX_ROUTE_GENERATION - 1);
        slot.advance(MAX_ROUTE_GENERATION);
        slot.publish(MAX_ROUTE_GENERATION - 1);
        assert_eq!(slot.claim(), None);
    }

    #[test]
    fn concurrent_stale_publishers_cannot_overwrite_latest_readiness() {
        let slot = std::sync::Arc::new(DeferredIrqWake::new());
        std::thread::scope(|scope| {
            for generation in 1..=4 {
                let slot = slot.clone();
                scope.spawn(move || {
                    for _ in 0..10000 {
                        slot.publish(generation);
                    }
                });
            }
            slot.advance(5);
            slot.publish(6);
        });
        assert_eq!(slot.claim(), Some(6));
        assert_eq!(slot.claim(), None);
    }

    #[test]
    fn concurrent_claimers_receive_one_coalesced_wake() {
        let slot = DeferredIrqWake::new();
        let claims = core::sync::atomic::AtomicUsize::new(0);
        slot.publish(1);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    if slot.claim() == Some(1) {
                        claims.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
        });
        assert_eq!(claims.load(Ordering::Relaxed), 1);
    }
}
