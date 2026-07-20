//! Lock-free in-memory debug trace ring buffer.
//!
//! Stores timestamped key-value events without serial-port contention,
//! so the probe does not perturb scheduler timing.  Entries are written
//! atomically with a single monotonically increasing index.
//!
//! Call `dump()` after the events of interest to print the buffer to
//! the serial console.  The serial writes perturb timing, so dump only
//! after the test burst.

use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

const CAPACITY: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TraceEvent {
    pub tick: u64,
    pub tag: u64,
    pub a: u64,
    pub b: u64,
    pub c: u64,
}

#[unsafe(no_mangle)]
#[used]
static mut DEBUG_TRACE: DebugTrace = DebugTrace::new();

pub struct DebugTrace {
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

    pub fn trace(&self, tag: u64, a: u64, b: u64, c: u64) {
        let idx = self.write_idx.fetch_add(1, Ordering::Relaxed) as usize % CAPACITY;
        let tick = read_tick();
        unsafe {
            let slot = &raw const self.buf[idx] as *mut TraceEvent;
            (*slot).tick = tick;
            (*slot).tag = tag;
            (*slot).a = a;
            (*slot).b = b;
            (*slot).c = c;
        }
    }
}

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

pub fn trace(tag: u64, a: u64, b: u64, c: u64) {
    let trace = unsafe { &*core::ptr::addr_of!(DEBUG_TRACE) };
    trace.trace(tag, a, b, c);
}

/// Print the trace buffer to the serial console.  Must be called from
/// thread context — serial writes take the kernel console lock.
pub fn dump() {
    let trace = unsafe { &*core::ptr::addr_of!(DEBUG_TRACE) };
    let total = trace.write_idx.load(Ordering::Relaxed);
    let len = if total > CAPACITY as u64 { CAPACITY } else { total as usize };
    let start = if total > CAPACITY as u64 {
        (total - CAPACITY as u64) as usize % CAPACITY
    } else {
        0
    };

    crate::logln!("[TRACE] {} total events, dumping {} (capacity {})", total, len, CAPACITY);

    for i in 0..len {
        let idx = (start + i) % CAPACITY;
        let e = unsafe {
            let slot = &raw const trace.buf[idx] as *const TraceEvent;
            core::ptr::read_volatile(slot)
        };
        let tag_name = match e.tag {
            TAG_CQ_WAIT_ENTER => "CQ_WAIT_ENTER",
            TAG_CQ_WAIT_RESUME => "CQ_WAIT_RESUME",
            TAG_CQ_WAIT_FAST => "CQ_WAIT_FAST",
            TAG_CQ_WAIT_GUARD => "CQ_WAIT_GUARD",
            TAG_COMPLETE => "COMPLETE",
            TAG_COMPLETE_DETACHED => "COMPLETE_DETACHED",
            TAG_WAKE => "WAKE",
            TAG_SIGNAL_CQ => "SIGNAL_CQ",
            TAG_WAKER_NOTIFY => "WAKER_NOTIFY",
            _ => "?",
        };
        crate::logln!(
            "[TRACE] tick={} {} a={:#x} b={:#x} c={:#x}",
            e.tick, tag_name, e.a, e.b, e.c
        );
    }
    crate::logln!("[TRACE] dump complete.");
}

static DUMP_DELAY_MS: AtomicU64 = AtomicU64::new(0);

/// Spawn a kernel thread that sleeps for `delay_ms` milliseconds,
/// then dumps the trace buffer to the serial console.  Call during boot.
pub fn dump_after(delay_ms: u64) {
    DUMP_DELAY_MS.store(delay_ms, Ordering::Relaxed);

    extern "C" fn dump_trace_thread() {
        let ms = DUMP_DELAY_MS.load(Ordering::Relaxed);
        crate::cpu::scheduler::sleep(
            crate::klib::time::duration::ExtDuration::from_millis(ms as u128),
        );
        dump();
    }

    crate::cpu::scheduler::spawn_thread(crate::memory::KERNEL_ASID, dump_trace_thread);
}

// ---- well-known event tags ----

pub const TAG_CQ_WAIT_ENTER: u64 = 0xc0_0001;
pub const TAG_CQ_WAIT_RESUME: u64 = 0xc0_0002;
pub const TAG_CQ_WAIT_FAST: u64 = 0xc0_0003;
pub const TAG_CQ_WAIT_GUARD: u64 = 0xc0_0004;
pub const TAG_COMPLETE: u64 = 0xc0_0010;
pub const TAG_COMPLETE_DETACHED: u64 = 0xc0_0011;
pub const TAG_WAKE: u64 = 0xc0_0012;
pub const TAG_SIGNAL_CQ: u64 = 0xc0_0013;
pub const TAG_WAKER_NOTIFY: u64 = 0xc0_0020;
