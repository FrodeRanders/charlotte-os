//! Shared protocol definitions for the reference CharlotteOS services.
//!
//! This is the userspace half of the Phase 3 name-service architecture: the
//! kernel moves opaque capabilities, while interface ids, opcodes, names,
//! generations, and lookup policy are defined here.
#![no_std]

/// Pack up to 8 ASCII bytes into a u64 service name (little-endian).
pub const fn name(bytes: &[u8]) -> u64 {
    let mut packed = [0u8; 8];
    let mut i = 0;
    while i < bytes.len() && i < 8 {
        packed[i] = bytes[i];
        i += 1;
    }
    u64::from_le_bytes(packed)
}

/// Name-service protocol (`charlotte-protocol-name` v1).
pub mod ns {
    /// Interface id: "NAME".
    pub const INTERFACE: u64 = super::name(b"NAME");
    pub const VERSION: u32 = 1;

    /// Register a service. `arg0` = packed name; the call must attach a
    /// re-delegable connection (`SEND | CALL | MINT_CONNECTION`) to the
    /// service's endpoint. Reply result = new instance generation (>= 1).
    pub const OP_REGISTER: u32 = 1;
    /// Look up a service. `arg0` = packed name. Reply result = current
    /// generation with an attenuated (`SEND | CALL`) connection cap
    /// attached, or [`ERR_NOT_FOUND`].
    pub const OP_LOOKUP: u32 = 2;

    /// The name is not registered.
    pub const ERR_NOT_FOUND: i64 = -1;
    /// A register call did not attach a re-delegable connection.
    pub const ERR_INVALID: i64 = -2;
    /// Unknown opcode.
    pub const ERR_BAD_OPCODE: i64 = -3;
}

/// Echo-service protocol (`charlotte-protocol-echo` v1).
pub mod echo {
    /// Interface id: "ECHO".
    pub const INTERFACE: u64 = super::name(b"ECHO");
    pub const VERSION: u32 = 1;
    /// The registered service name.
    pub const NAME: u64 = super::name(b"echo");

    /// Reply result = `arg0`.
    pub const OP_ECHO: u32 = 1;
    /// Reply 0, then the service exits its protection domain.
    pub const OP_SHUTDOWN: u32 = 2;
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
