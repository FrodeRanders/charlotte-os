//! CharlotteOS userspace runtime — the equivalent of `crt0` for EL0 programs.
//!
//! Provides:
//! - The [`entry!`] macro that generates `_start`, `#[panic_handler]`, and `#[global_allocator]`.
//! - A [`config`] module for typed output via the canonical config page.
//! - A safe [`Context`] passed to the program's Rust `main` function.
//!
//! ## Usage
//!
//! ```ignore
//! #![no_std]
//! #![no_main]
//!
//! extern crate alloc;
//! use catten_syscall::*;
//! use catten_rt::{config, Context};
//!
//! fn main(ctx: Context) -> ! {
//!     let a = ctx.arg(0).unwrap_or(0);
//!     let b = ctx.arg(1).unwrap_or(0);
//!     config::write(0, a.wrapping_add(b));
//!     unsafe { thread_exit(); }
//! }
//!
//! catten_rt::entry!(main);
//! ```
//!
//! The program does **not** define `_start`, `panic_handler`, or an allocator.
//! `_start` constructs the context before entering `main`. Programs that need
//! startup input request it explicitly with [`Context::read_startup_input`].
#![no_std]

pub mod config;

// ---- entry macro -----------------------------------------------------------

/// Generates the full EL0 program entry infrastructure: `_start`, a
/// `#[panic_handler]`, and a `#[global_allocator]` backed by a talc arena.
///
/// The user function takes a safe [`Context`] and never returns. The generated
/// `_start` remains the ELF entry point; `main` is a Rust source-level contract.
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

/// Launch-time facilities supplied to a CharlotteOS program.
///
/// This is the normal developer-facing startup contract. It hides canonical
/// virtual addresses and config-page offsets; raw config access remains
/// available for existing low-level services during the ABI transition.
#[derive(Clone, Copy)]
pub struct Context {
    args: &'static [u32],
}

impl Context {
    pub fn args(&self) -> &'static [u32] {
        self.args
    }

    pub fn arg(&self, index: usize) -> Option<u32> {
        self.args.get(index).copied()
    }

    pub fn capabilities(&self) -> InitialCapabilities {
        InitialCapabilities {
            index: 0,
        }
    }

    pub fn heap_layout(&self) -> MemoryRegion {
        let header = config::launch_layout();
        MemoryRegion {
            base: header.heap_base as usize,
            size: header.heap_size as usize,
        }
    }

    pub fn input_layout(&self) -> MemoryRegion {
        let header = config::launch_layout();
        MemoryRegion {
            base: header.input_base as usize,
            size: header.input_size as usize,
        }
    }

    pub fn completion_queue_layout(&self) -> CompletionQueueLayout {
        let header = config::launch_layout();
        CompletionQueueLayout {
            base: header.cq_base as usize,
            entries: header.cq_entries,
        }
    }

    /// Mutable diagnostic/status region, separate from read-only launch data.
    pub fn status_layout(&self) -> MemoryRegion {
        let header = config::launch_layout();
        MemoryRegion {
            base: header.status_base as usize,
            size: header.status_size as usize,
        }
    }

    pub fn bootstrap_cap(&self) -> Option<u64> {
        config::bootstrap_cap()
    }

    pub fn mmio_cap(&self) -> Option<u64> {
        config::mmio_cap()
    }

    pub fn irq_cap(&self) -> Option<u64> {
        config::irq_cap()
    }

    pub fn shard_cq_base(&self) -> Option<usize> {
        config::shard_cq_base()
    }

    pub fn shard_cq_count(&self) -> usize {
        config::shard_cq_count()
    }

    pub fn handoff_count(&self) -> u32 {
        config::handoff_count()
    }

    pub fn handoff_state_cap(&self) -> u64 {
        config::handoff_state_cap()
    }

    pub fn handoff_endpoint_cap(&self) -> u64 {
        config::handoff_endpoint_cap()
    }

    /// Read launch input explicitly, blocking until the requested buffer has
    /// been filled. The loader currently provides at most one 4 KiB page.
    pub fn read_startup_input(&self, buffer: &mut [u8]) -> Result<(), InputError> {
        if buffer.len() > config::INPUT_CAPACITY {
            return Err(InputError::TooLarge);
        }
        if buffer.is_empty() {
            return Ok(());
        }
        let cap =
            unsafe { catten_syscall::submit_read(buffer.as_mut_ptr() as usize, buffer.len()) };
        catten_syscall::wait(cap);
        catten_syscall::close(cap);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryRegion {
    pub base: usize,
    pub size: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionQueueLayout {
    pub base: usize,
    pub entries: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitialCapability {
    pub kind: config::CapabilityKind,
    pub rights: u16,
    pub flags: u32,
    pub handle: u64,
}

pub struct InitialCapabilities {
    index: usize,
}

impl Iterator for InitialCapabilities {
    type Item = InitialCapability;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let record = config::capability_record(self.index)?;
            self.index += 1;
            if let Some(kind) = config::CapabilityKind::from_raw(record.kind) {
                return Some(InitialCapability {
                    kind,
                    rights: record.rights,
                    flags: record.flags,
                    handle: record.handle,
                });
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputError {
    TooLarge,
}

pub fn run_main(main: fn(Context) -> !) -> ! {
    if !config::launch_header_is_compatible() {
        unsafe {
            thread_exit();
        }
    }
    main(Context {
        args: config::launch_args(),
    })
}

// ---- allocator support (used by entry! macro) -----------------------------

pub use talc::{
    TalcLock,
    source::Claim,
};

pub type HeapLock = TalcLock<spin::Mutex<()>, Claim>;

/// Construct the heap arena at the canonical heap VADDR (0x13000).
pub const fn heap() -> HeapLock {
    TalcLock::new(unsafe {
        Claim::new(charlotte_launch::HEAP_VADDR as *mut u8, charlotte_launch::HEAP_SIZE)
    })
}

// ---- plumbing (not user-facing) -------------------------------------------

pub use catten_syscall::thread_exit;
