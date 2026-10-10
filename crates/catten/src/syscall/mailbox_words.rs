//! Fixed word rings. Queue access is already lifecycle-serialized; these short
//! IRQ-state-preserving per-LP holds also cover detached boot-fixture reads.
use alloc::vec::Vec;

use super::{
    LpId,
    get_lp_count,
    get_lp_id,
    mailbox_budget,
};
use crate::cpu::multiprocessor::{
    ipi,
    spin::mutex::Mutex,
};

const CAPACITY: usize = 256;
pub(super) struct Ring {
    words: [u64; CAPACITY],
    head: usize,
    len: usize,
}
impl Ring {
    const fn new() -> Self {
        Self {
            words: [0; CAPACITY],
            head: 0,
            len: 0,
        }
    }

    fn push(&mut self, word: u64) -> Result<(), u64> {
        if self.len == CAPACITY {
            return Err(word);
        }
        self.words[(self.head + self.len) % CAPACITY] = word;
        self.len += 1;
        Ok(())
    }

    fn pop(&mut self) -> Option<u64> {
        if self.len == 0 {
            return None;
        }
        let word = self.words[self.head];
        self.head = (self.head + 1) % CAPACITY;
        self.len -= 1;
        Some(word)
    }
}
/// Field order releases the requested ring allocation before its original
/// charge. No queue Arc/Weak or durable sender escapes this word ABI.
pub(super) struct Words {
    rings: Vec<Mutex<Ring>>,
    _charge: mailbox_budget::QueueCharge,
}
impl Words {
    pub(super) fn backing_bytes() -> Result<usize, mailbox_budget::Error> {
        Self::backing_bytes_for(get_lp_count() as usize)
    }

    pub(super) fn backing_bytes_for(n: usize) -> Result<usize, mailbox_budget::Error> {
        n.checked_mul(core::mem::size_of::<Mutex<Ring>>())
            .ok_or(mailbox_budget::Error::ResourceLimit)
    }

    /// Borrow the preparation's charge until every fallible step ends; an
    /// allocation rejection must not refund before its partial backing drops.
    pub(super) fn prepare(n: usize) -> Result<Vec<Mutex<Ring>>, mailbox_budget::Error> {
        let mut rings = Vec::new();
        rings.try_reserve_exact(n).map_err(|_| mailbox_budget::Error::AllocationFailed)?;
        // Global's exact reservation uses this requested layout. Reject any
        // future excess capacity before publication rather than undercharge it.
        if rings.capacity() != n {
            return Err(mailbox_budget::Error::AllocationFailed);
        }
        for _ in 0..n {
            rings.push(Mutex::new(Ring::new()));
        }
        Ok(rings)
    }

    pub(super) fn new(rings: Vec<Mutex<Ring>>, charge: mailbox_budget::QueueCharge) -> Self {
        Self {
            rings,
            _charge: charge,
        }
    }

    pub(super) fn try_send_to(&self, target: LpId, message: u64) -> Result<(), u64> {
        let Some(ring) = self.rings.get(target as usize) else {
            return Err(message);
        };
        ring.lock().push(message)?;
        // The ring guard leaves before notifying the target LP.
        ipi::send_ipi(target);
        Ok(())
    }

    pub(super) fn try_recv_for_current_lp(&self) -> Option<u64> {
        self.rings.get(get_lp_id() as usize)?.lock().pop()
    }
}
