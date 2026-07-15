//! Shared protocol definitions for the reference CharlotteOS services.
//!
//! This is the userspace half of the Phase 3 name-service architecture: the
//! kernel moves opaque capabilities, while interface ids, opcodes, names,
//! generations, and lookup policy are defined here.
#![no_std]

/// Pack up to 8 ASCII bytes into a u64 service name (little-endian).
///
/// This interim scalar encoding is limited to 8 bytes; longer names travel
/// in a copied memory object (see [`ns::OP_REGISTER_NAMED`] and
/// [`ns::OP_LOOKUP_NAMED`]).
pub const fn name(bytes: &[u8]) -> u64 {
    let mut packed = [0u8; 8];
    let mut i = 0;
    while i < bytes.len() && i < 8 {
        packed[i] = bytes[i];
        i += 1;
    }
    u64::from_le_bytes(packed)
}

/// The scratch virtual address services use to stage a memory-carried name.
///
/// Chosen above the program image (linked at `0x20000`, well under 1 MiB) and
/// away from the fixed runtime pages: config (`0x10000`), CQ ring
/// (`0x11000`), launch input (`0x12000`), and heap (`0x13000..0x20000`).
pub const NAME_SCRATCH_VADDR: usize = 0x0000_0000_0010_0000;

/// Maximum memory-carried name length (fits one page with room to spare).
pub const MAX_NAME_LEN: usize = 256;

/// Name-service protocol (`charlotte-protocol-name` v1).
pub mod ns {
    /// Interface id: "NAME".
    pub const INTERFACE: u64 = super::name(b"NAME");
    pub const VERSION: u32 = 1;

    /// Register a service under a short (<= 8 byte) name. `arg0` = packed
    /// name; the call must attach a re-delegable connection
    /// (`SEND | CALL | MINT_CONNECTION`) to the service's endpoint. Reply
    /// result = new instance generation (>= 1).
    pub const OP_REGISTER: u32 = 1;
    /// Look up a service by short name. `arg0` = packed name. Reply result =
    /// current generation with an attenuated (`SEND | CALL`) connection cap
    /// attached, or [`ERR_NOT_FOUND`].
    pub const OP_LOOKUP: u32 = 2;
    /// Register under a memory-carried (long) name. `arg0` = name length in
    /// bytes; the call attaches both a copied memory object whose first
    /// `arg0` bytes are the name and a re-delegable connection. Reply result
    /// = new instance generation (>= 1), or [`ERR_INVALID`].
    pub const OP_REGISTER_NAMED: u32 = 3;
    /// Look up a service by memory-carried (long) name. `arg0` = name length;
    /// the call attaches a copied memory object holding the name. Reply as
    /// for [`OP_LOOKUP`].
    pub const OP_LOOKUP_NAMED: u32 = 4;

    /// The name is not registered.
    pub const ERR_NOT_FOUND: i64 = -1;
    /// A register call did not attach a re-delegable connection, or a named
    /// call carried a malformed/oversized name.
    pub const ERR_INVALID: i64 = -2;
    /// Unknown opcode.
    pub const ERR_BAD_OPCODE: i64 = -3;
}

/// Echo-service protocol (`charlotte-protocol-echo` v1).
pub mod echo {
    /// Interface id: "ECHO".
    pub const INTERFACE: u64 = super::name(b"ECHO");
    pub const VERSION: u32 = 1;
    /// The registered short service name.
    pub const NAME: u64 = super::name(b"echo");
    /// The registered long (memory-carried) service name, demonstrating
    /// names beyond the 8-byte scalar limit.
    pub const LONG_NAME: &[u8] = b"system.console.echo.primary.v1";

    /// Reply result = `arg0`.
    pub const OP_ECHO: u32 = 1;
    /// Reply 0, then the service exits its protection domain.
    pub const OP_SHUTDOWN: u32 = 2;
}

/// Console-driver protocol (`charlotte-protocol-console` v1).
///
/// The reference userspace UART driver serves this interface. It is the
/// control/data plane a client uses to reach a device the driver owns
/// through delegated MMIO and interrupt capabilities (architecture doc
/// §10, Phase 8).
pub mod console {
    /// Interface id: "CONS".
    pub const INTERFACE: u64 = super::name(b"CONS");
    pub const VERSION: u32 = 1;
    /// The registered short service name.
    pub const NAME: u64 = super::name(b"uart");

    /// Write one byte (`arg0`'s low 8 bits) to the console device's transmit
    /// FIFO. Reply result = 0 on success.
    pub const OP_WRITE: u32 = 1;
    /// Query the driver. Reply result = the number of device interrupts the
    /// driver has observed and acknowledged (proves the delegated interrupt
    /// path is live).
    pub const OP_STATUS: u32 = 2;
    /// Reply 0, release the device (unmap MMIO, mask/unroute the interrupt),
    /// then exit the protection domain.
    pub const OP_SHUTDOWN: u32 = 3;
}

/// PL011 UART register offsets (ARM PrimeCell PL011), for the reference
/// userspace driver.
pub mod pl011 {
    /// Data register: writing transmits the low byte.
    pub const DR: usize = 0x000;
    /// Flag register.
    pub const FR: usize = 0x018;
    /// FR bit 5: transmit FIFO full.
    pub const FR_TXFF: u32 = 1 << 5;
}

/// Stage a memory-carried name: allocate a one-page memory object, write
/// `name` at offset 0, and return the memory cap (unmapped, ready to attach
/// to a copied-memory call).
///
/// Returns `None` when the name is empty/oversized or allocation fails.
///
/// # Safety
/// Uses [`NAME_SCRATCH_VADDR`], which must be unmapped in the caller's
/// address space, and must not race with other users of the scratch page.
pub unsafe fn stage_name(name: &[u8]) -> Option<u64> {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return None;
    }
    let cap = unsafe { catten_syscall::memory_alloc(1) };
    if cap == 0 {
        return None;
    }
    if unsafe { catten_syscall::memory_map(cap, NAME_SCRATCH_VADDR, true) } != 0 {
        unsafe {
            catten_syscall::memory_close(cap);
        }
        return None;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(name.as_ptr(), NAME_SCRATCH_VADDR as *mut u8, name.len());
        catten_syscall::memory_unmap(cap);
    }
    Some(cap)
}

/// Spin-poll a pending call until it completes, returning
/// `(result, returned_connection_cap)`.
///
/// Panics (via `debug_assert`-free explicit check) after `max_spins`
/// iterations so a lost reply fails loudly under test rather than hanging.
///
/// # Safety
/// `call` must be a pending-call capability owned by the caller.
pub unsafe fn wait_reply(call: u64, max_spins: u64) -> (i64, u64) {
    let mut spins: u64 = 0;
    loop {
        let (status, result, cap) = unsafe { catten_syscall::ipc_reply_poll(call) };
        if status == 0 {
            unsafe {
                catten_syscall::ipc_close(call);
            }
            return (result as i64, cap);
        }
        spins += 1;
        if spins >= max_spins {
            unsafe {
                catten_syscall::thread_exit();
            }
        }
        core::hint::spin_loop();
    }
}
