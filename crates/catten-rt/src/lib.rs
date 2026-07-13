//! CharlotteOS userspace runtime — the equivalent of `crt0` for EL0 programs.
//!
//! Provides the [`entry!`] macro that generates `_start`, `#[panic_handler]`,
//! and `#[global_allocator]`, so the user's program only needs to define
//! their business logic.
//!
//! ## Usage
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! extern crate alloc;
//! use catten_syscall::*;
//!
//! fn cmain(asid: u64) -> ! {
//!     let cap = unsafe { submit(OpCode::Nop) };
//!     unsafe { wait(cap); }
//!     unsafe { thread_exit(); }
//! }
//!
//! catten_rt::entry!(cmain);
//! ```
#![no_std]

use core::panic::PanicInfo;

// ---- config page ----------------------------------------------------------
// The kernel writes the calling thread's ASID to offset 16 of the canonical
// config page (VA 0x0001_F000) during address-space setup.  `_start` reads it
// here and passes it to the user's entry function.
pub const CONFIG_VADDR: usize = 0x0000_0000_0001_0000;

/// Generates the full EL0 program entry infrastructure: `_start` (reads
/// ASID from the kernel-mapped config page and calls the given function),
/// a `#[panic_handler]`, and a `#[global_allocator]` backed by a talc arena.
///
/// The user function must have signature `fn(u64) -> !`.
///
/// ```ignore
/// catten_rt::entry!(my_main);
/// ```
#[macro_export]
macro_rules! entry {
    ($entry_fn:ident) => {
        #[global_allocator]
        static ALLOCATOR: $crate::HeapLock = $crate::heap();

        #[panic_handler]
        fn __catten_panic(_info: &::core::panic::PanicInfo) -> ! {
            unsafe { $crate::thread_exit(); }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn _start() -> ! {
            let asid = unsafe {
                ::core::ptr::read_volatile(
                    ($crate::CONFIG_VADDR as *const u32).add(4)
                ) as u64
            };
            $entry_fn(asid)
        }
    };
}

// ---- allocator support (used by entry! macro) -----------------------------

pub use talc::{source::Claim, TalcLock};

pub type HeapLock = TalcLock<spin::Mutex<()>, Claim>;

/// Construct the heap arena at the canonical heap VADDR (0x13000).
pub const fn heap() -> HeapLock {
    TalcLock::new(unsafe {
        Claim::new(0x0000_0000_0001_3000usize as *mut u8, 0xD000)
    })
}

// ---- plumbing (not user-facing) -------------------------------------------

pub use catten_syscall::thread_exit;
