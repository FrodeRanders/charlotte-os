//! CharlotteOS userspace runtime — the equivalent of `crt0` for EL0 programs.
//!
//! Provides:
//! - The [`entry!`] macro that generates `_start`, `#[panic_handler]`,
//!   and `#[global_allocator]`.
//! - A [`config`] module for typed input/output via the canonical config page.
//!
//! ## Usage
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! extern crate alloc;
//! use catten_syscall::*;
//! use catten_rt::config;
//!
//! fn cmain() -> ! {
//!     let a: u32 = config::read(0);
//!     let b: u32 = config::read(4);
//!     config::write(0, a.wrapping_add(b));
//!     unsafe { thread_exit(); }
//! }
//!
//! catten_rt::entry!(cmain);
//! ```
//!
//! The program does **not** define `_start`, `panic_handler`, or an allocator,
//! and never references `RESULT_PAGE`, `READ_BUF`, ASID, or fixed VAs.
#![no_std]

pub mod config;

use core::panic::PanicInfo;

// ---- entry macro -----------------------------------------------------------

/// Generates the full EL0 program entry infrastructure: `_start`, a
/// `#[panic_handler]`, and a `#[global_allocator]` backed by a talc arena.
///
/// The user function takes no arguments and never returns.  Inputs are read
/// from the config page via [`config::read`]; outputs are written via
/// [`config::write`].
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
            $entry_fn()
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
