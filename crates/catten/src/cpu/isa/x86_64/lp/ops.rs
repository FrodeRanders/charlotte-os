//! # Low-level operations for x86_64 Logical Processors

/// Per-LP data reached through the kernel GS base (`swapgs` on SYSCALL entry).
///
/// `kernel_stack` holds the top of the currently active user thread's kernel
/// stack (kept in lockstep with `TSS.RSP0`); the SYSCALL trampoline loads it as
/// its kernel stack. `user_stack` is a scratch slot in which the trampoline
/// saves the user's RSP before switching stacks.
#[repr(C)]
pub struct PerCpuData {
    pub kernel_stack: u64,
    pub user_stack: u64,
}

#[unsafe(no_mangle)]
pub static mut PER_CPU: [PerCpuData; crate::cpu::scheduler::system_scheduler::MAX_TRACKED_LPS] = [const {
    PerCpuData {
        kernel_stack: 0,
        user_stack: 0,
    }
};
    crate::cpu::scheduler::system_scheduler::MAX_TRACKED_LPS];

pub fn init_lp_state() {
    let basic_max = core::arch::x86_64::__cpuid(0).eax;
    let extended_max = core::arch::x86_64::__cpuid(0x8000_0000).eax;
    let leaf7 = (basic_max >= 7).then(|| core::arch::x86_64::__cpuid_count(7, 0));
    assert!(
        leaf7.is_some_and(|features| features.ebx & (1 << 0) != 0),
        "x86_64 CPU lacks required FSGSBASE support"
    );
    assert!(
        leaf7.is_some_and(|features| features.ebx & (1 << 7) != 0),
        "x86_64 CPU lacks required SMEP support"
    );
    assert!(
        extended_max >= 0x8000_0001
            && core::arch::x86_64::__cpuid(0x8000_0001).edx & (1 << 27) != 0,
        "x86_64 CPU lacks required RDTSCP support"
    );
    unsafe {
        core::arch::asm! {
            "mov rax, cr4",
            "or rax, 1<<16",     // enable FSGSBASE
            // The kernel never executes user mappings. User data is reached
            // through validated physical/HHDM aliases, so SMEP can remain on.
            "bts rax, 20",       // enable SMEP
            // Direct supervisor accesses to user mappings have not all been
            // converted to explicit STAC/CLAC regions yet.
            "btr rax, 21",       // clear SMAP
            "mov cr4, rax",
            "mov rax, 0",
            "wrfsbase rax",
            out("rax") _
        }
    }
    // Kernel execution always keeps the active GS base pointed at this LP's
    // per-CPU area. Ring-3 entry swaps to the zero user GS base; SYSCALL and
    // interrupt entry swap the per-CPU base back in before running Rust code.
    let lp_id = get_lp_id() as usize;
    let per_cpu_addr = unsafe { core::ptr::addr_of!(PER_CPU[lp_id]) } as u64;
    unsafe {
        core::arch::asm!(
            "wrgsbase {}",
            in(reg) per_cpu_addr,
            options(nomem, nostack, preserves_flags)
        );
        crate::cpu::isa::x86_64::constants::msrs::write(
            crate::cpu::isa::x86_64::constants::msrs::KERNEL_GS_BASE,
            0,
        );
    }
    // Enable SYSCALL/SYSRET. The handler address comes from the
    // syscall_entry trampoline in interrupts/syscall.rs.
    unsafe extern "C" {
        fn syscall_entry();
    }
    let handler_addr = syscall_entry as *const () as usize as u64;
    unsafe {
        crate::cpu::isa::x86_64::constants::msrs::setup_syscall(handler_addr);
    }
}

#[rustfmt::skip]
#[macro_export]
macro_rules! await_interrupt {
    () => {
        loop {
            unsafe {
                core::arch::asm!(
                    "sti",
                    "hlt", 
                    options(nomem, nostack, preserves_flags)
                );
            }
        }
    };
}
#[rustfmt::skip]
pub use await_interrupt;

#[inline(always)]
pub fn get_int_state() -> bool {
    let rflags: u64;
    unsafe {
        core::arch::asm!(
            "pushfq",
            "pop {}",
            out(reg) rflags,
            options(nomem, nostack, preserves_flags)
        );
    }
    rflags & (1 << rflags::IF_SHIFT) != 0
}

