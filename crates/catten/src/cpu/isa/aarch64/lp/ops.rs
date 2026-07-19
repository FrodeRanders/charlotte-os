#[macro_export]
macro_rules! await_interrupt {
    () => {
        loop {
            unsafe {
                core::arch::asm!(
                    "msr daifclr, 0b1111",
                    "wfi",
                    options(nomem, nostack, preserves_flags)
                );
            }
        }
    };
}
pub use await_interrupt;
#[macro_export]
macro_rules! mask_interrupts {
    () => {
        unsafe { core::arch::asm!("msr daifset, 0b1111", options(nomem, nostack)) }
    };
}
pub use mask_interrupts;
#[macro_export]
macro_rules! unmask_interrupts {
    () => {
        unsafe { core::arch::asm!("msr daifclr, 0b1111", options(nomem, nostack)) }
    };
}
pub use unmask_interrupts;

/// Returns `true` if IRQs are currently unmasked (enabled) on the calling
/// logical processor. The DAIF `I` bit (bit 7) is set when IRQs are masked, so
/// interrupts are enabled when it is clear.
#[inline(always)]
pub fn get_int_state() -> bool {
    let daif: u64;
    unsafe {
        core::arch::asm!(
            "mrs {daif}, daif",
            daif = out(reg) daif,
            options(nomem, nostack, preserves_flags)
        );
    }
    daif & (1 << 7) == 0
}

use core::arch::naked_asm;

use crate::cpu::isa::lp::LpId;
use crate::cpu::scheduler::system_scheduler::SYSTEM_SCHEDULER;
use crate::cpu::scheduler::threads::MASTER_THREAD_TABLE;
use crate::logln;
use crate::memory::VAddr;

