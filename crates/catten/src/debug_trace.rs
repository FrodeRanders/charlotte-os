//! Lock-free in-memory debug trace ring buffer.
//!
//! Stores timestamped key-value events without serial-port contention,
//! so the probe does not perturb scheduler timing.  Entries are written
//! atomically with a single monotonically increasing index; the buffer
//! is read post-mortem via GDB by inspecting the `DEBUG_TRACE` symbol.
//!
//! Capacity: 4096 entries (~80 KiB).  If the buffer fills, the oldest
//! entries are overwritten (ring-buffer semantics).

use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

const CAPACITY: usize = 4096;

/// One trace event: a small tag plus up to three u64 payloads.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TraceEvent {
    /// Monotonic tick counter at the time of the trace.  On AArch64 this
    /// is the raw value of the ARM Generic Timer's physical counter
    /// (CNTPCT_EL0), which runs at 62.5 MHz and never stops.
    pub tick: u64,
    /// Event tag — see constants below.
    pub tag: u64,
    pub a: u64,
    pub b: u64,
    pub c: u64,
}

/// Global trace ring, laid out in .bss so its address is stable and
/// inspectable from the debugger without any indirection.
#[unsafe(no_mangle)]
#[used]
static mut DEBUG_TRACE: DebugTrace = DebugTrace::new();

pub struct DebugTrace {
    /// Monotonically increasing write index.  Loops at u64::MAX.
    write_idx: AtomicU64,
    buf: [TraceEvent; CAPACITY],
}

impl DebugTrace {
    pub const fn new() -> Self {
        Self {
            write_idx: AtomicU64::new(0),
            buf: [TraceEvent { tick: 0, tag: 0, a: 0, b: 0, c: 0 }; CAPACITY],
        }
    }

    /// Append a trace event.  Lock-free: uses atomic fetch-add for the
    /// index.  On buffer wrap, the oldest entries are silently overwritten.
    pub fn trace(&self, tag: u64, a: u64, b: u64, c: u64) {
        let idx = self.write_idx.fetch_add(1, Ordering::Relaxed) as usize % CAPACITY;
        let tick = read_tick();
        // SAFETY: the write_idx is atomically incremented; each writer gets
        // a unique slot.  The read side (debugger) may observe partially
        // written entries at the current write position — acceptable for
        // an investigative trace.
        unsafe {
            let slot = &raw const self.buf[idx] as *mut TraceEvent;
            (*slot).tick = tick;
            (*slot).tag = tag;
            (*slot).a = a;
            (*slot).b = b;
            (*slot).c = c;
        }
    }

    /// Total events written so far (may wrap at u64::MAX).
    pub fn count(&self) -> u64 {
        self.write_idx.load(Ordering::Relaxed)
    }
}

/// Read the ARM Generic Timer physical counter.
#[cfg(target_arch = "aarch64")]
fn read_tick() -> u64 {
    let tick: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntpct_el0", out(reg) tick, options(nomem, nostack, preserves_flags));
    }
    tick
}

#[cfg(not(target_arch = "aarch64"))]
fn read_tick() -> u64 {
    0
}

// ---- trace point helpers ----

#[allow(dead_code)]
pub fn trace(tag: u64, a: u64, b: u64, c: u64) {
    let trace = unsafe { &*core::ptr::addr_of!(DEBUG_TRACE) };
    trace.trace(tag, a, b, c);
}

// ---- well-known event tags ----

/// Tag: wait_on_cq entry (fast-path check done, about to block).
///  a = asid,  b = work_generation,  c = last_seen_generation
pub const TAG_CQ_WAIT_ENTER: u64 = 0xc0_0001;

/// Tag: wait_on_cq resume (woke from yield, about to check generation).
///  a = asid,  b = work_generation,  c = last_seen_generation
pub const TAG_CQ_WAIT_RESUME: u64 = 0xc0_0002;

/// Tag: wait_on_cq fast-path return (no block needed).
///  a = asid,  b = work_generation,  c = last_seen_generation
pub const TAG_CQ_WAIT_FAST: u64 = 0xc0_0003;

/// Tag: wait_on_cq lost-wake guard fired.
///  a = asid,  b = work_generation,  c = last_seen_generation
pub const TAG_CQ_WAIT_GUARD: u64 = 0xc0_0004;

/// Tag: complete() bumping generation.
///  a = asid,  b = new work_generation,  c = cap
pub const TAG_COMPLETE: u64 = 0xc0_0010;

/// Tag: complete_detached() bumping generation.
///  a = asid,  b = new work_generation,  c = operation
pub const TAG_COMPLETE_DETACHED: u64 = 0xc0_0011;

/// Tag: wake() bumping generation.
///  a = asid,  b = new work_generation,  c = cq
pub const TAG_WAKE: u64 = 0xc0_0012;

/// Tag: signal_cq() called.
///  a = asid,  b = cq,  c = observer_count
pub const TAG_SIGNAL_CQ: u64 = 0xc0_0013;

/// Tag: Waker::notify() called (thread being re-admitted).
///  a = tid,  b = generation,  c = 0
pub const TAG_WAKER_NOTIFY: u64 = 0xc0_0020;
