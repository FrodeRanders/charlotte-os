//! CharlotteOS userspace runtime — the equivalent of `crt0` for EL0 programs.
//!
//! Provides:
//! - The [`entry!`] macro that generates `_start`, `#[panic_handler]`, and `#[global_allocator]`.
//! - A [`config`] module for typed output via the canonical config page.
//! - Type-driven launch arguments and input-buffer setup for `cmain`.
//!
//! ## Usage
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! extern crate alloc;
//! use catten_syscall::*;
//! use catten_rt::{config, Args, Input};
//!
//! fn cmain(args: Args, input: Input<32>) -> ! {
//!     let a = args.get(0).unwrap_or(0);
//!     let b = args.get(1).unwrap_or(0);
//!     let kernel_val = input.read_u32(0).unwrap_or(0);
//!     config::write(0, a.wrapping_add(b).wrapping_add(kernel_val));
//!     unsafe { thread_exit(); }
//! }
//!
//! catten_rt::entry!(cmain);
//! ```
//!
//! The program does **not** define `_start`, `panic_handler`, or an allocator.
//! The input length is part of the `Input<N>` parameter type; `_start` consumes
//! exactly `N` bytes before entering `cmain`.
#![no_std]

pub mod config;

// ---- entry macro -----------------------------------------------------------

/// Generates the full EL0 program entry infrastructure: `_start`, a
/// `#[panic_handler]`, and a `#[global_allocator]` backed by a talc arena.
///
/// The user function takes [`Args`] and [`Input<N>`], and never returns. The
/// input byte count is inferred from the function's `Input<N>` parameter.
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
            unsafe {
                $crate::thread_exit();
            }
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn _start() -> ! {
            $crate::run_main($entry_fn)
        }
    };
}

// ---- launch contract -------------------------------------------------------

#[derive(Clone, Copy)]
pub struct Args {
    words: &'static [u32],
}

impl Args {
    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<u32> {
        self.words.get(index).copied()
    }

    pub fn as_slice(&self) -> &'static [u32] {
        self.words
    }
}

pub struct Input<const N: usize> {
    bytes: &'static mut [u8; N],
}

impl<const N: usize> Input<N> {
    pub fn len(&self) -> usize {
        N
    }

    pub fn is_empty(&self) -> bool {
        N == 0
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..]
    }

    pub fn as_mut_bytes(&mut self) -> &mut [u8] {
        &mut self.bytes[..]
    }

    pub fn read_u32(&self, offset: usize) -> Option<u32> {
        if offset.checked_add(core::mem::size_of::<u32>())? > N {
            return None;
        }
        let ptr = unsafe { self.bytes.as_ptr().add(offset) as *const u32 };
        Some(unsafe { core::ptr::read_unaligned(ptr) })
    }
}

pub fn run_main<const N: usize>(main: fn(Args, Input<N>) -> !) -> ! {
    if N > config::INPUT_CAPACITY {
        unsafe {
            thread_exit();
        }
    }

    let args = launch_args();
    let input = launch_input::<N>();
    main(args, input)
}

fn launch_args() -> Args {
    let argc = unsafe { config::read::<usize>(config::ARGC_OFFSET) };
    let byte_len = argc.saturating_mul(core::mem::size_of::<u32>());
    if config::ARGS_OFFSET.saturating_add(byte_len) > 4096 {
        return Args {
            words: &[],
        };
    }
    let ptr = (config::CONFIG_VADDR + config::ARGS_OFFSET) as *const u32;
    Args {
        words: unsafe { core::slice::from_raw_parts(ptr, argc) },
    }
}

fn launch_input<const N: usize>() -> Input<N> {
    let bytes = config::INPUT_VADDR as *mut u8;
    if N > 0 {
        let cap = unsafe { catten_syscall::submit_read(bytes as usize, N) };
        unsafe {
            catten_syscall::wait(cap);
            catten_syscall::close(cap);
        }
    }
    Input {
        bytes: unsafe { &mut *(bytes as *mut [u8; N]) },
    }
}

// ---- allocator support (used by entry! macro) -----------------------------

pub use talc::{
    source::Claim,
    TalcLock,
};

pub type HeapLock = TalcLock<spin::Mutex<()>, Claim>;

/// Construct the heap arena at the canonical heap VADDR (0x13000).
pub const fn heap() -> HeapLock {
    TalcLock::new(unsafe { Claim::new(0x0000_0000_0001_3000usize as *mut u8, 0xd000) })
}

// ---- plumbing (not user-facing) -------------------------------------------

pub use catten_syscall::thread_exit;
