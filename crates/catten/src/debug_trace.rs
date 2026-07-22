//! Bounded lock-free scheduler diagnostic trace.
//!
//! Hot paths only write atomic words into memory. Serial output is deferred
//! until after the failure window so tracing does not serialize the scheduler.

#[cfg(feature = "scheduler_trace")]
use core::sync::atomic::{
    AtomicU64,
    Ordering,
};

#[cfg(feature = "scheduler_trace")]
const CAPACITY: usize = 16_384;

#[repr(C)]
#[cfg(feature = "scheduler_trace")]
struct TraceSlot {
    sequence: AtomicU64,
    tick: AtomicU64,
    tag: AtomicU64,
    lp: AtomicU64,
    a: AtomicU64,
    b: AtomicU64,
    c: AtomicU64,
}

#[cfg(feature = "scheduler_trace")]
impl TraceSlot {
    const fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            tick: AtomicU64::new(0),
            tag: AtomicU64::new(0),
            lp: AtomicU64::new(0),
            a: AtomicU64::new(0),
            b: AtomicU64::new(0),
            c: AtomicU64::new(0),
        }
    }
}

#[unsafe(no_mangle)]
#[used]
#[cfg(feature = "scheduler_trace")]
static DEBUG_TRACE: DebugTrace = DebugTrace::new();

#[cfg(feature = "scheduler_trace")]
#[repr(C)]
struct DebugTrace {
    write_index: AtomicU64,
    slots: [TraceSlot; CAPACITY],
}

#[cfg(feature = "scheduler_trace")]
impl DebugTrace {
    const fn new() -> Self {
        Self {
            write_index: AtomicU64::new(0),
            slots: [const { TraceSlot::new() }; CAPACITY],
        }
    }

    fn push(&self, tag: u64, a: u64, b: u64, c: u64) {
        let logical = self.write_index.fetch_add(1, Ordering::Relaxed);
        let slot = &self.slots[logical as usize % CAPACITY];
        let committed = logical.wrapping_mul(2).wrapping_add(2);
        slot.sequence.store(committed - 1, Ordering::Relaxed);
        slot.tick.store(read_tick(), Ordering::Relaxed);
        slot.tag.store(tag, Ordering::Relaxed);
        slot.lp.store(crate::cpu::isa::lp::ops::get_lp_id() as u64, Ordering::Relaxed);
        slot.a.store(a, Ordering::Relaxed);
        slot.b.store(b, Ordering::Relaxed);
        slot.c.store(c, Ordering::Relaxed);
        slot.sequence.store(committed, Ordering::Release);
    }
}

#[cfg(target_arch = "aarch64")]
#[cfg(feature = "scheduler_trace")]
fn read_tick() -> u64 {
    let tick: u64;
    unsafe {
        core::arch::asm!("mrs {}, cntvct_el0", out(reg) tick, options(nomem, nostack, preserves_flags));
    }
    tick
}

#[cfg(not(target_arch = "aarch64"))]
#[cfg(feature = "scheduler_trace")]
fn read_tick() -> u64 {
    0
}

#[inline]
#[cfg(feature = "scheduler_trace")]
pub fn trace(tag: u64, a: u64, b: u64, c: u64) {
    DEBUG_TRACE.push(tag, a, b, c);
}

#[inline]
#[cfg(not(feature = "scheduler_trace"))]
pub fn trace(_tag: u64, _a: u64, _b: u64, _c: u64) {}

#[cfg(feature = "scheduler_trace")]
pub fn dump() {
    let total = DEBUG_TRACE.write_index.load(Ordering::Acquire);
    let retained = total.min(CAPACITY as u64);
    let first = total - retained;
    crate::logln!("[TRACE] total={} retained={}", total, retained);
    for logical in first..total {
        let slot = &DEBUG_TRACE.slots[logical as usize % CAPACITY];
        let expected = logical.wrapping_mul(2).wrapping_add(2);
        if slot.sequence.load(Ordering::Acquire) != expected {
            continue;
        }
        crate::logln!(
            "[TRACE] tick={} lp={} {} a={:#x} b={:#x} c={:#x}",
            slot.tick.load(Ordering::Relaxed),
            slot.lp.load(Ordering::Relaxed),
            tag_name(slot.tag.load(Ordering::Relaxed)),
            slot.a.load(Ordering::Relaxed),
            slot.b.load(Ordering::Relaxed),
            slot.c.load(Ordering::Relaxed)
        );
    }
    crate::logln!("[TRACE] dump complete");
}

