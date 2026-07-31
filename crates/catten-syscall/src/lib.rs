//! Typed `no_std` wrappers for the CharlotteOS (catten) syscall ABI.
//!
//! Each function corresponds to exactly one `svc #N` instruction.
//!
//! ## Register convention
//!
//! The kernel derives the caller's ASID from the running thread context, so
//! `x0` is **not** used for an ASID parameter.  Arguments start at `x1`:
//!
//!   x0 — unused (kernel derives ASID)
//!   x1 — first argument
//!   x2 — second argument
//!   x3 — third argument
//!   x0 — return value (written back by the kernel)
//!   x1 — secondary return value (for MAILBOX_RECV, WAIT_TIMEOUT, etc.)
//!
//! # Example
//!
//! ```ignore
//! use catten_syscall::*;
//!
//! let cap = unsafe { submit(OpCode::Nop) };
//! let (ok, result) = unsafe { wait_timeout(cap, 5000) };
//! unsafe { thread_exit(); } // never returns
//! ```
#![no_std]

#[cfg(target_arch = "aarch64")]
use core::arch::asm;
use core::ops::BitOr;

/// Authoritative CharlotteOS syscall ABI numbers.
///
/// Both userspace wrappers and the kernel dispatcher consume this enum. Adding
/// a variant therefore requires the kernel's exhaustive dispatch match to be
/// updated before the workspace can compile.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyscallNumber {
    Log = 0,
    CompletionSubmit = 1,
    CompletionComplete = 2,
    CompletionPoll = 3,
    CompletionWait = 4,
    CompletionCancel = 5,
    CompletionClose = 6,
    SpawnThread = 7,
    ThreadExit = 8,
    MailboxSend = 9,
    MailboxRecv = 10,
    CompletionWaitTimeout = 11,
    CqWait = 12,
    MailboxOpenSend = 13,
    MailboxOpenRecv = 14,
    MailboxSendCap = 15,
    MailboxRecvCap = 16,
    MailboxClose = 17,
    IpcEndpointCreate = 18,
    IpcConnect = 19,
    IpcScalarSend = 20,
    IpcScalarCall = 21,
    IpcRecv = 22,
    IpcReply = 23,
    IpcReplyPoll = 24,
    IpcClose = 25,
    IpcReplyConnection = 26,
    IpcRecvBlock = 27,
    MemoryAlloc = 28,
    MemoryMap = 29,
    MemoryUnmap = 30,
    MemoryClose = 31,
    IpcScalarSendMove = 32,
    IpcScalarCallMove = 33,
    IpcReplyMove = 34,
    IpcScalarCallBorrowRead = 35,
    IpcScalarCallBorrowWrite = 36,
    IpcScalarSendCopy = 37,
    IpcScalarCallCopy = 38,
    IpcScalarCallConnection = 39,
    IpcScalarCallConnectionCopy = 40,
    CqWake = 41,
    CqWaitTimeout = 42,
    IpcEndpointBindCq = 43,
    DeviceMmioMap = 44,
    DeviceMmioUnmap = 45,
    DeviceIrqBindCq = 46,
    DeviceIrqAck = 47,
    DeviceClose = 48,
    MemoryGetPhys = 49,
    SpawnUpgrade = 50,
    IpcVectorSend = 51,
    IpcVectorCall = 52,
    IpcRecvVec = 53,
    IpcReplyWait = 54,
    CompletionSubmitDetachedTimer = 55,
    MemoryGetPhysPage = 56,
    DmaMap = 57,
    DmaUnmap = 58,
    ThreadStatistics = 59,
}

impl TryFrom<u16> for SyscallNumber {
    type Error = ();

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Log),
            1 => Ok(Self::CompletionSubmit),
            2 => Ok(Self::CompletionComplete),
            3 => Ok(Self::CompletionPoll),
            4 => Ok(Self::CompletionWait),
            5 => Ok(Self::CompletionCancel),
            6 => Ok(Self::CompletionClose),
            7 => Ok(Self::SpawnThread),
            8 => Ok(Self::ThreadExit),
            9 => Ok(Self::MailboxSend),
            10 => Ok(Self::MailboxRecv),
            11 => Ok(Self::CompletionWaitTimeout),
            12 => Ok(Self::CqWait),
            13 => Ok(Self::MailboxOpenSend),
            14 => Ok(Self::MailboxOpenRecv),
            15 => Ok(Self::MailboxSendCap),
            16 => Ok(Self::MailboxRecvCap),
            17 => Ok(Self::MailboxClose),
            18 => Ok(Self::IpcEndpointCreate),
            19 => Ok(Self::IpcConnect),
            20 => Ok(Self::IpcScalarSend),
            21 => Ok(Self::IpcScalarCall),
            22 => Ok(Self::IpcRecv),
            23 => Ok(Self::IpcReply),
            24 => Ok(Self::IpcReplyPoll),
            25 => Ok(Self::IpcClose),
            26 => Ok(Self::IpcReplyConnection),
            27 => Ok(Self::IpcRecvBlock),
            28 => Ok(Self::MemoryAlloc),
            29 => Ok(Self::MemoryMap),
            30 => Ok(Self::MemoryUnmap),
            31 => Ok(Self::MemoryClose),
            32 => Ok(Self::IpcScalarSendMove),
            33 => Ok(Self::IpcScalarCallMove),
            34 => Ok(Self::IpcReplyMove),
            35 => Ok(Self::IpcScalarCallBorrowRead),
            36 => Ok(Self::IpcScalarCallBorrowWrite),
            37 => Ok(Self::IpcScalarSendCopy),
            38 => Ok(Self::IpcScalarCallCopy),
            39 => Ok(Self::IpcScalarCallConnection),
            40 => Ok(Self::IpcScalarCallConnectionCopy),
            41 => Ok(Self::CqWake),
            42 => Ok(Self::CqWaitTimeout),
            43 => Ok(Self::IpcEndpointBindCq),
            44 => Ok(Self::DeviceMmioMap),
            45 => Ok(Self::DeviceMmioUnmap),
            46 => Ok(Self::DeviceIrqBindCq),
            47 => Ok(Self::DeviceIrqAck),
            48 => Ok(Self::DeviceClose),
            49 => Ok(Self::MemoryGetPhys),
            50 => Ok(Self::SpawnUpgrade),
            51 => Ok(Self::IpcVectorSend),
            52 => Ok(Self::IpcVectorCall),
            53 => Ok(Self::IpcRecvVec),
            54 => Ok(Self::IpcReplyWait),
            55 => Ok(Self::CompletionSubmitDetachedTimer),
            56 => Ok(Self::MemoryGetPhysPage),
            57 => Ok(Self::DmaMap),
            58 => Ok(Self::DmaUnmap),
            59 => Ok(Self::ThreadStatistics),
            _ => Err(()),
        }
    }
}

