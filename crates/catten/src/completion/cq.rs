//! # Completion Queue Ring Buffer (shared-memory, io_uring-style)
//!
//! A single-producer (kernel), single-consumer (userspace) ring for zero-syscall
//! completion delivery. The producer advances the head; the consumer advances
//! the tail. When head == tail the queue is empty; when `(head + 1) % capacity
//! == tail` the queue is full.
//!
//! ## Layout (single 4 KiB page)
//!
//! ```text
//! offset  size  field
//! 0x000   4     head (kernel writes, userspace reads)
//! 0x004   4     tail (userspace writes, kernel reads)
//! 0x008   4     capacity (N entries, immutable after init)
//! 0x00C   4     overflow (count of producer writes that found the ring full)
//! 0x010   N*16  entry array
//! ```
//!
//! Each entry is 16 bytes: `cap: u64, result: i64`.
//!
//! The ring is allocated from the kernel heap (`alloc::vec!`) and is
//! simultaneously accessible via HHDM (kernel) and, once mapped, a user virtual
//! address. For the self-test we drain from the kernel side; in production the
//! consumer runs in userspace.

use core::sync::atomic::{
    Ordering,
    fence,
};

use crate::{
    completion::{
        CompletionCap,
        OpResult,
    },
    memory::physical::PAddr,
};

/// One completion entry in the ring (16 bytes).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CqEntry {
    /// The capability this completion is for.
    pub cap: u64,
    /// The result: `>= 0` = Ok(bytes), `< 0` = Err(-code), `i64::MIN` = Cancelled.
    pub result: i64,
}

/// In-memory representation of the ring header + entry array.
///
/// Allocated as a boxed slice on the heap. Both the kernel (HHDM) and a
/// mapped user page reference the same physical memory.
#[repr(C)]
pub struct CompletionQueueRing {
    /// Producer index. Advanced by the kernel after writing an entry.
    pub head: u32,
    /// Consumer index. Advanced by userspace after reading an entry.
    pub tail: u32,
    /// Fixed number of entry slots.
    pub capacity: u32,
    /// Cumulative count of producer writes that found the shared ring full.
    /// `completion::complete()` retains those entries in a kernel backlog and
    /// retries later, so this is pressure telemetry, not a drop count.
    pub overflow: u32,
    /// Entry array. `capacity` entries, indexed modulo.
    pub entries: [CqEntry; 0],
}

impl CompletionQueueRing {
    /// Returns a `(Vec<u8>, &mut Self)` where the Vec owns the allocation and
    /// the reference points into it. `num_entries` is the requested number of
    /// slots; the actual capacity may be rounded down to fit in a page.
    ///
    /// The returned buffer is page-aligned and one 4 KiB page in size.
    pub fn new_page(num_entries: u32) -> (alloc::vec::Vec<u8>, *mut Self) {
        let page_size = 4096;
        let header_size = core::mem::size_of::<u32>() * 4;
        let max_entries = ((page_size - header_size) / core::mem::size_of::<CqEntry>()) as u32;
        let capacity = num_entries.min(max_entries);

        let mut buf = alloc::vec![0u8; page_size];
        let ptr = buf.as_mut_ptr() as *mut Self;

        unsafe {
            (*ptr).head = 0;
            (*ptr).tail = 0;
            (*ptr).capacity = capacity;
            (*ptr).overflow = 0;
        }

        (buf, ptr)
    }