#[cfg(feature = "scheduler_trace")]
static DUMP_DELAY_MS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "scheduler_trace")]
pub fn dump_after(delay_ms: u64) {
    DUMP_DELAY_MS.store(delay_ms, Ordering::Release);
    extern "C" fn dump_thread() {
        crate::cpu::scheduler::sleep_millis(DUMP_DELAY_MS.load(Ordering::Acquire));
        dump();
        crate::cpu::scheduler::abort();
    }
    crate::cpu::scheduler::spawn_thread(crate::memory::KERNEL_ASID, dump_thread);
}

#[cfg(not(feature = "scheduler_trace"))]
pub fn dump_after(_delay_ms: u64) {}

#[cfg(feature = "scheduler_trace")]
fn tag_name(tag: u64) -> &'static str {
    match tag {
        TAG_CQ_WAIT_ENTER => "CQ_WAIT_ENTER",
        TAG_CQ_WAIT_RESUME => "CQ_WAIT_RESUME",
        TAG_CQ_WAIT_FAST => "CQ_WAIT_FAST",
        TAG_CQ_WAIT_GUARD => "CQ_WAIT_GUARD",
        TAG_COMPLETE => "COMPLETE",
        TAG_COMPLETE_DETACHED => "COMPLETE_DETACHED",
        TAG_WAKE => "WAKE",
        TAG_SIGNAL_CQ => "SIGNAL_CQ",
        TAG_WAKER_NOTIFY => "WAKER_NOTIFY",
        TAG_SUBMIT_TIMER_OK => "SUBMIT_TIMER_OK",
        TAG_TIMER_FIRED => "TIMER_FIRED",
        TAG_TIMER_ARMED => "TIMER_ARMED",
        TAG_TIMER_STOPPED => "TIMER_STOPPED",
        TAG_SCHED_DISPATCH => "SCHED_DISPATCH",
        TAG_SCHED_ADMIT => "SCHED_ADMIT",
        TAG_STACK_ARENA_WAIT => "STACK_ARENA_WAIT",
        TAG_STACK_ARENA_ACQUIRED => "STACK_ARENA_ACQUIRED",
        TAG_STACK_ARENA_RELEASED => "STACK_ARENA_RELEASED",
        TAG_DEVICE_PHASE => "DEVICE_PHASE",
        _ => "?",
    }
}

pub const TAG_CQ_WAIT_ENTER: u64 = 0xc0_0001;
pub const TAG_CQ_WAIT_RESUME: u64 = 0xc0_0002;
pub const TAG_CQ_WAIT_FAST: u64 = 0xc0_0003;
pub const TAG_CQ_WAIT_GUARD: u64 = 0xc0_0004;
pub const TAG_COMPLETE: u64 = 0xc0_0010;
pub const TAG_COMPLETE_DETACHED: u64 = 0xc0_0011;
pub const TAG_WAKE: u64 = 0xc0_0012;
pub const TAG_SIGNAL_CQ: u64 = 0xc0_0013;
pub const TAG_WAKER_NOTIFY: u64 = 0xc0_0020;
pub const TAG_SUBMIT_TIMER_OK: u64 = 0xc0_0030;
pub const TAG_TIMER_FIRED: u64 = 0xc0_0031;
pub const TAG_TIMER_ARMED: u64 = 0xc0_0032;
pub const TAG_TIMER_STOPPED: u64 = 0xc0_0033;
pub const TAG_SCHED_DISPATCH: u64 = 0xc0_0040;
pub const TAG_SCHED_ADMIT: u64 = 0xc0_0041;
pub const TAG_STACK_ARENA_WAIT: u64 = 0xc0_0050;
pub const TAG_STACK_ARENA_ACQUIRED: u64 = 0xc0_0051;
pub const TAG_STACK_ARENA_RELEASED: u64 = 0xc0_0052;
pub const TAG_DEVICE_PHASE: u64 = 0xc0_0060;
