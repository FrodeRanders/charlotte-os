//! # Rust Panic Handler

use core::{
    panic::PanicInfo,
    sync::atomic::{
        AtomicBool,
        Ordering,
    },
};

const STACK_PAGE_SIZE: usize = 4096;
const MAX_BACKTRACE_FRAMES: usize = 32;

/// Only the first panicking LP attempts diagnostics. A second panic may be a
/// consequence of abandoned locks, and competing serial output would make the
/// original failure harder to recover from the log.
static PANIC_STARTED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;
}

/// The panic handler must not allocate, take ordinary kernel locks, unwind, or
/// re-enable interrupts. Panics can occur inside the allocator or while an
/// interrupt-masking lock is held, so resuming interrupt dispatch would expose
/// partially-mutated state and abandoned guards.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::cpu::isa::lp::ops::mask_interrupts!();

    if PANIC_STARTED.swap(true, Ordering::SeqCst) {
        panic_stop();
    }

    crate::log::panic_write(format_args!("***\nKernel panic:\n{info}\n"));
    dump_backtrace();
    crate::log::panic_write(format_args!("***\n"));

    // Preserve verifier attribution when its scheduler locks are available.
    // This runs after the dependency-free diagnostics so an unexpectedly
    // unavailable scheduler cannot hide the original failure.
    if let Some((tid, generation)) = crate::cpu::scheduler::current_thread_identity_nonblocking() {
        crate::self_test::results::fail_verifier_thread(tid as u64, generation);
    }

    panic_stop();
}

/// Print a conservative frame-pointer walk as raw kernel text addresses.
///
/// The custom kernel targets force frame pointers. To keep a corrupted stack
/// from turning diagnostics into a page fault, this walker never dereferences
/// outside the page containing the panic handler's current stack pointer. That
/// normally captures the allocator and its caller; deeper frames can be lost
/// when the chain crosses a page boundary. Addresses can be symbolized against
/// the unstripped `catten` ELF with `lldb` or `addr2line`.
#[inline(never)]
fn dump_backtrace() {
    let (stack_pointer, mut frame_pointer) = current_stack_and_frame_pointer();
    let page_start = stack_pointer & !(STACK_PAGE_SIZE - 1);
    let page_end = page_start.saturating_add(STACK_PAGE_SIZE);
    let text_start = core::ptr::addr_of!(__text_start) as usize;
    let text_end = core::ptr::addr_of!(__text_end) as usize;

    crate::log::panic_write(format_args!(
        "Kernel backtrace (raw return addresses; text={text_start:#018x}..{text_end:#018x}, \
         sp={stack_pointer:#018x}, fp={frame_pointer:#018x}):\n"
    ));

    let mut emitted = 0;
    for depth in 0..MAX_BACKTRACE_FRAMES {
        if frame_pointer < stack_pointer
            || frame_pointer < page_start
            || frame_pointer > page_end.saturating_sub(2 * size_of::<usize>())
            || !frame_pointer.is_multiple_of(align_of::<usize>())
        {
            break;
        }

        // SAFETY: Both words are aligned and constrained to the mapped page
        // containing the currently executing panic stack.
        let (next_frame, return_address) = unsafe {
            let frame = frame_pointer as *const usize;
            (frame.read(), frame.add(1).read())
        };
        if return_address < text_start || return_address >= text_end {
            break;
        }

        crate::log::panic_write(format_args!("  #{depth:02} {return_address:#018x}\n"));
        emitted += 1;

        if next_frame <= frame_pointer
            || next_frame > page_end.saturating_sub(2 * size_of::<usize>())
            || !next_frame.is_multiple_of(align_of::<usize>())
        {
            break;
        }
        frame_pointer = next_frame;
    }

    if emitted == 0 {
        crate::log::panic_write(format_args!(
            "  <frame chain unavailable in current stack page>\n"
        ));
    }
}

#[inline(always)]
fn current_stack_and_frame_pointer() -> (usize, usize) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        let stack_pointer: usize;
        let frame_pointer: usize;
        core::arch::asm!(
            "mov {stack_pointer}, rsp",
            "mov {frame_pointer}, rbp",
            stack_pointer = out(reg) stack_pointer,
            frame_pointer = out(reg) frame_pointer,
            options(nomem, nostack, preserves_flags),
        );
        (stack_pointer, frame_pointer)
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        let stack_pointer: usize;
        let frame_pointer: usize;
        core::arch::asm!(
            "mov {stack_pointer}, sp",
            "mov {frame_pointer}, x29",
            stack_pointer = out(reg) stack_pointer,
            frame_pointer = out(reg) frame_pointer,
            options(nomem, nostack, preserves_flags),
        );
        (stack_pointer, frame_pointer)
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    (0, 0)
}

/// Permanently stop this LP. Other LPs that subsequently panic observe
/// `PANIC_STARTED` and stop without trying to use possibly abandoned state.
#[cold]
#[inline(never)]
fn panic_stop() -> ! {
    #[cfg(target_arch = "x86_64")]
    loop {
        unsafe {
            core::arch::asm!("cli", "hlt", options(nomem, nostack));
        }
    }

    #[cfg(target_arch = "aarch64")]
    loop {
        unsafe {
            core::arch::asm!("msr daifset, #0xf", "wfi", options(nomem, nostack));
        }
    }

    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    loop {
        core::hint::spin_loop();
    }
}
