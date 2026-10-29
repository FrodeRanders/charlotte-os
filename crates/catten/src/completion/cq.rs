//! # Completion Queue Ring Buffer (shared-memory, io_uring-style)
//! §8.2 richer completion record — 32-byte entries.
//! Field order: operation: u64, cookie: u64, status: u32, flags: u32, result: i64.

use core::sync::atomic::{
    AtomicU32,
    Ordering,
};

use crate::{
    completion::OpResult,
    cpu::isa::interface::memory::address::PhysicalAddress,
    memory::physical::PAddr,
};

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CqEntry {
    pub operation: u64,
    pub cookie: u64,
    pub status: u32,
    /// Bit 0: reserved for `returned_capability_present` (§8.2). When set,
    /// the consumer should read a `returned_capability: u64` from the
    /// following 8 bytes (the logical entry is then 40 bytes rather than
    /// 32). Not yet wired — capabilities are currently returned through
    /// the IPC reply path.
    pub flags: u32,
    pub result: i64,
}

pub fn op_result_to_fields(r: &OpResult) -> (u32, i64) {
    match r {
        OpResult::Ok(n) => (0, *n),
        OpResult::Err(code) => (1, -(*code as i64)),
        OpResult::Cancelled => (2, 0),
    }
}

pub fn fields_to_op_result(status: u32, result: i64) -> OpResult {
    match status {
        0 => OpResult::Ok(result),
        1 => OpResult::Err((-result) as i32),
        _ => OpResult::Cancelled,
    }
}

/// Legacy helpers retained for existing callers.
pub fn op_result_to_i64(r: OpResult) -> i64 {
    let (_, v) = op_result_to_fields(&r);
    v
}
pub fn i64_to_op_result(r: i64) -> OpResult {
    if r == i64::MIN {
        OpResult::Cancelled
    } else if r >= 0 {
        OpResult::Ok(r)
    } else {
        OpResult::Err((-r) as i32)
    }
}

#[repr(C)]
pub struct CompletionQueueRing {
    head: AtomicU32,
    tail: AtomicU32,
    capacity: AtomicU32,
    overflow: AtomicU32,
    entries: [CqEntry; 0],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CqRingError {
    CapacityTooSmall,
    AllocationFailed,
    FrameMisaligned,
}

impl CompletionQueueRing {
    /// Entry capacity of a 4 KiB ring page for the requested entry count. The
    /// shared `capacity` field mirrors this value for userspace, but the
    /// kernel's producer state is authoritative and must never read it back.
    pub const fn capacity_for(num_entries: u32) -> u32 {
        let max = ((4096 - core::mem::size_of::<Self>()) / core::mem::size_of::<CqEntry>()) as u32;
        if num_entries < max {
            num_entries
        } else {
            max
        }
    }

    /// u64 backing preserves the alignment required by the header and entries.
    pub fn new_page(num_entries: u32) -> Result<(alloc::vec::Vec<u64>, *mut Self), CqRingError> {
        if num_entries < 2 {
            return Err(CqRingError::CapacityTooSmall);
        }
        let cap = Self::capacity_for(num_entries);
        let mut buf = alloc::vec::Vec::new();
        buf.try_reserve_exact(512).map_err(|_| CqRingError::AllocationFailed)?;
        if buf.capacity() > 512 {
            return Err(CqRingError::AllocationFailed);
        }
        buf.resize(512, 0u64);
        let ptr = buf.as_mut_ptr() as *mut Self;
        // SAFETY: aligned, exclusively owned full-page backing is initialized
        // before the ring is published to any producer or consumer.
        unsafe {
            (*ptr).head = AtomicU32::new(0);
            (*ptr).tail = AtomicU32::new(0);
            (*ptr).capacity = AtomicU32::new(cap);
            (*ptr).overflow = AtomicU32::new(0);
        }
        Ok((buf, ptr))
    }

    /// # Safety
    ///
    /// `frame` must identify an exclusively owned, writable 4 KiB physical
    /// frame that remains allocated for the returned ring's lifetime.
    pub unsafe fn init_at_phys(frame: PAddr, num_entries: u32) -> Result<*mut Self, CqRingError> {
        if num_entries < 2 {
            return Err(CqRingError::CapacityTooSmall);
        }
        if !crate::cpu::isa::interface::memory::address::Address::is_aligned_to(&frame, 4096) {
            return Err(CqRingError::FrameMisaligned);
        }
        let cap = Self::capacity_for(num_entries);
        let ptr: *mut Self = frame.into();
        for i in 0..4096 {
            // SAFETY: caller grants exclusive access to this aligned 4 KiB frame.
            unsafe {
                (frame.into_hhdm_mut::<u8>()).add(i).write_volatile(0);
            }
        }
        // SAFETY: aligned, exclusively owned full-page backing is initialized
        // before the ring is published to any producer or consumer.
        unsafe {
            (*ptr).head = AtomicU32::new(0);
            (*ptr).tail = AtomicU32::new(0);
            (*ptr).capacity = AtomicU32::new(cap);
            (*ptr).overflow = AtomicU32::new(0);
        }
        Ok(ptr)
    }

