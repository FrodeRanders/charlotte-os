//! CQ payload custody and delivery. Ring publication, FIFO backlog ownership
//! and detached refund extraction share the existing serialized queue state.
//! Caller-owned refund batches survive local registry guards before disposal;
//! wait/source generations retain their existing completion-family contract.
use alloc::collections::VecDeque;

use super::*;

mod preparation;
pub(super) use preparation::{
    Prepared,
    RetiredCq,
    attach,
    open_namespace,
    test_preparation,
};

pub(super) enum BacklogOwner {
    Capability,
    Detached {
        record: RetiredEntry<(OperationId, DetachedOp)>,
    },
}

struct BacklogEntry {
    operation: OperationId,
    cookie: u64,
    status: u32,
    result: i64,
    owner: BacklogOwner,
}

/// One completion queue: the shared ring plus its bounded non-lossy backlog
/// and a monotonic work-generation counter. An address space owns one per
/// shard.
pub(super) struct CqState {
    /// The shared ring (zero-syscall drain path). The allocation backing a
    /// heap-backed ring is kept alive by `_buf`.
    ring: *mut crate::completion::cq::CompletionQueueRing,
    /// Kernel-authoritative producer cursor and capacity. The ring page is
    /// mapped writable to EL0, so `head`/`capacity` in shared memory are only
    /// a publication mirror; the producer must never read them back.
    ring_head: u32,
    ring_capacity: u32,
    /// Entries that could not fit in the shared ring yet. Every entry either
    /// belongs to a live capability or retains a detached operation's live
    /// submission slot, so the address-space capacity also bounds this queue.
    /// Closing a cap-based completion removes its undelivered redundant entry.
    backlog: VecDeque<BacklogEntry>,
    retained_limit: usize,
    /// Monotonic counter bumped every time new work arrives on this queue:
    /// a completion is posted or an explicit wake is posted.  Used by
    /// [`wait_on_cq_checked`] to detect new work without depending on the shared
    /// ring's `pending()` count — callers that poll individual completions
    /// via `poll(cap)` and never drain the ring are not stuck in a busy-spin.
    pub(super) work_generation: u64,
    /// The `work_generation` value consumed by this queue's reactor. Only new
    /// work releases the next wait. Charlotte's per-shard model has one
    /// blocking reactor per CQ; multiple independent waiters would require a
    /// cursor per waiter rather than this queue-wide cursor.
    pub(super) last_seen_generation: u64,
    /// Threads blocked waiting for this queue to become readable.
    pub(super) waiters:
        crate::klib::observer::registration::ListRef<crate::klib::observer::waiter_budget::Charge>,
    #[allow(dead_code)]
    _buf: Option<alloc::vec::Vec<u64>>,
    // Last: free heap backing before returning admission.
    _storage_charge: cq_budget::Charge,
}

impl CqState {
    /// Borrowed raw ring address; ownership and every dereference remain with
    /// the existing registry/mapping/quiescent-consumer contract.
    pub(super) fn ring_ptr(&self) -> *mut cq::CompletionQueueRing {
        self.ring
    }

    pub(super) fn ring_capacity(&self) -> u32 {
        self.ring_capacity
    }

    pub(super) fn producer_head(&self) -> u32 {
        self.ring_head
    }

    pub(super) fn backlog_len(&self) -> usize {
        self.backlog.len()
    }

    pub(super) fn backlog_is_empty(&self) -> bool {
        self.backlog.is_empty()
    }

    pub(super) fn detached_backlog_len(&self) -> usize {
        self.backlog
            .iter()
            .filter(|entry| matches!(&entry.owner, BacklogOwner::Detached { .. }))
            .count()
    }

    /// Only the redundant row for this exact capability/operation may leave.
    /// Detached rows keep their complete node and charge, even on a mismatch.
    pub(super) fn discard_capability(&mut self, operation: OperationId, cap: CompletionCap) {
        self.backlog.retain(|entry| {
            !matches!(&entry.owner, BacklogOwner::Capability)
                || entry.operation != operation
                || entry.cookie != cap
        });
    }