pub const MAX_SYSCALL_NUMBER: u16 = SyscallNumber::ThreadStatistics as u16;

// ---- observability wire format ---------------------------------------------

pub const THREAD_STATISTICS_MAGIC: u64 = 0x3154_4154_534f_4343; // "CCOSTAT1"
pub const THREAD_STATISTICS_VERSION: u64 = 1;
pub const THREAD_STATISTICS_HEADER_U64S: usize = 6;
pub const THREAD_STATISTICS_RECORD_U64S: usize = 16;
pub const OBSERVABILITY_NONE: u64 = u64::MAX;

// ---- op codes --------------------------------------------------------------

/// Operation codes for COMPLETION_SUBMIT.
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpCode {
    Nop = 0,
    Read = 1,
    Write = 2,
    Timer = 3,
}

// ---- endpoint IPC constants -----------------------------------------------

pub type IpcStatusCode = u64;

pub mod ipc_status {
    use super::IpcStatusCode;

    pub const OK: IpcStatusCode = 0;
    pub const QUEUE_FULL: IpcStatusCode = 1;
    pub const NO_MESSAGE: IpcStatusCode = 2;
    pub const PENDING: IpcStatusCode = 3;
    pub const UNKNOWN_CAPABILITY: IpcStatusCode = 4;
    pub const WRONG_TYPE: IpcStatusCode = 5;
    pub const PERMISSION_DENIED: IpcStatusCode = 6;
    pub const ENDPOINT_CLOSED: IpcStatusCode = 7;
    pub const REPLY_ALREADY_USED: IpcStatusCode = 8;
    pub const MEMORY_TRANSFER_FAILED: IpcStatusCode = 9;
}

pub const IPC_REPLY_CANCELLED: i64 = -3;
pub const IPC_REPLY_ENDPOINT_CLOSED: i64 = -7;

pub type MemoryStatusCode = u64;

pub mod memory_status {
    use super::MemoryStatusCode;