#[rustfmt::skip]
#[macro_export]
macro_rules! mask_interrupts {
    () => {
        unsafe {
            core::arch::asm!("cli", options(nomem, nostack));
        }
    };
}
#[rustfmt::skip]
pub use mask_interrupts;

#[rustfmt::skip]
#[macro_export]
macro_rules! unmask_interrupts {
    () => {
        unsafe {
            core::arch::asm!("sti", options(nomem, nostack));
        }
    };
}
#[rustfmt::skip]
pub use unmask_interrupts;

pub fn get_lic_id() -> u32 {
    let apic_id: u32;
    use crate::cpu::isa::constants::*;
    unsafe {
        core::arch::asm!(
            "rdmsr",
            inlateout("ecx") msrs::x2apic::ID_REG => _,
            lateout("eax") apic_id,
            lateout("edx") _,
            options(nostack, preserves_flags)
        );
    }
    apic_id
}

use core::{
    arch::{
        asm,
        naked_asm,
    },
    sync::atomic::Ordering,
};

use super::LpId;
use crate::cpu::isa::constants::*;

pub fn store_lp_id(id: LpId) {
    unsafe {
        asm!(
            "wrmsr",
            in("eax") id,
            in("edx") 0_u32,
            in("ecx") msrs::TSC_AUX,
            options(nostack, preserves_flags)
        );
    }
}

pub fn get_lp_id() -> LpId {
    let mut id: u32;
    unsafe {
        core::arch::asm!(
            "rdtscp",
            out("edx") _,
            out("eax") _,
            out("ecx") id,
        );
    }
    id as crate::cpu::isa::lp::LpId
}

use crate::{
    cpu::scheduler::{
        system_scheduler::SYSTEM_SCHEDULER,
        threads::MASTER_THREAD_TABLE,
    },
    logln,
    memory::VAddr,
};

#[inline]
pub extern "C" fn get_lp_local_base() -> VAddr {
    let ret: u64;
    unsafe {
        core::arch::asm!(
            "rdgsbase {}",
            out(reg) ret,
            options(nomem, nostack, preserves_flags)
        );
    }
    VAddr::from(ret)
}