    pub(super) fn post(
        &mut self,
        operation: u64,
        cookie: u64,
        result: &OpResult,
        owner: impl FnOnce() -> BacklogOwner,
        releases: &mut budget::Releases,
    ) -> PostOutcome {
        let (status, val) = crate::completion::cq::op_result_to_fields(result);
        let flushed = self.flush(releases);
        let delivered_current =
            self.backlog.is_empty() && self.push_completion(operation, cookie, status, val);
        if !delivered_current {
            assert!(
                self.backlog.len() < self.retained_limit,
                "completion backlog exceeded address-space submission capacity"
            );
            self.backlog.push_back(BacklogEntry {
                operation,
                cookie,
                status,
                result: val,
                // Transfer admission only if a retained entry needs it. A direct
                // delivery leaves its existing record owner intact for disposal
                // beyond the containing registry guard.
                owner: owner(),
            });
        }
        PostOutcome {
            flushed,
            delivered_current,
        }
    }

    /// Writes as many retained entries as fit, preserving order. Reports how many
    /// entries became visible and how many detached entries therefore released
    /// submission slots.
    pub(super) fn flush(&mut self, releases: &mut budget::Releases) -> FlushOutcome {
        if self.backlog.is_empty() {
            return FlushOutcome::default();
        }

        let mut outcome = FlushOutcome::default();
        while self.ring_has_space() {
            let Some(entry) = self.backlog.pop_front() else {
                break;
            };
            let written =
                self.push_completion(entry.operation, entry.cookie, entry.status, entry.result);
            if !written {
                // A racing shared-tail update made the ring full; retain the
                // entry and retry after the consumer advances.
                self.backlog.push_front(entry);
                break;
            }
            outcome.total += 1;
            if let BacklogOwner::Detached {
                mut record,
            } = entry.owner
            {
                outcome.detached += 1;
                releases
                    .push(record.value_mut().1.record_charge.take().expect("backlog lost charge"));
                releases.push_node(record);
            }
        }
        if outcome.total != 0 {
            // Retained records becoming visible are new work even if the original
            // completion generation was consumed before ring space was available.
            self.work_generation = self.work_generation.wrapping_add(1);
        }
        outcome
    }

    /// Sanitized consumer cursor. EL0 owns `tail` but may hand back garbage;
    /// an out-of-range value is treated as empty so producer arithmetic stays
    /// bounded by the kernel-side capacity.
    fn shared_tail(&self) -> u32 {
        let tail = unsafe { core::ptr::read_volatile(&(*self.ring).tail) };
        if tail < self.ring_capacity {
            tail
        } else {
            self.ring_head
        }
    }

    fn ring_has_space(&self) -> bool {
        (self.ring_head + 1) % self.ring_capacity != self.shared_tail()
    }

    pub(super) fn ring_pending(&self) -> u32 {
        let tail = self.shared_tail();
        if self.ring_head >= tail {
            self.ring_head - tail
        } else {
            self.ring_head + self.ring_capacity - tail
        }
    }

    /// Publishes one entry to the shared ring using the kernel-authoritative
    /// producer cursor. Returns `false` when the ring is full.
    fn push_completion(&mut self, operation: u64, cookie: u64, status: u32, result: i64) -> bool {
        if !self.ring_has_space() {
            let overflow = unsafe { core::ptr::read_volatile(&(*self.ring).overflow) };
            unsafe {
                core::ptr::write_volatile(&mut (*self.ring).overflow, overflow.wrapping_add(1));
            }
            return false;
        }
        let head = self.ring_head;
        let entry = unsafe { &mut *(*self.ring).entry_ptr(head as usize) };
        unsafe {
            core::ptr::write_volatile(&mut entry.operation, operation);
            core::ptr::write_volatile(&mut entry.cookie, cookie);
            core::ptr::write_volatile(&mut entry.status, status);
            core::ptr::write_volatile(&mut entry.flags, 0u32);
            core::ptr::write_volatile(&mut entry.result, result);
        }
        core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
        let next = (head + 1) % self.ring_capacity;
        unsafe {
            core::ptr::write_volatile(&mut (*self.ring).head, next);
        }
        self.ring_head = next;
        true
    }
}

impl Drop for CqState {
    fn drop(&mut self) {
        drop(self.waiters.close());
        while let Some(entry) = self.backlog.pop_front() {
            if let BacklogOwner::Detached {
                record,
            } = entry.owner
            {
                dispose_detached_node(record);
            }
        }
    }
}
/// Posts one entry to a queue's ring, spilling to its non-lossy backlog when
/// the ring is full. Any backlog is flushed first so ordering is preserved.
pub(super) struct PostOutcome {
    pub(super) flushed: FlushOutcome,
    pub(super) delivered_current: bool,
}

#[derive(Clone, Copy, Default)]
pub(super) struct FlushOutcome {
    pub(super) total: usize,
    pub(super) detached: usize,
}
