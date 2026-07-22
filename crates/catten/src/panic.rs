//! # Rust Panic Handler

use core::panic::PanicInfo;

use crate::cpu::isa::lp::ops::await_interrupt;

/// The panic handler must be dependency-light: it uses `early_log`/`early_logln`
/// (direct serial writes) rather than `logln!`, because `logln!` routes through
/// `INT_STATE`, a `LazyLock` whose first-use initialization allocates from the
/// kernel heap. A panic can occur before the heap is initialized, or *because*
/// the heap/memory subsystem faulted — in which case using the heap from the
/// panic handler causes a fault, which re-enters the panic handler, recursing
/// until the stack overflows and the CPU triple-faults (observed on x86_64 as a
/// silent hang). Writing straight to the serial console avoids that trap.
#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    crate::early_logln!("***\nKernel panic:\n{}\n***", info);
    await_interrupt!();
}