#[inline]
pub extern "C" fn set_lp_local_base(base: VAddr) {
    unsafe {
        core::arch::asm!(
            "wrgsbase {}",
            in(reg) <VAddr as Into<u64>>::into(base),
            options(nomem, nostack, preserves_flags)
        )
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn cond_yield_lp() {
    let interrupts_were_enabled = get_int_state();
    mask_interrupts!();
    #[cfg(feature = "yield_trace")]
    #[derive(Clone, Copy)]
    enum YieldTrace {
        None,
        NoSwitch {
            lp_id: LpId,
        },
        FromThread {
            current: usize,
            next: usize,
            lp_id: LpId,
        },
        FromNonThread {
            next: usize,
            lp_id: LpId,
        },
    }
    #[cfg(feature = "yield_trace")]
    let mut trace = YieldTrace::None;
    // Collect switch parameters and release all locks before calling switch_ctx.
    // switch_ctx may permanently abandon the current stack (initial non-thread switch),
    // so any guards held across it would never be dropped, leaving locks permanently locked.
    let switch_params: Option<(*mut u64, *const u64, usize, u64)> = {
        let sched = SYSTEM_SCHEDULER.read();
        let mut lsched = sched.get_lp_scheduler().lock();
        if lsched.is_ctx_switch_pending() {
            let curr_tid = lsched.get_tid();
            if let Ok(next_tid) = lsched.next() {
                if curr_tid.is_some() {
                    if next_tid != curr_tid.unwrap() {
                        let (curr_rsp0_ptr, next_rsp0_ptr, next_asid, next_stack_top) = {
                            let mut tt_guard = MASTER_THREAD_TABLE.write();
                            let curr_thread = tt_guard
                                .get_mut(
                                    curr_tid.expect("Current thread ID not found during yield."),
                                )
                                .expect("Current thread not found during yield.");
                            let curr_rsp0_ptr = &raw mut curr_thread.context.rsp_cpl0;
                            let next_thread = tt_guard
                                .get_mut(next_tid)
                                .expect("Next thread not found during yield.");
                            let next_rsp0_ptr = &raw mut next_thread.context.rsp_cpl0;
                            let next_asid = next_thread.asid;
                            let next_stack_top = next_thread.context.kernel_stack_top;
                            (curr_rsp0_ptr, next_rsp0_ptr, next_asid, next_stack_top)
                        };
                        #[cfg(feature = "yield_trace")]
                        {
                            trace = YieldTrace::FromThread {
                                current: curr_tid.unwrap(),
                                next: next_tid,
                                lp_id: get_lp_id(),
                            };
                        }
                        lsched.clear_ctx_switch_pending();
                        Some((curr_rsp0_ptr, next_rsp0_ptr, next_asid, next_stack_top))
                    } else {
                        #[cfg(feature = "yield_trace")]
                        {
                            trace = YieldTrace::NoSwitch {
                                lp_id: get_lp_id(),
                            };
                        }
                        // Nothing else to run: still clear the pending flag so
                        // the quantum timer is re-armed (otherwise the timer
                        // stops when a single thread is runnable).
                        lsched.clear_ctx_switch_pending();
                        None
                    }
                } else {
                    let (next_rsp0_ptr, next_asid, next_stack_top) = {
                        let mut tt_guard = MASTER_THREAD_TABLE.write();
                        let next_thread = tt_guard
                            .get_mut(next_tid)
                            .expect("Next thread not found during yield.");
                        (
                            &raw mut next_thread.context.rsp_cpl0,
                            next_thread.asid,
                            next_thread.context.kernel_stack_top,
                        )
                    };
                    #[cfg(feature = "yield_trace")]
                    {
                        trace = YieldTrace::FromNonThread {
                            next: next_tid,
                            lp_id: get_lp_id(),
                        };
                    }
                    lsched.clear_ctx_switch_pending();
                    Some((core::ptr::null_mut(), next_rsp0_ptr, next_asid, next_stack_top))
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
    #[cfg(feature = "yield_trace")]
    match trace {
        YieldTrace::None => {}
        YieldTrace::NoSwitch {
            lp_id,
        } => logln!(
            "No thread switch needed during yield on LP {:?} because the next thread is the same \
             as the current thread.",
            lp_id
        ),
        YieldTrace::FromThread {
            current,
            next,
            lp_id,
        } => logln!("Yielding from thread {:?} to thread {:?} on LP {:?}", current, next, lp_id),
        YieldTrace::FromNonThread {
            next,
            lp_id,
        } => logln!("Yielding from non-thread context to thread {:?} on LP {:?}", next, lp_id),
    }
    if let Some((curr_rsp0_ptr, next_rsp0_ptr, next_asid, next_stack_top)) = switch_params {
        // Publish the incoming thread's ASID and kernel-stack top before the
        // switch so the SYSCALL entry path attributes the caller to the right
        // address space and lands on the correct per-thread kernel stack.
        let lp_id = get_lp_id() as usize;
        crate::cpu::isa::x86_64::memory::paging::CURRENT_LOGICAL_ASID[lp_id]
            .store(next_asid, Ordering::Release);
        if next_asid != crate::memory::KERNEL_ASID {
            crate::cpu::isa::x86_64::init::gdt::write_rsp0(next_stack_top);
            unsafe {
                PER_CPU[lp_id].kernel_stack = next_stack_top;
            }
        }
        switch_ctx(curr_rsp0_ptr, next_rsp0_ptr);
    }
    crate::cpu::scheduler::threads::retire_requested_threads();
    // Reap exited threads now that we are on a different thread's stack.
    crate::cpu::scheduler::threads::reap_dead_threads();
    crate::cpu::scheduler::maybe_sample_rebalance();
    if interrupts_were_enabled {
        unmask_interrupts!();
    }
}

#[unsafe(no_mangle)]
#[unsafe(naked)]
pub extern "C" fn switch_ctx(curr_rsp0_ptr: *mut u64, next_rsp0_ptr: *const u64) {
    naked_asm!(
        // if `curr_rsp0_ptr` is null, then we are yielding from a non-thread context (e.g., the initial kernel thread context after boot) and thus we don't need to save the current context
        "cmp rdi, 0",
        "je skip_save",
        // save caller-saved registers
        "push rbx",
        "push rbp",
        "push r12",
        "push r13",
        "push r14",
        "push r15",
        "pushfq",
        "mov rax, cr3",
        "push rax",
        // compute the stack pointer offset in the thread context and save it to the current thread context
        "mov [rdi], rsp",
        "skip_save:",
        // load the stack pointer from the next thread context
        "mov rsp, [rsi]",
        // restore caller-saved registers
        "pop rax",
        "mov cr3, rax",
        "pop rax",
        "pop r15",
        "pop r14",
        "pop r13",
        "pop r12",
        "pop rbp",
        "pop rbx",
        "push rax",
        "popfq",
        // return to the next thread
        "ret",
    );
}

#[unsafe(no_mangle)]
#[unsafe(naked)]
pub extern "C" fn enter_init_thread_ctx(rsp0_ptr: *const u64) {
    naked_asm!(
                // load the stack pointer from the next thread context
        "mov rsp, [rsi]",
        // restore caller-saved registers
        "pop rax",
        "mov cr3, rax",
        "popfq",
        "xor r15, r15",
        "xor r14, r14",
        "xor r13, r13",
        "xor r12, r12",
        "xor r11, r11",
        "xor r10, r10",
        "xor r9, r9",
        "xor r8, r8",
        "xor rbp, rbp",
        "xor rdx, rdx",
        "xor rcx, rcx",
        "xor rbx, rbx",
        "xor rax, rax",     
        // return to the thread's kernel entry point (which will then `iretq` to the user entry point for user threads)
        "ret",
    );
}

#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn user_trampoline() -> ! {
    // Safety: This function should only be entered by returning from `yield_lp` after having
    // switched to a new user thread. The caller is responsible for ensuring that the stack is
    // properly set up with a `UserEntryFrames` struct, and that the CPU is in the correct state for
    // executing this trampoline (e.g., interrupts disabled, correct segment selectors, etc.).
    naked_asm!(
        // Context switches run with the per-LP kernel GS base active. Swap to
        // the zero user base before entering ring 3; the hidden kernel base is
        // retained for the next privilege transition.
        "swapgs",
        // `iretq` to the user entry point
        "iretq",
    );
}

#[unsafe(no_mangle)]
#[unsafe(naked)]
pub unsafe extern "C" fn kernel_thread_trampoline() -> ! {
    naked_asm!(
        "sti",
        "sub rsp, 8",
        "call r12",
        // The entry point returned: abort the thread so it is descheduled and
        // reaped (rsp is 16-byte aligned here after the `sub rsp, 8` above, as
        // required by the SysV ABI for the call). `abort` never returns.
        "call {abort}",
        abort = sym crate::cpu::scheduler::abort,
    );
}

/// Entry point of a logical processor's dedicated idle thread.
///
/// The idle thread runs only when its LP has no other runnable thread. It waits
/// for an interrupt; the interrupt handler's tail (`cond_yield_lp`) switches to
/// any thread that has since become runnable, so when execution resumes here
/// there was simply nothing to run and we wait again. Interrupts are
/// (re-)enabled on each iteration so the quantum timer and IPIs can wake the LP.
pub extern "C" fn lp_idle_loop() {
    loop {
        // Drain deferred device-interrupt wakes (see `yield_lp`) so a driver
        // shard blocked in `CQ_WAIT` is released even when its LP has nothing
        // else to run.
        crate::device::drain_deferred_wakes();
        // A deferred wake may select this same idle LP. Same-LP admission does
        // not need an IPI, so honour its pending switch before sleeping.
        // Reconcile the software timer queue with the hardware comparator so
        // an LP never halts with a queued event but no timer interrupt armed.
        crate::timers::process_local_events();
        cond_yield_lp();
        unsafe {
            core::arch::asm!("sti", "hlt", options(nomem, nostack, preserves_flags));
        }
    }
}
