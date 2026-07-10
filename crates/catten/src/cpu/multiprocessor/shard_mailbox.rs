//! # Typed Shard Mailbox (Option B seed)
//!
//! A bounded, typed, owned-message transport between logical processors, built
//! on the kernel's IPI infrastructure. Each LP has one [`ShardReceiver<M>`]
//! (single-consumer) and an arbitrary number of cloneable [`ShardSender<M>`]
//! handles.
//!
//! This is the kernel realization of sitas's `ShardSender<M>` /
//! `ShardReceiver<M>` / `ShardMailbox<M>` pattern: senders push owned `M`
//! values into the target LP's bounded queue and deliver a wake IPI; the
//! receiver drains the queue at its leisure.
//!
//! ## Backpressure
//!
//! Each per-LP queue is bounded (default 256 entries). [`ShardSender::try_send`]
//! returns `Err(M)` when the target queue is full, matching the
//! `SubmitError::WouldBlock` / `ShardSendError::Full` contract. The receiver
//! drains entries one at a time via [`ShardReceiver::try_recv`].

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use concurrent_queue::ConcurrentQueue;

use crate::cpu::isa::lp::LpId;
use crate::cpu::isa::lp::ops::get_lp_id;
use crate::cpu::multiprocessor::get_lp_count;
use crate::cpu::multiprocessor::ipi;

/// Default per-LP queue capacity.
pub const DEFAULT_CAPACITY: usize = 256;

/// A cloneable producer handle for sending owned messages to one LP's mailbox.
///
/// All senders targeting the same LP share the same underlying bounded queue.
/// Cloning is cheap (`Arc` bump).
pub struct ShardSender<M> {
    shared: Arc<SharedMailbox<M>>,
}

impl<M> Clone for ShardSender<M> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

impl<M> ShardSender<M> {
    /// The LP this sender is bound to.
    pub fn target_lp(&self) -> LpId {
        self.shared.lp_id
    }

    /// Non-blocking send. On success the message is queued and a wake IPI is
    /// delivered to the target LP. Returns `Err(m)` with the original message
    /// if the target queue is full (backpressure).
    pub fn try_send(&self, message: M) -> Result<(), M> {
        let q = &self.shared.queue;
        if let Err(e) = q.push(message) {
            return Err(e.into_inner());
        }
        // Notify the target LP that work is available.
        ipi::send_ipi(self.shared.lp_id);
        self.shared.sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

/// A single-consumer receiver for one LP's mailbox. At most one receiver per LP
/// is allowed; construction panics if a receiver was already taken.
///
/// The receiver **must** be drained from the owning LP's thread context (not
/// from an interrupt handler).
pub struct ShardReceiver<M> {
    shared: Arc<SharedMailbox<M>>,
    taken: bool,
}

impl<M> ShardReceiver<M> {
    /// The LP this receiver is bound to.
    pub fn lp_id(&self) -> LpId {
        self.shared.lp_id
    }

    /// Non-blocking receive. Returns `None` when the queue is empty.
    pub fn try_recv(&mut self) -> Option<M> {
        self.shared.queue.pop().ok()
    }
}

impl<M> Drop for ShardReceiver<M> {
    fn drop(&mut self) {
        if self.taken {
            self.shared.receiver_taken.store(false, Ordering::Release);
        }
    }
}

/// Shared state behind a single mailbox — one bounded `ConcurrentQueue<M>` plus
/// accounting fields.
struct SharedMailbox<M> {
    lp_id: LpId,
    queue: ConcurrentQueue<M>,
    /// Whether a receiver has been created (true = taken).
    receiver_taken: AtomicBool,
    /// Cumulative count of successfully `try_send`-ed messages.
    sent: AtomicU64,
}

/// A set of per-LP typed mailboxes, indexed by `LpId`. One-shot factory:
/// after construction a `ShardReceiver<M>` can be taken for each LP, and
/// `ShardSender<M>` handles targeting any LP can be cloned freely.
pub struct ShardMailboxSet<M> {
    mailboxes: Vec<Arc<SharedMailbox<M>>>,
}

impl<M> ShardMailboxSet<M> {
    /// Creates one bounded queue per LP. `capacity` is the per-LP queue bound
    /// (backpressure threshold).
    pub fn new(capacity: usize) -> Self {
        let n = get_lp_count() as usize;
        let mut mailboxes = Vec::with_capacity(n);
        for lp in 0..n {
            mailboxes.push(Arc::new(SharedMailbox {
                lp_id: lp as LpId,
                queue: ConcurrentQueue::bounded(capacity),
                receiver_taken: AtomicBool::new(false),
                sent: AtomicU64::new(0),
            }));
        }
        Self { mailboxes }
    }

    /// Returns a cloneable sender for `target_lp`. `target_lp` must be in range.
    #[track_caller]
    pub fn sender_to(&self, target_lp: LpId) -> ShardSender<M> {
        let idx = target_lp as usize;
        assert!(
            idx < self.mailboxes.len(),
            "ShardMailboxSet::sender_to: LP {target_lp} out of range"
        );
        ShardSender {
            shared: Arc::clone(&self.mailboxes[idx]),
        }
    }

    /// Takes the receiver for `lp`. Panics if `lp` is out of range or if a
    /// receiver was already taken for that LP.
    #[track_caller]
    pub fn receiver_for(&self, lp: LpId) -> ShardReceiver<M> {
        let idx = lp as usize;
        assert!(
            idx < self.mailboxes.len(),
            "ShardMailboxSet::receiver_for: LP {lp} out of range"
        );
        let shared = Arc::clone(&self.mailboxes[idx]);
        let prev = shared.receiver_taken.swap(true, Ordering::AcqRel);
        assert!(
            !prev,
            "ShardMailboxSet::receiver_for: receiver for LP {lp} already taken",
        );
        ShardReceiver { shared, taken: true }
    }

    /// Takes the receiver for the **calling LP**. Convenience wrapper around
    /// [`Self::receiver_for`] that asserts the caller is on a valid LP.
    #[track_caller]
    pub fn receiver_for_current_lp(&self) -> ShardReceiver<M> {
        self.receiver_for(get_lp_id())
    }
}