    /// Diagnostic mirror only. Production uses CqState's private capacity.
    pub fn capacity(&self) -> u32 {
        self.capacity.load(Ordering::Relaxed)
    }

    pub fn published_head(&self) -> u32 {
        self.head.load(Ordering::Acquire)
    }

    pub fn overflow_count(&self) -> u32 {
        self.overflow.load(Ordering::Relaxed)
    }

    /// Acquire the consumer's release before reusing a slot it has finished.
    pub(super) fn consumer_tail(&self) -> u32 {
        self.tail.load(Ordering::Acquire)
    }

    /// Caller has initialized every entry up to this private producer cursor.
    pub(super) fn publish_head(&self, head: u32) {
        self.head.store(head, Ordering::Release);
    }

    pub(super) fn record_overflow(&self) {
        self.overflow.fetch_add(1, Ordering::Relaxed);
    }

    pub fn pending(&self) -> u32 {
        let head = self.published_head();
        let tail = self.consumer_tail();
        let capacity = self.capacity();
        if capacity < 2 || head >= capacity || tail >= capacity {
            return 0;
        }
        if head >= tail {
            head - tail
        } else {
            head + capacity - tail
        }
    }

    /// Discard the currently published notifications, not operation owners.
    ///
    /// # Safety
    /// No other agent may concurrently consume this ring or update its tail.
    /// The backing allocation must remain live through the operation.
    pub unsafe fn drain(&mut self) {
        let head = self.published_head();
        self.tail.store(head, Ordering::Release);
    }

    pub fn is_full(&self) -> bool {
        let capacity = self.capacity();
        let head = self.head.load(Ordering::Relaxed);
        capacity >= 2 && head < capacity && (head + 1) % capacity == self.consumer_tail()
    }

    /// Serialized fixture producer. Runtime delivery uses its private cursor
    /// and capacity, never these writable shared mirrors.
    pub fn write(&mut self, op: u64, cookie: u64, status: u32, result: i64) -> bool {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.consumer_tail();
        let capacity = self.capacity();
        if capacity < 2 || head >= capacity || tail >= capacity {
            return false;
        }
        let next = (head + 1) % capacity;
        if next == tail {
            self.record_overflow();
            return false;
        }
        // SAFETY: this full-page ring has one serialized producer. Acquire of
        // tail establishes that the consumer finished the free slot; head is
        // released only after all payload fields have been initialized.
        unsafe {
            self.entry_ptr(head as usize).write_volatile(CqEntry {
                operation: op,
                cookie,
                status,
                flags: 0,
                result,
            });
        }
        self.publish_head(next);
        true
    }

    pub fn write_batch<'a, I>(&mut self, entries: I) -> usize
    where
        I: Iterator<Item = &'a (u64, u64, u32, i64)>,
    {
        let head = self.head.load(Ordering::Relaxed);
        let tail = self.consumer_tail();
        let capacity = self.capacity();
        if capacity < 2 || head >= capacity || tail >= capacity {
            return 0;
        }
        let pending = if head >= tail {
            head - tail
        } else {
            head + capacity - tail
        };
        let free = (capacity - 1 - pending) as usize;
        let mut written = 0;
        for &(operation, cookie, status, result) in entries.take(free) {
            let slot = (head as usize + written) % capacity as usize;
            // SAFETY: the acquired tail bounds this batch to free slots in
            // the full-page backing. No other producer may write these slots.
            unsafe {
                self.entry_ptr(slot).write_volatile(CqEntry {
                    operation,
                    cookie,
                    status,
                    flags: 0,
                    result,
                });
            }
            written += 1;
        }
        if written != 0 {
            self.publish_head((head + written as u32) % capacity);
        }
        written
    }

    /// Serialized fixture consumer. Published queues are consumed by their
    /// owning EL0 reactor, not by simultaneous kernel inspection readers.
    pub fn read(&mut self) -> Option<CqEntry> {
        let head = self.published_head();
        let tail = self.tail.load(Ordering::Relaxed);
        let capacity = self.capacity();
        if capacity < 2 || head >= capacity || tail >= capacity || head == tail {
            return None;
        }
        // SAFETY: the acquire head load observes the complete initialized
        // entry. The sole consumer finishes copying before releasing tail.
        let entry = unsafe { self.entry_ptr(tail as usize).read_volatile() };
        self.tail.store((tail + 1) % capacity, Ordering::Release);
        Some(entry)
    }

    pub(crate) fn entry_ptr(&self, idx: usize) -> *mut CqEntry {
        let base = self as *const Self as *mut u8;
        base.wrapping_add(core::mem::offset_of!(Self, entries))
            .wrapping_add(idx.wrapping_mul(core::mem::size_of::<CqEntry>()))
            .cast()
    }
}

const _: () = {
    assert!(core::mem::size_of::<CompletionQueueRing>() == 16);
    assert!(core::mem::offset_of!(CompletionQueueRing, entries) == 16);
    assert!(core::mem::size_of::<CqEntry>() == 32);
};