/// Enable Advanced SIMD and floating-point instruction access at EL1 (and EL0).
///
/// The kernel is compiled with the `+neon` feature, so the compiler freely
/// emits FP/SIMD instructions (for `memcpy`, formatting, etc.). Those trap as
/// "undefined instruction" unless `CPACR_EL1.FPEN` permits them. Limine leaves
/// FP/SIMD access trapped, so this must run before any Rust code that could use
/// those registers — i.e. as the very first thing on each logical processor.
#[inline(always)]
pub fn enable_fp_simd() {
    unsafe {
        // Ensure the kernel executes at EL1h (using SP_ELx) rather than EL1t
        // (using SP_EL0). Some entry paths may hand control over in EL1t; if we
        // stayed there, an interrupt taken in kernel code would push state onto
        // SP_EL0, which we do not maintain as a valid kernel stack. We copy the
        // current stack pointer into SP_EL1 before selecting it so the switch
        // does not lose the stack.
        core::arch::asm!(
            "mov {tmp}, sp",     // capture the currently active SP (SP_EL0 if EL1t)
            "msr spsel, #1",     // select SP_EL1 as the active stack pointer
            "mov sp, {tmp}",     // point SP_EL1 at the same stack we were using
            tmp = out(reg) _,
            options(preserves_flags)
        );
        // CPACR_EL1.FPEN = 0b11: do not trap FP/SIMD at EL0 or EL1.
        core::arch::asm!(
            "mrs {tmp}, cpacr_el1",
            "orr {tmp}, {tmp}, #(0b11 << 20)",
            "msr cpacr_el1, {tmp}",
            "isb",
            tmp = out(reg) _,
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub fn store_lp_id(lp_id: LpId) {
    unsafe {
        core::arch::asm!(
            "msr tpidr_el1, {lp_id:x}",
            lp_id = in(reg) lp_id as u64,
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub fn get_lp_id() -> LpId {
    let lp_id: u64;
    unsafe {
        core::arch::asm!(
            "mrs {lp_id:x}, tpidr_el1",
            lp_id = out(reg) lp_id,
            options(nomem, nostack, preserves_flags)
        );
    }
    lp_id as LpId
}

pub fn get_lic_id() -> u32 {
    (mpidr() & 0xff) as u32
}

/// Returns the raw MPIDR_EL1 value for this logical processor.
pub fn mpidr() -> u64 {
    let mpidr_el1: u64;
    unsafe {
        core::arch::asm!(
            "mrs {mpidr_el1}, mpidr_el1",
            mpidr_el1 = out(reg) mpidr_el1,
            options(nomem, nostack, preserves_flags)
        );
    }
    mpidr_el1
}

/// Print the MPIDR at boot so we can verify the topology.
pub fn log_mpidr() {
    let m = mpidr();
    let a3 = (m >> 32) & 0xff;
    let a2 = (m >> 16) & 0xff;
    let a1 = (m >> 8)  & 0xff;
    let a0 = m & 0xff;
    let lp = get_lp_id();
    crate::early_logln!("[MPIDR] LP{} mpidr={} aff={}.{}.{}.{}", lp, m, a3, a2, a1, a0);
}

pub fn set_lp_local_base(vaddr: VAddr) {
    unsafe {
        core::arch::asm!(
            "msr tpidr_el0, {vaddr:x}",
            vaddr = in(reg) <VAddr as Into<u64>>::into(vaddr),
            options(nomem, nostack, preserves_flags)
        );
    }
}

pub fn get_lp_local_base() -> crate::memory::VAddr {
    let addr: u64;
    unsafe {
        core::arch::asm!(
            "mrs {addr}, tpidr_el0",
            addr = out(reg) addr,
            options(nomem, nostack, preserves_flags)
        );
    }
    crate::memory::VAddr::from(addr)
}

/// Conditionally yield the current logical processor to the scheduler if a
/// context switch is pending.
///
/// This mirrors the x86-64 implementation: it collects the switch parameters
/// (the saved-SP slots of the current and next threads) while holding the
/// scheduler locks, releases every lock, and only then performs the actual
/// register/stack switch via [`switch_ctx`]. Releasing the locks first is
/// essential because `switch_ctx` may permanently abandon the current stack
/// (on the very first switch away from the boot context), so any lock guard
/// still held across the switch would never be dropped and would deadlock the
/// system.
#[unsafe(no_mangle)]
pub extern "C" fn cond_yield_lp() {
    let interrupts_were_enabled = get_int_state();
    mask_interrupts!();
    // Collect switch parameters and release all locks before calling switch_ctx.
    let switch_params: Option<(*mut u64, *const u64, *mut u8, *mut u8)> = {
        let sched = SYSTEM_SCHEDULER.read();
        let mut lsched = sched.get_lp_scheduler().lock();
        if lsched.is_ctx_switch_pending() {
            let curr_tid = lsched.get_tid();
            if let Ok(next_tid) = lsched.next() {
                if let Some(curr_tid) = curr_tid {
                    if next_tid != curr_tid {
                        let (curr_sp_ptr, curr_on_cpu, next_sp_ptr, next_on_cpu) = {
                            let mut tt_guard = MASTER_THREAD_TABLE.write();
                            let curr_thread = tt_guard
                                .get_mut(curr_tid)
                                .expect("Current thread not found during yield.");
                            let curr_sp_ptr = &raw mut curr_thread.context.saved_sp;
                            let curr_on_cpu = &raw mut curr_thread.context.on_cpu;
                            let next_thread = tt_guard
                                .get_mut(next_tid)
                                .expect("Next thread not found during yield.");
                            let next_sp_ptr = &raw mut next_thread.context.saved_sp;
                            let next_on_cpu = &raw mut next_thread.context.on_cpu;
                            (curr_sp_ptr, curr_on_cpu, next_sp_ptr, next_on_cpu)
                        };
                        lsched.clear_ctx_switch_pending();
                        Some((curr_sp_ptr, next_sp_ptr, curr_on_cpu, next_on_cpu))
                    } else {
                        // The only runnable thread is the current one, so there
                        // is nothing to switch to. Still clear the pending flag
                        // (which re-arms the quantum timer): otherwise, with a
                        // single runnable thread, the timer is never re-armed
                        // and stops firing, which would freeze `sleep` and any
                        // other timer-driven wakeups.
                        lsched.clear_ctx_switch_pending();
                        None
                    }
                } else {
                    let (next_sp_ptr, next_on_cpu) = {
                        let mut tt_guard = MASTER_THREAD_TABLE.write();
                        let next_thread = tt_guard
                            .get_mut(next_tid)
                            .expect("Next thread not found during yield.");
                        (
                            &raw mut next_thread.context.saved_sp as *const u64,
                            &raw mut next_thread.context.on_cpu,
                        )
                    };
                    lsched.clear_ctx_switch_pending();
                    Some((core::ptr::null_mut(), next_sp_ptr, core::ptr::null_mut(), next_on_cpu))
                }
            } else {
                logln!(
                    "LP {:?}: No runnable threads found during yield, even though a context \
                     switch was pending. Awaiting interrupt...",
                    (get_lp_id())
                );
                await_interrupt!();
            }
        } else {
            None
        }
        // lsched and sched guards dropped here before switch_ctx
    };
    if let Some((curr_sp_ptr, next_sp_ptr, curr_on_cpu, next_on_cpu)) = switch_params {
        unsafe {
            switch_ctx(curr_sp_ptr, next_sp_ptr, curr_on_cpu, next_on_cpu);
        }
    }
    // Reap any threads that exited: this runs after switching away from a dying
    // thread, so we are now on a different thread's stack and can safely free
    // the dead thread's kernel stack.
    crate::cpu::scheduler::threads::reap_dead_threads();
    if interrupts_were_enabled {
        unmask_interrupts!();
    }
}

/// Perform a kernel-mode context switch between two threads.
///
/// `curr_sp_ptr` receives the current thread's stack pointer after its
/// callee-saved state has been pushed; if it is null the current context is
/// abandoned without being saved (used for the first switch away from the boot
/// context). `next_sp_ptr` points at the stack pointer to restore for the
/// incoming thread.
///
/// `curr_on_cpu` / `next_on_cpu` point at the respective threads' `on_cpu`
/// ownership bytes (null when there is no such thread, e.g. an abandoned boot
/// context). The routine implements the SMP hand-off:
///  1. save the outgoing thread, then **release-store** `*curr_on_cpu = 0`,
///     publishing the completed save to other LPs;
///  2. **acquire-wait** until `*next_on_cpu == 0`, so a thread that was woken
///     onto this LP is never restored until the LP that last ran it finished
///     saving it (closing the wake-before-save race), then claim it by setting
///     `*next_on_cpu = 1`;
///  3. restore the incoming thread.
///
/// The saved frame layout (from higher to lower address, i.e. in push order)
/// is: `ttbr0_el1`, then the callee-saved general purpose registers x19-x30.
/// The AArch64 PCS requires x19-x28 plus the frame pointer x29 and the link
/// register x30 to be preserved across calls, so saving these is sufficient to
/// resume the interrupted `cond_yield_lp` in the outgoing thread. Restoring x30
/// and executing `ret` returns into the incoming thread exactly where it last
/// called `switch_ctx` (or into a trampoline for a freshly created thread).
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn switch_ctx(
    curr_sp_ptr: *mut u64,
    next_sp_ptr: *const u64,
    curr_on_cpu: *mut u8,
    next_on_cpu: *mut u8,
) {
    naked_asm!(
        // x0 = curr_sp_ptr, x1 = next_sp_ptr, x2 = curr_on_cpu, x3 = next_on_cpu
        "cbz x0, 2f",
        // Save callee-saved registers of the outgoing thread.
        "stp x29, x30, [sp, #-16]!",
        "stp x27, x28, [sp, #-16]!",
        "stp x25, x26, [sp, #-16]!",
        "stp x23, x24, [sp, #-16]!",
        "stp x21, x22, [sp, #-16]!",
        "stp x19, x20, [sp, #-16]!",
        // Save the outgoing thread's user translation table base register as a
        // 16-byte pair (with a zero pad) to keep the stack 16-byte aligned, as
        // required by the SP alignment check that firmware enables.
        "mrs x4, ttbr0_el1",
        "stp x4, xzr, [sp, #-16]!",
        // Store the outgoing stack pointer into *curr_sp_ptr.
        "mov x4, sp",
        "str x4, [x0]",
        // Publish the completed save: release-store *curr_on_cpu = 0 so another
        // LP that acquire-observes it may safely restore this thread. The
        // release orders the saved_sp store above before the flag clear.
        "cbz x2, 2f",
        "stlrb wzr, [x2]",
        "2:",
        // Wait until the incoming thread is no longer owned by any LP (its last
        // LP finished saving it), then atomically claim it. The exclusive
        // acquire load pairs with the release-store above so we observe its
        // fully-saved context; the store-exclusive prevents two LPs from both
        // claiming the same thread if a scheduler bug ever duplicates a run
        // queue entry.
        "cbz x3, 4f",
        "3:",
        "ldaxrb w4, [x3]",
        "cbnz w4, 3b",
        "mov w5, #1",
        "stxrb w6, w5, [x3]",
        "cbnz w6, 3b",
        "4:",
        // Load the incoming stack pointer from *next_sp_ptr.
        "ldr x4, [x1]",
        "mov sp, x4",
        // Restore the incoming thread's user translation table base register
        // and synchronise so subsequent EL0 accesses use the new mappings.
        "ldp x4, xzr, [sp], #16",
        "mrs x5, ttbr0_el1",
        "msr ttbr0_el1, x4",
        // If the address space changed, invalidate this core's stage-1 TLB.
        // Address spaces do not yet carry distinct hardware ASIDs (TTBR0.ASID is
        // 0 for all of them), so without this a second user address space would
        // hit stale TLB entries from the first — benign under emulation but a
        // real fault on hardware/HVF. Kernel<->kernel switches keep the same
        // TTBR0 and skip the flush.
        "cmp x4, x5",
        "b.eq 5f",
        "tlbi vmalle1",
        "5:",
        "dsb ish",
        "isb",
        // Restore callee-saved registers.
        "ldp x19, x20, [sp], #16",
        "ldp x21, x22, [sp], #16",
        "ldp x23, x24, [sp], #16",
        "ldp x25, x26, [sp], #16",
        "ldp x27, x28, [sp], #16",
        "ldp x29, x30, [sp], #16",
        // Return into the incoming thread.
        "ret",
    );
}

/// Trampoline used as the initial return target for a freshly created kernel
/// thread. The thread's entry point is placed in the x19 slot of the initial
/// saved frame; when `switch_ctx` restores that frame and returns here, we
/// unmask interrupts, call the entry point, and abort the thread cleanly if it
/// ever returns.
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn kernel_thread_trampoline() -> ! {
    naked_asm!(
        // Enable interrupts for the newly scheduled thread.
        "msr daifclr, 0b1111",
        // The entry point was restored into x19 by switch_ctx.
        "blr x19",
        // If the entry point returns, abort the thread. `abort` is `-> !`.
        "bl {abort}",
        abort = sym crate::cpu::scheduler::abort,
    );
}

/// Trampoline used to enter a user thread at EL0. The initial saved frame is
/// arranged so that `switch_ctx` restores it and returns here with:
/// - x19 = user entry point (loaded into `ELR_EL1`)
/// - x20 = user stack top (loaded into `SP_EL0`)
///
/// `SPSR_EL1` is set to zero, which selects EL0t (EL0 using `SP_EL0`) with all
/// interrupts unmasked, and `eret` then drops to the user entry point.
#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn user_trampoline() -> ! {
    naked_asm!(
        "msr elr_el1, x19",
        "msr sp_el0, x20",
        "msr spsr_el1, xzr",
        "eret",
    );
}

/// Entry point of a logical processor's dedicated idle thread.
///
/// The idle thread runs only when its LP has no other runnable thread. It waits
/// for an interrupt; the IRQ handler's tail (`cond_yield_lp`) switches to any
/// thread that has since become runnable, so when execution resumes here there
/// was simply nothing to run and we wait again. Interrupts are (re-)enabled on
/// each iteration so the quantum timer and IPIs can wake the LP.
pub extern "C" fn lp_idle_loop() {
    loop {
        // Drain deferred device-interrupt wakes (see `yield_lp`) so a driver
        // shard blocked in `CQ_WAIT` is released even when its LP has nothing
        // else to run. A resulting wake makes the driver runnable and marks a
        // context switch pending, honoured by the IRQ-tail `cond_yield_lp`
        // after the `wfi` returns on the next interrupt.
        crate::device::drain_deferred_wakes();
        // A deferred wake may select this same idle LP. Same-LP admission does
        // not need an IPI, so honour its pending switch before sleeping.
        cond_yield_lp();
        unsafe {
            core::arch::asm!(
                "msr daifclr, 0b1111",
                "wfi",
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}