    pub const OK: MemoryStatusCode = 0;
    pub const UNKNOWN_CAPABILITY: MemoryStatusCode = 1;
    pub const WRONG_OWNER: MemoryStatusCode = 2;
    pub const ALREADY_MAPPED: MemoryStatusCode = 3;
    pub const NOT_MAPPED: MemoryStatusCode = 4;
    pub const INVALID_LENGTH: MemoryStatusCode = 5;
    pub const NOT_PAGE_ALIGNED: MemoryStatusCode = 6;
    pub const ADDRESS_SPACE_MISSING: MemoryStatusCode = 7;
    pub const MAP_FAILED: MemoryStatusCode = 8;
    pub const UNMAP_FAILED: MemoryStatusCode = 9;
    pub const FRAME_ALLOC_FAILED: MemoryStatusCode = 10;
    pub const FRAME_FREE_FAILED: MemoryStatusCode = 11;
    pub const MISSING_RIGHT: MemoryStatusCode = 12;
    pub const LENDING_ACTIVE: MemoryStatusCode = 13;
    pub const NOT_LENT: MemoryStatusCode = 14;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcRights(u32);

impl IpcRights {
    pub const ALL: Self =
        Self(Self::SEND.0 | Self::CALL.0 | Self::RECEIVE.0 | Self::MINT_CONNECTION.0);
    pub const CALL: Self = Self(1 << 1);
    pub const MINT_CONNECTION: Self = Self(1 << 3);
    pub const RECEIVE: Self = Self(1 << 2);
    pub const SEND: Self = Self(1 << 0);

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl BitOr for IpcRights {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

// ---- raw svc primitives ----------------------------------------------------

/// Issue `svc #imm` with `x1=arg1, x2=arg2, x3=arg3`, return `x0`.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc3(imm: SyscallNumber, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    let ret: u64;
    unsafe {
        match imm as u16 {
            0 => asm!("svc #0", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            1 => asm!("svc #1", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            2 => asm!("svc #2", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            3 => asm!("svc #3", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            4 => asm!("svc #4", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            5 => asm!("svc #5", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            6 => asm!("svc #6", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            7 => asm!("svc #7", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            8 => asm!("svc #8", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            9 => asm!("svc #9", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            10 => asm!("svc #10", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            12 => asm!("svc #12", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            13 => asm!("svc #13", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            14 => asm!("svc #14", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            15 => asm!("svc #15", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            16 => asm!("svc #16", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            17 => asm!("svc #17", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            18 => asm!("svc #18", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            19 => asm!("svc #19", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            20 => asm!("svc #20", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            21 => asm!("svc #21", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            23 => asm!("svc #23", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            25 => asm!("svc #25", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            26 => asm!("svc #26", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            28 => asm!("svc #28", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            29 => asm!("svc #29", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            30 => asm!("svc #30", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            31 => asm!("svc #31", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            34 => asm!("svc #34", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            41 => asm!("svc #41", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            43 => asm!("svc #43", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            44 => asm!("svc #44", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            45 => asm!("svc #45", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            46 => asm!("svc #46", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            48 => asm!("svc #48", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            49 => asm!("svc #49", lateout("x0") ret, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            55 => asm!("svc #55", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            56 => asm!("svc #56", lateout("x0") ret, in("x1") arg1, in("x2") arg2, options(nostack, nomem, preserves_flags)),
            57 => asm!("svc #57", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            58 => asm!("svc #58", lateout("x0") ret, in("x1") arg1, in("x2") arg2, options(nostack, nomem, preserves_flags)),
            _ => panic!("syscall {:?} has no svc3 emitter", imm),
        }
    }
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc4(imm: SyscallNumber, arg1: u64, arg2: u64, arg3: u64, arg4: u64) -> u64 {
    let ret: u64;
    unsafe {
        match imm as u16 {
            26 => asm!("svc #26", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, options(nostack, nomem, preserves_flags)),
            32 => asm!("svc #32", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, options(nostack, nomem, preserves_flags)),
            33 => asm!("svc #33", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, options(nostack, nomem, preserves_flags)),
            35 => asm!("svc #35", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, options(nostack, nomem, preserves_flags)),
            36 => asm!("svc #36", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, options(nostack, nomem, preserves_flags)),
            37 => asm!("svc #37", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, options(nostack, nomem, preserves_flags)),
            38 => asm!("svc #38", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, options(nostack, nomem, preserves_flags)),
            50 => asm!("svc #50", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, options(nostack, nomem, preserves_flags)),
            51 => asm!("svc #51", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, options(nostack, nomem, preserves_flags)),
            52 => asm!("svc #52", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, options(nostack, nomem, preserves_flags)),
            _ => panic!("syscall {:?} has no svc4 emitter", imm),
        }
    }
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc5(imm: SyscallNumber, arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> u64 {
    let ret: u64;
    unsafe {
        match imm as u16 {
            39 => asm!("svc #39", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, in("x5") arg5, options(nostack, nomem, preserves_flags)),
            _ => panic!("syscall {:?} has no svc5 emitter", imm),
        }
    }
    ret
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc6(
    imm: SyscallNumber,
    arg1: u64,
    arg2: u64,
    arg3: u64,
    arg4: u64,
    arg5: u64,
    arg6: u64,
) -> u64 {
    let ret: u64;
    unsafe {
        match imm as u16 {
            40 => asm!("svc #40", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, in("x5") arg5, in("x6") arg6, options(nostack, nomem, preserves_flags)),
            _ => panic!("syscall {:?} has no svc6 emitter", imm),
        }
    }
    ret
}

#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
unsafe fn svc3(_imm: SyscallNumber, _arg1: u64, _arg2: u64, _arg3: u64) -> u64 {
    0
}

#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
unsafe fn svc4(_imm: SyscallNumber, _arg1: u64, _arg2: u64, _arg3: u64, _arg4: u64) -> u64 {
    0
}

#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
unsafe fn svc3_x1(_imm: SyscallNumber, _arg1: u64, _arg2: u64, _arg3: u64) -> (u64, u64) {
    (0, 0)
}

#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
unsafe fn svc3_x2(_imm: SyscallNumber, _arg1: u64, _arg2: u64, _arg3: u64) -> (u64, u64, u64) {
    (0, 0, 0)
}

#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
unsafe fn svc3_x3(_imm: SyscallNumber, _arg1: u64, _arg2: u64, _arg3: u64) -> (u64, u64, u64, u64) {
    (0, 0, 0, 0)
}

#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
unsafe fn svc5(_imm: SyscallNumber, _a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64) -> u64 {
    0
}

#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
unsafe fn svc6(
    _imm: SyscallNumber,
    _a1: u64,
    _a2: u64,
    _a3: u64,
    _a4: u64,
    _a5: u64,
    _a6: u64,
) -> u64 {
    0
}

#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
unsafe fn svc_ipc_recv(_endpoint: u64) -> IpcMessage {
    IpcMessage {
        status: ipc_status::NO_MESSAGE,
        opcode: 0,
        arg0: 0,
        reply: 0,
        sender: 0,
        interface: 0,
        version: 0,
        memory: 0,
        connection: 0,
    }
}

#[cfg(not(target_arch = "aarch64"))]
#[inline(always)]
unsafe fn svc_ipc_recv_block(_endpoint: u64) -> IpcMessage {
    IpcMessage {
        status: ipc_status::ENDPOINT_CLOSED,
        opcode: 0,
        arg0: 0,
        reply: 0,
        sender: 0,
        interface: 0,
        version: 0,
        memory: 0,
        connection: 0,
    }
}

/// Like [`svc3`] but also captures the x1 return value (for syscalls that
/// return a secondary value in x1, e.g. MAILBOX_RECV_CAP, WAIT_TIMEOUT).
#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc3_x1(imm: SyscallNumber, arg1: u64, arg2: u64, _arg3: u64) -> (u64, u64) {
    let ret: u64;
    let x1_out: u64;
    unsafe {
        match imm as u16 {
            3 => asm!("svc #3", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            10 => asm!("svc #10", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            11 => asm!("svc #11", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, in("x2") arg2, options(nostack, nomem, preserves_flags)),
            16 => asm!("svc #16", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            24 => asm!("svc #24", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            42 => asm!("svc #42", lateout("x0") ret, inlateout("x1") arg1 => x1_out, in("x2") arg2, in("x3") _arg3, options(nostack, nomem, preserves_flags)),
             47 => asm!("svc #47", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
             59 => asm!("svc #59", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            _ => panic!("syscall {:?} has no svc3_x1 emitter", imm),
        }
    }
    (ret, x1_out)
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc3_x2(imm: SyscallNumber, arg1: u64, _arg2: u64, _arg3: u64) -> (u64, u64, u64) {
    let ret: u64;
    let x1_out: u64;
    let x2_out: u64;
    unsafe {
        match imm as u16 {
            24 => asm!("mov x1, x4", "svc #24", lateout("x0") ret, lateout("x1") x1_out, lateout("x2") x2_out, in("x4") arg1, options(nostack, nomem, preserves_flags)),
            54 => asm!("mov x1, x4", "svc #54", lateout("x0") ret, lateout("x1") x1_out, lateout("x2") x2_out, in("x4") arg1, options(nostack, nomem, preserves_flags)),
            _ => panic!("syscall {:?} has no svc3_x2 emitter", imm),
        }
    }
    (ret, x1_out, x2_out)
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc3_x3(imm: SyscallNumber, arg1: u64, _arg2: u64, _arg3: u64) -> (u64, u64, u64, u64) {
    let ret: u64;
    let x1_out: u64;
    let x2_out: u64;
    let x3_out: u64;
    unsafe {
        match imm as u16 {
            24 => asm!("mov x1, x4", "svc #24", lateout("x0") ret, lateout("x1") x1_out, lateout("x2") x2_out, lateout("x3") x3_out, in("x4") arg1, options(nostack, nomem, preserves_flags)),
            54 => asm!("mov x1, x4", "svc #54", lateout("x0") ret, lateout("x1") x1_out, lateout("x2") x2_out, lateout("x3") x3_out, in("x4") arg1, options(nostack, nomem, preserves_flags)),
            _ => panic!("syscall {:?} has no svc3_x3 emitter", imm),
        }
    }
    (ret, x1_out, x2_out, x3_out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcMessage {
    pub status: IpcStatusCode,
    pub opcode: u32,
    pub arg0: u64,
    pub reply: u64,
    pub sender: u64,
    pub interface: u64,
    pub version: u32,
    pub memory: u64,
    pub connection: u64,
}

impl IpcMessage {
    pub const fn is_ok(self) -> bool {
        self.status == ipc_status::OK
    }
}

/// Receive a scalar endpoint IPC message from `endpoint`.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc_ipc_recv(endpoint: u64) -> IpcMessage {
    let status: u64;
    let opcode: u64;
    let arg0: u64;
    let reply: u64;
    let sender: u64;
    let interface: u64;
    let version: u64;
    let memory: u64;
    let connection: u64;
    unsafe {
        asm!(
            "svc #22",
            lateout("x0") status,
            inlateout("x1") endpoint => opcode,
            lateout("x2") arg0,
            lateout("x3") reply,
            lateout("x4") sender,
            lateout("x5") interface,
            lateout("x6") version,
            lateout("x7") memory,
            lateout("x8") connection,
            options(nostack, nomem, preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

/// Block until an endpoint IPC message is readable, then receive it.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc_ipc_recv_block(endpoint: u64) -> IpcMessage {
    let status: u64;
    let opcode: u64;
    let arg0: u64;
    let reply: u64;
    let sender: u64;
    let interface: u64;
    let version: u64;
    let memory: u64;
    let connection: u64;
    unsafe {
        asm!(
            "svc #27",
            lateout("x0") status,
            inlateout("x1") endpoint => opcode,
            lateout("x2") arg0,
            lateout("x3") reply,
            lateout("x4") sender,
            lateout("x5") interface,
            lateout("x6") version,
            lateout("x7") memory,
            lateout("x8") connection,
            options(nostack, nomem, preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

// ---- public syscall wrappers ------------------------------------------------
// Most wrappers are safe: the kernel validates all capability arguments and
// returns error codes for invalid inputs. Only operations that take raw
// pointers, target specific LPs, or diverge must stay `unsafe`.

/// Emit a kernel debug log line with two arbitrary values (smoke debugging).
#[inline(always)]
pub fn el0_log(a: u64, b: u64) {
    unsafe {
        svc3(SyscallNumber::Log, a, b, 0);
    }
}

/// Submit an async operation.  Returns a completion capability.
#[inline(always)]
pub fn submit(op: OpCode) -> u64 {
    unsafe { svc3(SyscallNumber::CompletionSubmit, op as u64, 0, 0) }
}

/// Submit a timer operation that completes after `timeout_ms` milliseconds.
/// Returns a completion capability that auto-completes when the timer fires,
/// or `u64::MAX` when submission fails. Capability zero is valid.
#[inline(always)]
pub fn submit_timer(timeout_ms: u64) -> u64 {
    unsafe { svc3(SyscallNumber::CompletionSubmit, OpCode::Timer as u64, 0, timeout_ms) }
}

/// Submit a capability-free timer whose sole completion is a record in `cq`.
///
/// `user_data` is copied into the completion record's cookie. The returned
/// operation ID is only useful for cancellation; `u64::MAX` means submission
/// failed.
#[inline(always)]
pub fn submit_detached_timer(timeout_ms: u64, cq: u32, user_data: u64) -> u64 {
    unsafe { svc3(SyscallNumber::CompletionSubmitDetachedTimer, timeout_ms, cq as u64, user_data) }
}

/// One entry in the shared completion queue ABI.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CqEntry {
    pub operation: u64,
    pub cookie: u64,
    pub status: u32,
    pub flags: u32,
    pub result: i64,
}

#[repr(C)]
struct CompletionQueueRing {
    head: u32,
    tail: u32,
    capacity: u32,
    overflow: u32,
    entries: [CqEntry; 0],
}

/// Read one completion from the kernel-mapped shared CQ page.
///
/// # Safety
/// `base` must be the base address and `entries` the advertised capacity of
/// the caller's mapped completion queue. Only its owning reactor may consume
/// the queue.
#[inline]
pub unsafe fn cq_read(base: usize, entries: u32) -> Option<CqEntry> {
    let ring = base as *mut CompletionQueueRing;
    if ring.is_null() {
        return None;
    }
    let capacity = unsafe { core::ptr::read_volatile(&(*ring).capacity) };
    if capacity < 2 || capacity > entries {
        return None;
    }
    let head = unsafe { core::ptr::read_volatile(&(*ring).head) };
    let tail = unsafe { core::ptr::read_volatile(&(*ring).tail) };
    if head == tail || head >= capacity || tail >= capacity {
        return None;
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::Acquire);
    let entry = unsafe {
        let entries_base = core::ptr::addr_of!((*ring).entries).cast::<CqEntry>();
        core::ptr::read_volatile(entries_base.add(tail as usize))
    };
    unsafe {
        core::ptr::write_volatile(&mut (*ring).tail, (tail + 1) % capacity);
    }
    core::sync::atomic::fence(core::sync::atomic::Ordering::Release);
    Some(entry)
}

/// Submit a Read with a user buffer.  `buf_ptr`/`buf_len` point to a
/// writable buffer in the caller's address space that the kernel fills.
/// Returns the capability.
///
/// # Safety
/// `buf_ptr` must point to a writable buffer of at least `buf_len` bytes
/// in the caller's address space.
#[inline(always)]
pub unsafe fn submit_read(buf_ptr: usize, buf_len: usize) -> u64 {
    unsafe {
        svc3(SyscallNumber::CompletionSubmit, OpCode::Read as u64, buf_ptr as u64, buf_len as u64)
    }
}

/// Post a terminal result for a completion capability.
#[inline(always)]
pub fn complete(cap: u64, result_code: i64) {
    unsafe {
        svc3(SyscallNumber::CompletionComplete, cap, result_code as u64, 0);
    }
}

/// Non-blocking check: drain the completion if it is terminal.
/// Returns `(0, result_code)` when ready and `(1, 0)` while pending.
#[inline(always)]
pub fn poll(cap: u64) -> (u64, u64) {
    unsafe { svc3_x1(SyscallNumber::CompletionPoll, cap, 0, 0) }
}

/// Block until the given capability reaches a terminal completion.
#[inline(always)]
pub fn wait(cap: u64) {
    unsafe {
        svc3(SyscallNumber::CompletionWait, cap, 0, 0);
    }
}

/// Request cancellation of an in-flight capability.
#[inline(always)]
pub fn cancel(cap: u64) {
    unsafe {
        svc3(SyscallNumber::CompletionCancel, cap, 0, 0);
    }
}

/// Release a completed/drained capability slot.
#[inline(always)]
pub fn close(cap: u64) {
    unsafe {
        svc3(SyscallNumber::CompletionClose, cap, 0, 0);
    }
}

/// Spawn a new EL0 thread pinned to `target_lp`, starting at `entry_vaddr`.
/// Returns the kernel-assigned thread id.
///
/// # Safety
/// `entry_vaddr` must point to valid executable code in the caller's address
/// space. `target_lp` must be a valid LP id.
#[inline(always)]
pub unsafe fn spawn_thread(entry_vaddr: usize, target_lp: u32) -> u64 {
    unsafe { svc3(SyscallNumber::SpawnThread, entry_vaddr as u64, target_lp as u64, 0) }
}

/// Terminate the calling EL0 thread.  Never returns.
///
/// # Safety
/// Divergent: must not be called while holding locks or resources that the
/// kernel does not track on thread teardown.
#[inline(always)]
pub unsafe fn thread_exit() -> ! {
    unsafe {
        svc3(SyscallNumber::ThreadExit, 0, 0, 0);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Send a 64-bit message to the target LP's global mailbox.
/// Returns 0 on success, 1 on queue-full.
///
/// # Safety
/// `target_lp` must be a valid LP id.
#[inline(always)]
pub unsafe fn mailbox_send_raw(target_lp: u32, message: u64) -> u64 {
    unsafe { svc3(SyscallNumber::MailboxSend, target_lp as u64, message, 0) }
}

/// Receive a message from the calling LP's global mailbox.
/// Returns `(msg, 0)` on success, `(0, 1)` when empty.
#[inline(always)]
pub fn mailbox_recv_raw() -> (u64, u64) {
    unsafe { svc3_x1(SyscallNumber::MailboxRecv, 0, 0, 0) }
}

/// Block on a capability with a timeout in milliseconds.
/// Returns `(0, result_code)` on completion, `(1, _)` on timeout.
#[inline(always)]
pub fn wait_timeout(cap: u64, timeout_ms: u64) -> (u64, u64) {
    unsafe { svc3_x1(SyscallNumber::CompletionWaitTimeout, cap, timeout_ms, 0) }
}

/// Block until CQ `cq` of the caller has at least `min_complete` pending
/// entries or an explicit wake is posted to it.  Returns the pending count.
#[inline(always)]
pub fn cq_wait(min_complete: u64, cq: u32) -> u64 {
    unsafe { svc3(SyscallNumber::CqWait, min_complete, cq as u64, 0) }
}

/// Post an explicit wake to CQ `cq`'s waiters, so a peer shard blocked in
/// [`cq_wait`]/[`cq_wait_timeout`] on that queue returns even without a
/// completion.
#[inline(always)]
pub fn cq_wake(cq: u32) -> u64 {
    unsafe { svc3(SyscallNumber::CqWake, cq as u64, 0, 0) }
}

/// Block until CQ `cq` of the caller has at least `min_complete` entries, an
/// explicit wake is posted to it, or `timeout_ms` elapses. Returns
/// `(pending, timed_out)` where `timed_out` is 1 if the deadline fired first.
#[inline(always)]
pub fn cq_wait_timeout(min_complete: u64, timeout_ms: u64, cq: u32) -> (u64, u64) {
    unsafe { svc3_x1(SyscallNumber::CqWaitTimeout, min_complete, timeout_ms, cq as u64) }
}

/// Bind an endpoint's readiness to the caller's CQ `cq`: the kernel posts a
/// coalesced wake to that queue when the endpoint's message queue becomes
/// nonempty and when the endpoint closes, so one [`cq_wait`] covers both
/// completions and endpoint work. Returns an IPC status code.
#[inline(always)]
pub fn ipc_endpoint_bind_cq(endpoint: u64, cq: u32) -> u64 {
    unsafe { svc3(SyscallNumber::IpcEndpointBindCq, endpoint, cq as u64, 0) }
}

// ---- device capabilities (userspace drivers) -------------------------------

pub type DeviceStatusCode = u64;

pub mod device_status {
    use super::DeviceStatusCode;

    pub const OK: DeviceStatusCode = 0;
    pub const UNKNOWN_CAPABILITY: DeviceStatusCode = 1;
    pub const WRONG_TYPE: DeviceStatusCode = 2;
    pub const ALREADY_MAPPED: DeviceStatusCode = 3;
    pub const NOT_MAPPED: DeviceStatusCode = 4;
    pub const MAP_FAILED: DeviceStatusCode = 5;
    pub const NOT_BOUND: DeviceStatusCode = 6;
    pub const ALREADY_BOUND: DeviceStatusCode = 7;
    pub const NOT_PAGE_ALIGNED: DeviceStatusCode = 8;
    pub const INVALID_INTERRUPT: DeviceStatusCode = 9;
}

/// Map an MMIO region capability into the caller's address space at
/// `base_vaddr` as device memory. Returns a device status code.
#[inline(always)]
pub fn device_mmio_map(cap: u64, base_vaddr: usize, writable: bool) -> DeviceStatusCode {
    unsafe { svc3(SyscallNumber::DeviceMmioMap, cap, base_vaddr as u64, writable as u64) }
}

/// Unmap an MMIO region capability from the caller. Returns a device status code.
#[inline(always)]
pub fn device_mmio_unmap(cap: u64) -> DeviceStatusCode {
    unsafe { svc3(SyscallNumber::DeviceMmioUnmap, cap, 0, 0) }
}

/// Bind an interrupt capability to the caller's CQ `cq` and arm the source.
/// Delivered interrupts post a coalesced readiness wake to that queue, so one
/// [`cq_wait`] covers device interrupts, completions, and endpoint work.
/// Returns a device status code.
#[inline(always)]
pub fn device_irq_bind_cq(cap: u64, cq: u32) -> DeviceStatusCode {
    unsafe { svc3(SyscallNumber::DeviceIrqBindCq, cap, cq as u64, 0) }
}

/// Acknowledge an interrupt capability: clear its pending count and re-arm the
/// source. Returns `(status, consumed)` where `consumed` is the number of
/// coalesced interrupts observed since the last acknowledgement.
#[inline(always)]
pub fn device_irq_ack(cap: u64) -> (DeviceStatusCode, u64) {
    unsafe { svc3_x1(SyscallNumber::DeviceIrqAck, cap, 0, 0) }
}

/// Close a device capability (unmap an MMIO region or mask and unroute an
/// interrupt). Returns a device status code.
#[inline(always)]
pub fn device_close(cap: u64) -> DeviceStatusCode {
    unsafe { svc3(SyscallNumber::DeviceClose, cap, 0, 0) }
}

/// Open a sender capability targeting LP `target_lp`.  Returns the cap.
///
/// # Safety
/// `target_lp` must be a valid LP id.
#[inline(always)]
pub unsafe fn mailbox_open_send(target_lp: u32) -> u64 {
    unsafe { svc3(SyscallNumber::MailboxOpenSend, target_lp as u64, 0, 0) }
}

/// Open a receiver capability for the calling LP.  Returns the cap.
#[inline(always)]
pub fn mailbox_open_recv() -> u64 {
    unsafe { svc3(SyscallNumber::MailboxOpenRecv, 0, 0, 0) }
}

/// Send a message through a sender capability.
/// Returns 0 on success, 1 on queue-full, 2 on invalid cap.
#[inline(always)]
pub fn mailbox_send(cap: u64, message: u64) -> u64 {
    unsafe { svc3(SyscallNumber::MailboxSendCap, cap, message, 0) }
}

/// Receive a message through a receiver capability.
/// Returns `(msg, 0)` on success, `(0, 1)` when empty, `(0, 2)` on invalid cap.
#[inline(always)]
pub fn mailbox_recv(cap: u64) -> (u64, u64) {
    unsafe { svc3_x1(SyscallNumber::MailboxRecvCap, cap, 0, 0) }
}

/// Close a mailbox capability.  Returns 0 on success, 1 on invalid cap.
#[inline(always)]
pub fn mailbox_close(cap: u64) -> u64 {
    unsafe { svc3(SyscallNumber::MailboxClose, cap, 0, 0) }
}

/// Create an endpoint owned by the caller. Returns endpoint cap, or 0 on error.
#[inline(always)]
pub fn ipc_endpoint_create(interface: u64, version: u32, capacity: usize) -> u64 {
    unsafe { svc3(SyscallNumber::IpcEndpointCreate, interface, version as u64, capacity as u64) }
}

/// Mint a same-address-space connection from an endpoint cap.
#[inline(always)]
pub fn ipc_connect(endpoint: u64, rights: IpcRights) -> u64 {
    unsafe { svc3(SyscallNumber::IpcConnect, endpoint, rights.bits() as u64, 0) }
}

/// Send a scalar message through a connection. Returns status code.
#[inline(always)]
pub fn ipc_scalar_send(connection: u64, opcode: u32, arg0: u64) -> u64 {
    unsafe { svc3(SyscallNumber::IpcScalarSend, connection, opcode as u64, arg0) }
}

/// Call through a connection. Returns pending-call cap, or 0 on error.
#[inline(always)]
pub fn ipc_scalar_call(connection: u64, opcode: u32, arg0: u64) -> u64 {
    unsafe { svc3(SyscallNumber::IpcScalarCall, connection, opcode as u64, arg0) }
}

/// Receive a scalar endpoint IPC message.
#[inline(always)]
pub fn ipc_recv(endpoint: u64) -> IpcMessage {
    unsafe { svc_ipc_recv(endpoint) }
}

/// Block until a scalar endpoint IPC message is readable, then receive it.
#[inline(always)]
pub fn ipc_recv_block(endpoint: u64) -> IpcMessage {
    unsafe { svc_ipc_recv_block(endpoint) }
}

/// Complete a call using a reply-token cap. Returns status code.
#[inline(always)]
pub fn ipc_reply(reply: u64, result: i64) -> u64 {
    unsafe { svc3(SyscallNumber::IpcReply, reply, result as u64, 0) }
}

/// Poll a pending-call cap.
///
/// Returns `(0, result, returned_cap)` when ready, where `returned_cap` is 0
/// when the reply did not delegate a capability. Returns `(1, 0, 0)` while pending.
#[inline(always)]
pub fn ipc_reply_poll(call: u64) -> (u64, u64, u64) {
    unsafe { svc3_x2(SyscallNumber::IpcReplyPoll, call, 0, 0) }
}

/// Poll a pending-call cap, including any returned memory-object cap.
///
/// Returns `(0, result, returned_connection, returned_memory)` when ready.
/// Either returned cap is 0 when absent.
#[inline(always)]
pub fn ipc_reply_poll_with_memory(call: u64) -> (u64, u64, u64, u64) {
    unsafe { svc3_x3(SyscallNumber::IpcReplyPoll, call, 0, 0) }
}

/// Block until a pending call completes.
///
/// Returns `(0, result, returned_cap)` when ready.
#[inline(always)]
pub fn ipc_reply_wait(call: u64) -> (u64, u64, u64) {
    unsafe { svc3_x2(SyscallNumber::IpcReplyWait, call, 0, 0) }
}

/// Block until a pending call completes, including any returned memory cap.
#[inline(always)]
pub fn ipc_reply_wait_with_memory(call: u64) -> (u64, u64, u64, u64) {
    unsafe { svc3_x3(SyscallNumber::IpcReplyWait, call, 0, 0) }
}

/// Close an endpoint IPC capability. Returns status code.
#[inline(always)]
pub fn ipc_close(cap: u64) -> u64 {
    unsafe { svc3(SyscallNumber::IpcClose, cap, 0, 0) }
}

/// Complete a call and return a delegated connection cap to the original caller.
#[inline(always)]
pub fn ipc_reply_connection(reply: u64, endpoint: u64, rights: IpcRights, result: i64) -> u64 {
    unsafe {
        svc4(
            SyscallNumber::IpcReplyConnection,
            reply,
            endpoint,
            rights.bits() as u64,
            result as u64,
        )
    }
}

/// Allocate a first-class memory object owned by the caller.
#[inline(always)]
pub fn memory_alloc(pages: usize) -> u64 {
    unsafe { svc3(SyscallNumber::MemoryAlloc, pages as u64, 0, 0) }
}

/// Map a memory object at `base_vaddr`. Returns a memory status code.
#[inline(always)]
pub fn memory_map(cap: u64, base_vaddr: usize, writable: bool) -> MemoryStatusCode {
    unsafe { svc3(SyscallNumber::MemoryMap, cap, base_vaddr as u64, writable as u64) }
}

/// Unmap a memory object from the caller. Returns a memory status code.
#[inline(always)]
pub fn memory_unmap(cap: u64) -> MemoryStatusCode {
    unsafe { svc3(SyscallNumber::MemoryUnmap, cap, 0, 0) }
}

/// Close a memory object cap. Returns a memory status code.
#[inline(always)]
pub fn memory_close(cap: u64) -> MemoryStatusCode {
    unsafe { svc3(SyscallNumber::MemoryClose, cap, 0, 0) }
}

/// Return the physical base address of the first frame of memory object
/// `cap`, or 0 on error. Owned and IPC-borrowed caps are accepted.
#[inline(always)]
pub fn memory_get_phys(cap: u64) -> u64 {
    memory_get_phys_page(cap, 0)
}

/// Return the physical address of page `page_index` in memory object `cap`,
/// or 0 if the capability is invalid or the index is out of bounds. Owned and
/// IPC-borrowed capabilities are accepted.
#[inline(always)]
pub fn memory_get_phys_page(cap: u64, page_index: usize) -> u64 {
    unsafe { svc3(SyscallNumber::MemoryGetPhysPage, cap, page_index as u64, 0) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum DmaDirection {
    DeviceRead = 1,
    DeviceWrite = 2,
    Bidirectional = 3,
}

/// Pin `memory` into `domain` and return its base I/O virtual address.
#[inline(always)]
pub fn dma_map(domain: u64, memory: u64, direction: DmaDirection) -> u64 {
    unsafe { svc3(SyscallNumber::DmaMap, domain, memory, direction as u64) }
}

/// Remove an IOVA mapping, synchronously invalidate the IOTLB, and unpin it.
#[inline(always)]
pub fn dma_unmap(domain: u64, iova: u64) -> DeviceStatusCode {
    unsafe { svc3(SyscallNumber::DmaUnmap, domain, iova, 0) }
}

/// Request the supervisor to spawn a replacement domain (syscall 50).
/// `elf_cap` and `elf_size` identify a persistent ELF memory object. When
/// `elf_cap` is zero, `elf_size` is interpreted as an embedded-image selector,
/// `state_cap` is moved to the replacement, and `target_connection` proves
/// that the authorized service manager can address the service being replaced.
/// Returns the new domain's ASID or 0 on failure.
///
/// # Safety
/// `state_cap` must be a valid transferable memory-object capability and
/// `target_connection` must be a live callable connection held by the
/// authorized service-manager domain.
#[inline(always)]
pub unsafe fn spawn_upgrade(
    elf_cap: u64,
    elf_size: u64,
    state_cap: u64,
    target_connection: u64,
) -> u64 {
    unsafe { svc4(SyscallNumber::SpawnUpgrade, elf_cap, elf_size, state_cap, target_connection) }
}

/// Send a scalar message and move a memory object to the receiver.
#[inline(always)]
pub fn ipc_scalar_send_move(connection: u64, opcode: u32, arg0: u64, memory: u64) -> IpcStatusCode {
    unsafe { svc4(SyscallNumber::IpcScalarSendMove, connection, opcode as u64, arg0, memory) }
}

/// Call through a connection and move a memory object to the receiver.
#[inline(always)]
pub fn ipc_scalar_call_move(connection: u64, opcode: u32, arg0: u64, memory: u64) -> u64 {
    unsafe { svc4(SyscallNumber::IpcScalarCallMove, connection, opcode as u64, arg0, memory) }
}

/// Reply to a call and move a memory object back to the caller.
#[inline(always)]
pub fn ipc_reply_move(reply: u64, memory: u64, result: i64) -> IpcStatusCode {
    unsafe { svc3(SyscallNumber::IpcReplyMove, reply, memory, result as u64) }
}

/// Call through a connection with a reply-bound immutable memory borrow.
#[inline(always)]
pub fn ipc_scalar_call_borrow_read(connection: u64, opcode: u32, arg0: u64, memory: u64) -> u64 {
    unsafe { svc4(SyscallNumber::IpcScalarCallBorrowRead, connection, opcode as u64, arg0, memory) }
}

/// Call through a connection with a reply-bound writable memory borrow.
#[inline(always)]
pub fn ipc_scalar_call_borrow_write(connection: u64, opcode: u32, arg0: u64, memory: u64) -> u64 {
    unsafe {
        svc4(SyscallNumber::IpcScalarCallBorrowWrite, connection, opcode as u64, arg0, memory)
    }
}

/// Send a scalar message with a copied memory object.
#[inline(always)]
pub fn ipc_scalar_send_copy(connection: u64, opcode: u32, arg0: u64, memory: u64) -> IpcStatusCode {
    unsafe { svc4(SyscallNumber::IpcScalarSendCopy, connection, opcode as u64, arg0, memory) }
}

/// Call through a connection with a copied memory object.
#[inline(always)]
pub fn ipc_scalar_call_copy(connection: u64, opcode: u32, arg0: u64, memory: u64) -> u64 {
    unsafe { svc4(SyscallNumber::IpcScalarCallCopy, connection, opcode as u64, arg0, memory) }
}

/// Call through a connection carrying a delegated connection capability.
///
/// `delegate` must be an endpoint cap or a re-delegable connection cap owned
/// by the caller and bearing `MINT_CONNECTION`. The receiver observes the
/// minted connection cap in the `connection` field of the received message.
/// Returns the pending-call cap, or 0 on error.
#[inline(always)]
pub fn ipc_scalar_call_connection(
    connection: u64,
    opcode: u32,
    arg0: u64,
    delegate: u64,
    rights: IpcRights,
) -> u64 {
    unsafe {
        svc5(
            SyscallNumber::IpcScalarCallConnection,
            connection,
            opcode as u64,
            arg0,
            delegate,
            rights.bits() as u64,
        )
    }
}

/// Call through a connection carrying both a delegated connection capability
/// and a copied memory object.
///
/// The receiver observes the copied memory cap in the `memory` field and the
/// minted connection cap in the `connection` field of the received message.
/// This is the combined-attachment primitive used to register services under
/// memory-carried (long) names. Returns the pending-call cap, or 0 on error.
#[inline(always)]
pub fn ipc_scalar_call_connection_copy(
    connection: u64,
    opcode: u32,
    arg0: u64,
    delegate: u64,
    rights: IpcRights,
    memory: u64,
) -> u64 {
    unsafe {
        svc6(
            SyscallNumber::IpcScalarCallConnectionCopy,
            connection,
            opcode as u64,
            arg0,
            delegate,
            rights.bits() as u64,
            memory,
        )
    }
}

/// Send a vector of memory-object caps through a connection.  `x4` is a
/// memory-object cap holding a packed [`CapVectorEntry`] array.
/// Returns an IPC status code in x0.
#[inline(always)]
pub fn ipc_vector_send(connection: u64, opcode: u32, arg0: u64, cap_vector: u64) -> IpcStatusCode {
    unsafe { svc4(SyscallNumber::IpcVectorSend, connection, opcode as u64, arg0, cap_vector) }
}

/// Call carrying a vector of memory-object caps.  `x4` is a memory-object
/// cap holding a packed [`CapVectorEntry`] array.  Returns the pending-call
/// cap in x0, or 0 on error.
#[inline(always)]
pub fn ipc_vector_call(connection: u64, opcode: u32, arg0: u64, cap_vector: u64) -> u64 {
    unsafe { svc4(SyscallNumber::IpcVectorCall, connection, opcode as u64, arg0, cap_vector) }
}

/// Receive a message and fill a result page with delivered cap IDs.
/// `x1` = endpoint cap, `x3` = result page cap (mapped writable).
/// Returns the same 9-register shape as [`ipc_recv`], and the result
/// page contents are updated with the cap IDs.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
/// # Safety
///
/// `result_page` must name a writable memory-object capability containing
/// enough space for the kernel's packed capability-vector result.
pub unsafe fn ipc_recv_vec(endpoint: u64, result_page: u64) -> IpcMessage {
    let status: u64;
    let opcode: u64;
    let arg0: u64;
    let reply: u64;
    let sender: u64;
    let interface: u64;
    let version: u64;
    let memory: u64;
    let connection: u64;
    unsafe {
        asm!(
            "svc #53",
            lateout("x0") status,
            inlateout("x1") endpoint => opcode,
            inlateout("x2") result_page => arg0,
            lateout("x3") reply,
            lateout("x4") sender,
            lateout("x5") interface,
            lateout("x6") version,
            lateout("x7") memory,
            lateout("x8") connection,
            options(nostack, nomem, preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

#[cfg(not(target_arch = "aarch64"))]
/// # Safety
///
/// Mirrors the AArch64 ABI: `result_page` must identify a writable result
/// page when this syscall is implemented on the target architecture.
pub unsafe fn ipc_recv_vec(_endpoint: u64, _result_page: u64) -> IpcMessage {
    IpcMessage {
        status: ipc_status::NO_MESSAGE,
        opcode: 0,
        arg0: 0,
        reply: 0,
        sender: 0,
        interface: 0,
        version: 0,
        memory: 0,
        connection: 0,
    }
}

/// Snapshot scheduler statistics. With capability zero, the result contains
/// only threads owned by the caller. A delegated system-observer capability
/// authorizes a machine-wide snapshot. Returns
/// `(memory_object_capability, exact_byte_length)`.
#[inline]
pub fn thread_statistics_snapshot(system_observer: u64) -> (u64, u64) {
    unsafe { svc3_x1(SyscallNumber::ThreadStatistics, system_observer, 0, 0) }
}