    /// Initializes a `CompletionQueueRing` on a pre-allocated physical frame.
    /// The frame must be at least one 4 KiB page. The ring is accessed through
    /// the HHDM window; the caller is responsible for mapping the same physical
    /// frame into the target address space for userspace access.
    ///
    /// # Safety
    ///
    /// `frame` must be a valid, 4 KiB-aligned physical address that is not used
    /// for any other purpose while this ring is alive.
    pub unsafe fn init_at_phys(frame: PAddr, num_entries: u32) -> *mut Self {
        let page_size = 4096;
        let header_size = core::mem::size_of::<u32>() * 4;
        let max_entries = ((page_size - header_size) / core::mem::size_of::<CqEntry>()) as u32;
        let capacity = num_entries.min(max_entries);

        let ptr: *mut Self = frame.into();

        // Zero the entire page first (clean HHDM memory).
        let byte_ptr: *mut u8 = frame.into();
        unsafe {
            for i in 0..page_size {
                byte_ptr.add(i).write_volatile(0);
            }

            (*ptr).head = 0;
            (*ptr).tail = 0;
            (*ptr).capacity = capacity;
            (*ptr).overflow = 0;
        }

        ptr
    }

    /// Returns the number of pending (un-consumed) entries.
    pub fn pending(&self) -> u32 {
        let h = unsafe { core::ptr::read_volatile(&self.head) };
        let t = unsafe { core::ptr::read_volatile(&self.tail) };
        if h >= t {
            h - t
        } else {
            h + self.capacity - t
        }
    }

    /// Whether the ring is full.
    pub fn is_full(&self) -> bool {
        let next = (unsafe { core::ptr::read_volatile(&self.head) } + 1) % self.capacity;
        next == unsafe { core::ptr::read_volatile(&self.tail) }
    }

    /// Kernel (producer) side: write one completion entry. If the ring is full,
    /// the write is skipped and the overflow counter is incremented. The caller
    /// is responsible for retaining the entry and retrying later.
    pub fn write(&mut self, cap: CompletionCap, result: OpResult) -> bool {
        let result_code = op_result_to_i64(result);
        let h = unsafe { core::ptr::read_volatile(&self.head) };
        let t = unsafe { core::ptr::read_volatile(&self.tail) };
        let next = (h + 1) % self.capacity;
        if next == t {
            // Ring is full. The completion layer keeps the entry in its
            // per-address-space backlog and retries on a later completion.
            self.overflow = self.overflow.wrapping_add(1);
            return false;
        }

        let entry_ptr = self.entry_ptr(h as usize);
        unsafe {
            core::ptr::write_volatile(&mut (*entry_ptr).cap, cap as u64);
            core::ptr::write_volatile(&mut (*entry_ptr).result, result_code);
        }
        fence(Ordering::Release);
        unsafe { core::ptr::write_volatile(&mut self.head, next) };
        true
    }

    /// Consumer side: drain one completion entry. Returns `None` when empty.
    pub fn read(&mut self) -> Option<CqEntry> {
        let h = unsafe { core::ptr::read_volatile(&self.head) };
        let t = unsafe { core::ptr::read_volatile(&self.tail) };
        if h == t {
            return None;
        }

        let entry_ptr = self.entry_ptr(t as usize);
        fence(Ordering::Acquire);
        let cap = unsafe { core::ptr::read_volatile(&(*entry_ptr).cap) };
        let result = unsafe { core::ptr::read_volatile(&(*entry_ptr).result) };
        let next = (t + 1) % self.capacity;
        unsafe { core::ptr::write_volatile(&mut self.tail, next) };
        Some(CqEntry {
            cap,
            result,
        })
    }

    fn entry_ptr(&self, idx: usize) -> *mut CqEntry {
        let base = self as *const Self as *mut u8;
        let entries_offset = core::mem::offset_of!(Self, entries);
        let entry_size = core::mem::size_of::<CqEntry>();
        unsafe { base.add(entries_offset).add(idx * entry_size) as *mut CqEntry }
    }
}

pub fn op_result_to_i64(r: OpResult) -> i64 {
    match r {
        OpResult::Ok(n) => n,
        OpResult::Err(code) => -(code as i64),
        OpResult::Cancelled => i64::MIN,
    }
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

#[cfg(test)]
#[allow(unused_imports)]
mod tests {
    // Tests are in self_test/completion_cq.rs (boot-time); kept here as
    // documentation of the expected contract.
}
