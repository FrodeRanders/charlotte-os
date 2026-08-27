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
//!     let mode = ctx.manifest_value(charlotte_launch::manifest_key(b"mode"));
//!     config::write(0, mode.is_some() as u32);
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
pub mod entropy;
pub mod log;
pub mod owned;
pub use charlotte_launch::manifest_key;

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
            $crate::domain_abort();
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
pub struct Context;

impl Context {
    /// Iterate the typed, named launch manifest in supervisor-provided order.
    /// Keys may repeat, which is useful for lists such as Raft seed peers.
    pub fn manifest(&self) -> ManifestEntries {
        ManifestEntries {
            index: 0,
        }
    }

    /// Return the first manifest value with `key`.
    pub fn manifest_value(&self, key: u64) -> Option<ManifestValue> {
        self.manifest().find(|entry| entry.key == key).map(|entry| entry.value)
    }

    pub fn capabilities(&self) -> InitialCapabilities {
        InitialCapabilities {
            index: 0,
        }
    }

    /// Borrow the immutable profile object supplied by the launcher.
    ///
    /// The capability has kernel-enforced read-only rights and remains owned
    /// by the launch environment for the domain lifetime.
    pub fn profile_memory(&self) -> Option<owned::LaunchMemoryRef<'_>> {
        let record = config::capability_record_by_kind(config::CapabilityKind::Profile)?;
        if record.rights != charlotte_launch::PROFILE_CAPABILITY_RIGHT_MAP_READ {
            return None;
        }
        let metadata = charlotte_launch::ProfileCapabilityMetadata::decode(record.metadata)?;
        // The config page is the trusted ABI boundary and the returned borrow
        // cannot outlive this Context reference.
        unsafe {
            owned::LaunchMemoryRef::from_raw(record.handle, metadata.byte_len() as usize).ok()
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

    /// Borrow the launch-provided bootstrap connection.
    ///
    /// Prefer this typed view over [`bootstrap_cap`](Self::bootstrap_cap) in
    /// services and applications. The launch environment owns the capability,
    /// so dropping the view does not close it.
    pub fn bootstrap_connection(&self) -> Option<owned::ConnectionRef<'_>> {
        let cap = config::bootstrap_cap()?;
        // The launch contract keeps initial capabilities live for the process
        // lifetime. The borrow is deliberately limited to this Context.
        unsafe { owned::ConnectionRef::from_raw(cap).ok() }
    }

    pub fn mmio_cap(&self) -> Option<u64> {
        config::mmio_cap()
    }

    pub fn irq_cap(&self) -> Option<u64> {
        config::irq_cap()
    }

    pub fn system_observer_cap(&self) -> Option<u64> {
        config::system_observer_cap()
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
        owned::ReadOperation::submit(buffer).map_err(|_| InputError::SubmissionFailed)?.wait();
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
    pub metadata: InitialCapabilityMetadata,
    pub handle: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InitialCapabilityMetadata {
    None,
    Profile(charlotte_launch::ProfileCapabilityMetadata),
    Unknown(u32),
}

fn capability_metadata(record: charlotte_launch::CapabilityRecord) -> InitialCapabilityMetadata {
    match config::CapabilityKind::from_raw(record.kind) {
        Some(config::CapabilityKind::Profile) => {
            charlotte_launch::ProfileCapabilityMetadata::decode(record.metadata)
                .map(InitialCapabilityMetadata::Profile)
                .unwrap_or(InitialCapabilityMetadata::Unknown(record.metadata))
        }
        _ if record.metadata == 0 => InitialCapabilityMetadata::None,
        _ => InitialCapabilityMetadata::Unknown(record.metadata),
    }
}

pub struct InitialCapabilities {
    index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestValue {
    Unsigned(u64),
    Signed(i64),
    Bytes(&'static [u8]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestEntry {
    pub key: u64,
    pub flags: u16,
    pub value: ManifestValue,
}

pub struct ManifestEntries {
    index: usize,
}

impl Iterator for ManifestEntries {
    type Item = ManifestEntry;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let record = config::manifest_record(self.index)?;
            self.index += 1;
            let Some(kind) = config::ManifestValueKind::from_raw(record.kind) else {
                continue;
            };
            let value = match kind {
                config::ManifestValueKind::Unsigned if record.value_len == 8 => {
                    ManifestValue::Unsigned(record.value)
                }
                config::ManifestValueKind::Signed if record.value_len == 8 => {
                    ManifestValue::Signed(record.value as i64)
                }
                config::ManifestValueKind::Bytes => {
                    let Some(bytes) = config::manifest_bytes(record) else {
                        continue;
                    };
                    ManifestValue::Bytes(bytes)
                }
                _ => continue,
            };
            return Some(ManifestEntry {
                key: record.key,
                flags: record.flags,
                value,
            });
        }
    }
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
                    metadata: capability_metadata(record),
                    handle: record.handle,
                });
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputError {
    TooLarge,
    SubmissionFailed,
}

pub fn run_main(main: fn(Context) -> !) -> ! {
    if !config::launch_header_is_compatible() {
        domain_abort();
    }
    main(Context)
}

// ---- allocator support (used by entry! macro) -----------------------------

pub use talc::{
    TalcLock,
    source::Claim,
};

pub type HeapLock = TalcLock<spin::Mutex<()>, Claim>;

/// Construct the heap arena at the canonical heap virtual address.
pub const fn heap() -> HeapLock {
    TalcLock::new(unsafe {
        Claim::new(charlotte_launch::HEAP_VADDR as *mut u8, charlotte_launch::HEAP_SIZE)
    })
}

// ---- plumbing (not user-facing) -------------------------------------------

pub use catten_syscall::{
    domain_abort,
    thread_exit,
};
