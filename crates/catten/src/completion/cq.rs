//! # Completion Queue Ring Buffer (shared-memory, io_uring-style)
//! §8.2 richer completion record — 32-byte entries.
//! Field order: operation: u64, cookie: u64, status: u32, flags: u32, result: i64.

use core::sync::atomic::{
    Ordering,
    fence,
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
    pub head: u32,
    pub tail: u32,
    pub capacity: u32,
    pub overflow: u32,
    pub entries: [CqEntry; 0],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CqRingError {
    CapacityTooSmall,
}

impl CompletionQueueRing {
    pub fn new_page(num_entries: u32) -> Result<(alloc::vec::Vec<u8>, *mut Self), CqRingError> {
        if num_entries < 2 {
            return Err(CqRingError::CapacityTooSmall);
        }
        let ps: usize = 4096;
        let hs = 16usize;
        let max = ((ps - hs) / core::mem::size_of::<CqEntry>()) as u32;
        let cap = num_entries.min(max);
        let mut buf = alloc::vec![0u8; ps];
        let ptr = buf.as_mut_ptr() as *mut Self;
        unsafe {
            (*ptr).head = 0;
            (*ptr).tail = 0;
            (*ptr).capacity = cap;
            (*ptr).overflow = 0;
        }
        Ok((buf, ptr))
    }

    pub unsafe fn init_at_phys(frame: PAddr, num_entries: u32) -> Result<*mut Self, CqRingError> {
        if num_entries < 2 {
            return Err(CqRingError::CapacityTooSmall);
        }
        let ps: usize = 4096;
        let hs = 16usize;
        let max = ((ps - hs) / core::mem::size_of::<CqEntry>()) as u32;
        let cap = num_entries.min(max);
        let ptr: *mut Self = frame.into();
        for i in 0..ps {
            unsafe {
                (crate::memory::physical::PAddr::from(frame).into_hhdm_mut::<u8>())
                    .add(i)
                    .write_volatile(0);
            }
        }
        unsafe {
            (*ptr).head = 0;
            (*ptr).tail = 0;
            (*ptr).capacity = cap;
            (*ptr).overflow = 0;
        }
        Ok(ptr)
    }

    pub fn pending(&self) -> u32 {
        core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
        let h = unsafe { core::ptr::read_volatile(&self.head) };
        let t = unsafe { core::ptr::read_volatile(&self.tail) };
        if h >= t {
            h - t
        } else {
            h + self.capacity - t
        }
    }

    /// Consume all pending entries, advancing `tail` to `head`.  Callers
    /// that poll individual completions via `poll(cap)` rather than reading
    /// the shared ring must drain it before calling `cq_wait` again;
    /// otherwise the undrained entries make `pending()` non-zero, causing
    /// `cq_wait` to return immediately without blocking.
    pub unsafe fn drain(&mut self) {
        // SAFETY: &mut self guarantees exclusive access.
        let h = unsafe { core::ptr::read_volatile(&self.head) };
        unsafe { core::ptr::write_volatile(&mut self.tail, h) };
        fence(Ordering::Release);
    }

    pub fn is_full(&self) -> bool {
        core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
        let n = (unsafe { core::ptr::read_volatile(&self.head) } + 1) % self.capacity;
        n == unsafe { core::ptr::read_volatile(&self.tail) }
    }

    pub fn write(&mut self, op: u64, cookie: u64, status: u32, result: i64) -> bool {
        let h = unsafe { core::ptr::read_volatile(&self.head) };
        let t = unsafe { core::ptr::read_volatile(&self.tail) };
        let next = (h + 1) % self.capacity;
        if next == t {
            self.overflow = self.overflow.wrapping_add(1);
            return false;
        }
        let e = self.entry_ptr(h as usize);
        unsafe {
            core::ptr::write_volatile(&mut (*e).operation, op);
            core::ptr::write_volatile(&mut (*e).cookie, cookie);
            core::ptr::write_volatile(&mut (*e).status, status);
            core::ptr::write_volatile(&mut (*e).flags, 0u32);
            core::ptr::write_volatile(&mut (*e).result, result);
        }
        fence(Ordering::Release);
        unsafe {
            core::ptr::write_volatile(&mut self.head, next);
        }
        true
    }

    pub fn write_batch<'a, I>(&mut self, entries: I) -> usize
    where
        I: Iterator<Item = &'a (u64, u64, u32, i64)>,
    {
        let h = unsafe { core::ptr::read_volatile(&self.head) };
        let t = unsafe { core::ptr::read_volatile(&self.tail) };
        let pending = if h >= t {
            h - t
        } else {
            h + self.capacity - t
        };
        let free = (self.capacity - 1 - pending) as usize;
        let mut written: usize = 0;
        for (op, cookie, status, result) in entries.take(free) {
            let slot = ((h as usize) + written) % self.capacity as usize;
            let e = self.entry_ptr(slot);
            unsafe {
                core::ptr::write_volatile(&mut (*e).operation, *op);
                core::ptr::write_volatile(&mut (*e).cookie, *cookie);
                core::ptr::write_volatile(&mut (*e).status, *status);
                core::ptr::write_volatile(&mut (*e).flags, 0u32);
                core::ptr::write_volatile(&mut (*e).result, *result);
            }
            written += 1;
        }
        if written > 0 {
            fence(Ordering::Release);
            let next = ((h as usize + written) % self.capacity as usize) as u32;
            unsafe {
                core::ptr::write_volatile(&mut self.head, next);
            }
        }
        written
    }

    pub fn read(&mut self) -> Option<CqEntry> {
        let h = unsafe { core::ptr::read_volatile(&self.head) };
        let t = unsafe { core::ptr::read_volatile(&self.tail) };
        if h == t {
            return None;
        }
        let e = self.entry_ptr(t as usize);
        fence(Ordering::Acquire);
        let op = unsafe { core::ptr::read_volatile(&(*e).operation) };
        let cookie = unsafe { core::ptr::read_volatile(&(*e).cookie) };
        let status = unsafe { core::ptr::read_volatile(&(*e).status) };
        let r = unsafe { core::ptr::read_volatile(&(*e).result) };
        unsafe {
            core::ptr::write_volatile(&mut self.tail, (t + 1) % self.capacity);
        }
        Some(CqEntry {
            operation: op,
            cookie,
            status,
            flags: 0,
            result: r,
        })
    }

    fn entry_ptr(&self, idx: usize) -> *mut CqEntry {
        let base = self as *const Self as *mut u8;
        let off = core::mem::offset_of!(Self, entries);
        unsafe { base.add(off).add(idx * core::mem::size_of::<CqEntry>()) as *mut CqEntry }
    }
}
