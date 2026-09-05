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

#[cfg(any(target_arch = "aarch64", target_arch = "x86_64"))]
use core::arch::asm;
use core::ops::BitOr;

/// Authoritative CharlotteOS syscall ABI numbers.
///
/// Both userspace wrappers and the kernel dispatcher consume this enum. Adding
/// a variant therefore requires the kernel's exhaustive dispatch match to be
/// updated before the workspace can compile.
///
/// The full list (and its [`MAX_SYSCALL_NUMBER`]) is defined in one place by
/// [`define_syscall_numbers!`], so the accepted syscall range can never drift
/// out of sync with the enum.
macro_rules! define_syscall_numbers {
    ($(($variant:ident, $number:expr)),+ $(,)?) => {
        #[repr(u16)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum SyscallNumber {
            $($variant = $number),+
        }

        impl TryFrom<u16> for SyscallNumber {
            type Error = ();

            fn try_from(value: u16) -> Result<Self, Self::Error> {
                match value {
                    $($number => Ok(Self::$variant),)+
                    _ => Err(()),
                }
            }
        }

        /// The largest assigned syscall number.
        ///
        /// Derived from the list above at compile time, so adding a new syscall
        /// automatically extends the accepted range — there is deliberately no
        /// hand-maintained copy of this value to update.
        pub const MAX_SYSCALL_NUMBER: u16 = {
            let mut max = 0u16;
            $( if ($number as u16) > max { max = $number as u16; } )+
            max
        };
    };
}

define_syscall_numbers!(
    (Log, 0),
    (CompletionSubmit, 1),
    (CompletionComplete, 2),
    (CompletionPoll, 3),
    (CompletionWait, 4),
    (CompletionCancel, 5),
    (CompletionClose, 6),
    (SpawnThread, 7),
    (ThreadExit, 8),
    (MailboxSend, 9),
    (MailboxRecv, 10),
    (CompletionWaitTimeout, 11),
    (CqWait, 12),
    (MailboxOpenSend, 13),
    (MailboxOpenRecv, 14),
    (MailboxSendCap, 15),
    (MailboxRecvCap, 16),
    (MailboxClose, 17),
    (IpcEndpointCreate, 18),
    (IpcConnect, 19),
    (IpcScalarSend, 20),
    (IpcScalarCall, 21),
    (IpcRecv, 22),
    (IpcReply, 23),
    (IpcReplyPoll, 24),
    (IpcClose, 25),
    (IpcReplyConnection, 26),
    (IpcRecvBlock, 27),
    (MemoryAlloc, 28),
    (MemoryMap, 29),
    (MemoryUnmap, 30),
    (MemoryClose, 31),
    (IpcScalarSendMove, 32),
    (IpcScalarCallMove, 33),
    (IpcReplyMove, 34),
    (IpcScalarCallBorrowRead, 35),
    (IpcScalarCallBorrowWrite, 36),
    (IpcScalarSendCopy, 37),
    (IpcScalarCallCopy, 38),
    (IpcScalarCallConnection, 39),
    (IpcScalarCallConnectionCopy, 40),
    (CqWake, 41),
    (CqWaitTimeout, 42),
    (IpcEndpointBindCq, 43),
    (DeviceMmioMap, 44),
    (DeviceMmioUnmap, 45),
    (DeviceIrqBindCq, 46),
    (DeviceIrqAck, 47),
    (DeviceClose, 48),
    (MemoryGetPhys, 49),
    (SpawnUpgrade, 50),
    (IpcVectorSend, 51),
    (IpcVectorCall, 52),
    (IpcRecvVec, 53),
    (IpcReplyWait, 54),
    (CompletionSubmitDetachedTimer, 55),
    (MemoryGetPhysPage, 56),
    (DmaMap, 57),
    (DmaUnmap, 58),
    (ThreadStatistics, 59),
    (IpcConnectionWatchClosed, 60),
    (ObserveThreadExit, 61),
    (GetTid, 62),
    (MemorySize, 63),
    (SpawnArtifact, 64),
    (RetireArtifact, 65),
    (MemoryMapAny, 66),
    (DeviceMmioMapAny, 67),
    (DomainAbort, 68),
    (DmaMapExclusive, 69),
    (GetDomainIdentity, 70),
    (IpcRecvAuthenticated, 71),
    (IpcRecvBlockAuthenticated, 72),
    (IpcRecvVecAuthenticated, 73),
    (LogStr, 74),
    (RandomU64, 75),
    (SpawnArtifactScoped, 76),
    (SpawnOperationalConnector, 77),
    (RequestNodeShutdown, 78),
);

/// Supervisor-assigned roles carried in the kernel-authenticated IPC sender
/// envelope. These bits intentionally match `charlotte_authorization::Roles`.
pub mod domain_roles {
    pub const POLICY_ADMIN: u32 = 1 << 0;
    pub const SERVICE_MANAGER: u32 = 1 << 1;
}

/// Reasons accepted by the deployment-agent retirement gate. These values
/// are mirrored into the protected-domain lifecycle record.
pub mod artifact_retirement_reason {
    pub const DEPLOYMENT_RETIRED: u32 = 1;
    pub const NODE_SHUTDOWN: u32 = 2;
}

/// Exact identity and stable policy principal of the calling domain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DomainIdentityInfo {
    pub asid: u64,
    pub generation: u64,
    pub principal: u64,
    pub roles: u32,
}

// ---- observability wire format ---------------------------------------------

pub const THREAD_STATISTICS_MAGIC: u64 = 0x3154_4154_534f_4343; // "CCOSTAT1"
pub const THREAD_STATISTICS_VERSION: u64 = 1;

pub mod thread_statistics_header {
    pub const MAGIC: usize = 0;
    pub const VERSION: usize = 1;
    pub const RECORD_BYTES: usize = 2;
    pub const RECORD_COUNT: usize = 3;
    pub const COUNTER_FREQUENCY_HZ: usize = 4;
    pub const MONOTONIC_TICKS: usize = 5;
    pub const WORDS: usize = 6;
}

pub mod thread_statistics_record {
    pub const TID: usize = 0;
    pub const GENERATION: usize = 1;
    pub const ASID: usize = 2;
    pub const STATE: usize = 3;
    pub const AFFINITY_LP: usize = 4;
    pub const PINNED_LP: usize = 5;
    pub const DISPATCH_COUNT: usize = 6;
    pub const SAMPLE_COUNT: usize = 7;
    pub const MIN_TICKS: usize = 8;
    pub const MAX_TICKS: usize = 9;
    pub const TOTAL_TICKS_LOW: usize = 10;
    pub const TOTAL_TICKS_HIGH: usize = 11;
    pub const SUM_OF_SQUARES_LOW: usize = 12;
    pub const SUM_OF_SQUARES_HIGH: usize = 13;
    pub const SATURATED: usize = 14;
    pub const CURRENT_SLICE_STARTED_AT: usize = 15;
    pub const WORDS: usize = 16;
}

pub const THREAD_STATISTICS_HEADER_U64S: usize = thread_statistics_header::WORDS;
pub const THREAD_STATISTICS_RECORD_U64S: usize = thread_statistics_record::WORDS;
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

/// Sentinel returned by completion submission when validation or bounded
/// admission fails. Capability IDs never use this value.
pub const COMPLETION_SUBMIT_FAILED: u64 = u64::MAX;

pub type CompletionStatusCode = u64;

/// Status values returned in `x0` by completion poll/wait-with-timeout.
pub mod completion_status {
    use super::CompletionStatusCode;

    pub const READY: CompletionStatusCode = 0;
    pub const PENDING_OR_TIMEOUT: CompletionStatusCode = 1;
    pub const INVALID_CAPABILITY: CompletionStatusCode = 2;
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

/// One entry in the packed capability-vector page consumed by vector IPC.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapVectorEntry {
    pub cap: u64,
    /// 0=Copy, 1=Move, 2=BorrowRead, 3=BorrowWrite.
    pub mode: u32,
    pub reserved: u32,
}

pub const CAP_VECTOR_MAX: usize =
    (4096 - core::mem::size_of::<u16>()) / core::mem::size_of::<CapVectorEntry>();

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
            60 => asm!("svc #60", lateout("x0") ret, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            61 => asm!("svc #61", lateout("x0") ret, in("x1") arg1, in("x2") arg2, options(nostack, nomem, preserves_flags)),
            62 => asm!("svc #62", lateout("x0") ret, options(nostack, nomem, preserves_flags)),
            63 => asm!("svc #63", lateout("x0") ret, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            64 => asm!("svc #64", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            68 => asm!("svc #68", lateout("x0") ret, options(nostack, nomem, preserves_flags)),
            69 => asm!("svc #69", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            74 => asm!("svc #74", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
            78 => asm!("svc #78", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, options(nostack, nomem, preserves_flags)),
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
            65 => asm!("svc #65", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, options(nostack, nomem, preserves_flags)),
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
            76 => asm!("svc #76", lateout("x0") ret, in("x1") arg1, in("x2") arg2, in("x3") arg3, in("x4") arg4, in("x5") arg5, options(nostack, nomem, preserves_flags)),
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

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn svc3(imm: SyscallNumber, arg1: u64, arg2: u64, arg3: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") imm as u64 => ret,
            in("rdi") arg1,
            in("rsi") arg2,
            in("rdx") arg3,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags),
        );
    }
    ret
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn svc4(imm: SyscallNumber, arg1: u64, arg2: u64, arg3: u64, arg4: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") imm as u64 => ret,
            in("rdi") arg1,
            in("rsi") arg2,
            in("rdx") arg3,
            in("r10") arg4,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags),
        );
    }
    ret
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn svc5(imm: SyscallNumber, arg1: u64, arg2: u64, arg3: u64, arg4: u64, arg5: u64) -> u64 {
    let ret: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") imm as u64 => ret,
            in("rdi") arg1,
            in("rsi") arg2,
            in("rdx") arg3,
            in("r10") arg4,
            in("r8") arg5,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags),
        );
    }
    ret
}

#[cfg(target_arch = "x86_64")]
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
        asm!(
            "syscall",
            inlateout("rax") imm as u64 => ret,
            in("rdi") arg1,
            in("rsi") arg2,
            in("rdx") arg3,
            in("r10") arg4,
            in("r8") arg5,
            in("r9") arg6,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags),
        );
    }
    ret
}

/// Like [`svc3`] but also captures the `regs[1]` (rdi) return value.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn svc3_x1(imm: SyscallNumber, arg1: u64, arg2: u64, arg3: u64) -> (u64, u64) {
    let ret: u64;
    let x1_out: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") imm as u64 => ret,
            inlateout("rdi") arg1 => x1_out,
            in("rsi") arg2,
            in("rdx") arg3,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags),
        );
    }
    (ret, x1_out)
}

/// Like [`svc3`] but also captures the `regs[1]`/`regs[2]` (rdi/rsi) returns.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn svc3_x2(imm: SyscallNumber, arg1: u64, arg2: u64, arg3: u64) -> (u64, u64, u64) {
    let ret: u64;
    let x1_out: u64;
    let x2_out: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") imm as u64 => ret,
            inlateout("rdi") arg1 => x1_out,
            inlateout("rsi") arg2 => x2_out,
            in("rdx") arg3,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags),
        );
    }
    (ret, x1_out, x2_out)
}

/// Like [`svc3`] but also captures the `regs[1]`/`regs[2]`/`regs[3]` returns.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn svc3_x3(imm: SyscallNumber, arg1: u64, arg2: u64, arg3: u64) -> (u64, u64, u64, u64) {
    let ret: u64;
    let x1_out: u64;
    let x2_out: u64;
    let x3_out: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") imm as u64 => ret,
            inlateout("rdi") arg1 => x1_out,
            inlateout("rsi") arg2 => x2_out,
            inlateout("rdx") arg3 => x3_out,
            lateout("rcx") _,
            lateout("r11") _,
            options(preserves_flags),
        );
    }
    (ret, x1_out, x2_out, x3_out)
}

#[cfg(target_arch = "x86_64")]
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
            "syscall",
            inlateout("rax") SyscallNumber::IpcRecv as u64 => status,
            inlateout("rdi") endpoint => opcode,
            lateout("rsi") arg0,
            lateout("rdx") reply,
            lateout("r10") sender,
            lateout("r8") interface,
            lateout("r9") version,
            lateout("r11") memory,
            lateout("rcx") connection,
            options(preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation: 0,
        sender_principal: 0,
        sender_roles: 0,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

#[cfg(target_arch = "x86_64")]
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
            "syscall",
            inlateout("rax") SyscallNumber::IpcRecvBlock as u64 => status,
            inlateout("rdi") endpoint => opcode,
            lateout("rsi") arg0,
            lateout("rdx") reply,
            lateout("r10") sender,
            lateout("r8") interface,
            lateout("r9") version,
            lateout("r11") memory,
            lateout("rcx") connection,
            options(preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation: 0,
        sender_principal: 0,
        sender_roles: 0,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn svc_ipc_recv_authenticated(endpoint: u64) -> IpcMessage {
    let status: u64;
    let opcode: u64;
    let arg0: u64;
    let reply: u64;
    let sender: u64;
    let interface: u64;
    let version: u64;
    let memory: u64;
    let connection: u64;
    let sender_generation: u64;
    let sender_principal: u64;
    let sender_roles: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") SyscallNumber::IpcRecvAuthenticated as u64 => status,
            inlateout("rdi") endpoint => opcode,
            lateout("rsi") arg0,
            lateout("rdx") reply,
            lateout("r10") sender,
            lateout("r8") interface,
            lateout("r9") version,
            lateout("r11") memory,
            lateout("rcx") connection,
            lateout("r12") sender_generation,
            lateout("r13") sender_principal,
            lateout("r14") sender_roles,
            options(preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation,
        sender_principal,
        sender_roles: sender_roles as u32,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
unsafe fn svc_ipc_recv_block_authenticated(endpoint: u64) -> IpcMessage {
    let status: u64;
    let opcode: u64;
    let arg0: u64;
    let reply: u64;
    let sender: u64;
    let interface: u64;
    let version: u64;
    let memory: u64;
    let connection: u64;
    let sender_generation: u64;
    let sender_principal: u64;
    let sender_roles: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") SyscallNumber::IpcRecvBlockAuthenticated as u64 => status,
            inlateout("rdi") endpoint => opcode,
            lateout("rsi") arg0,
            lateout("rdx") reply,
            lateout("r10") sender,
            lateout("r8") interface,
            lateout("r9") version,
            lateout("r11") memory,
            lateout("rcx") connection,
            lateout("r12") sender_generation,
            lateout("r13") sender_principal,
            lateout("r14") sender_roles,
            options(preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation,
        sender_principal,
        sender_roles: sender_roles as u32,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline(always)]
unsafe fn svc3(_imm: SyscallNumber, _arg1: u64, _arg2: u64, _arg3: u64) -> u64 {
    0
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline(always)]
unsafe fn svc4(_imm: SyscallNumber, _arg1: u64, _arg2: u64, _arg3: u64, _arg4: u64) -> u64 {
    0
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline(always)]
unsafe fn svc5(_imm: SyscallNumber, _a1: u64, _a2: u64, _a3: u64, _a4: u64, _a5: u64) -> u64 {
    0
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
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

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline(always)]
unsafe fn svc3_x1(_imm: SyscallNumber, _arg1: u64, _arg2: u64, _arg3: u64) -> (u64, u64) {
    (0, 0)
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline(always)]
unsafe fn svc3_x2(_imm: SyscallNumber, _arg1: u64, _arg2: u64, _arg3: u64) -> (u64, u64, u64) {
    (0, 0, 0)
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline(always)]
unsafe fn svc3_x3(_imm: SyscallNumber, _arg1: u64, _arg2: u64, _arg3: u64) -> (u64, u64, u64, u64) {
    (0, 0, 0, 0)
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline(always)]
unsafe fn svc_ipc_recv(_endpoint: u64) -> IpcMessage {
    IpcMessage {
        status: ipc_status::NO_MESSAGE,
        opcode: 0,
        arg0: 0,
        reply: 0,
        sender: 0,
        sender_generation: 0,
        sender_principal: 0,
        sender_roles: 0,
        interface: 0,
        version: 0,
        memory: 0,
        connection: 0,
    }
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline(always)]
unsafe fn svc_ipc_recv_block(_endpoint: u64) -> IpcMessage {
    IpcMessage {
        status: ipc_status::ENDPOINT_CLOSED,
        opcode: 0,
        arg0: 0,
        reply: 0,
        sender: 0,
        sender_generation: 0,
        sender_principal: 0,
        sender_roles: 0,
        interface: 0,
        version: 0,
        memory: 0,
        connection: 0,
    }
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline(always)]
unsafe fn svc_ipc_recv_authenticated(endpoint: u64) -> IpcMessage {
    unsafe { svc_ipc_recv(endpoint) }
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
#[inline(always)]
unsafe fn svc_ipc_recv_block_authenticated(endpoint: u64) -> IpcMessage {
    unsafe { svc_ipc_recv_block(endpoint) }
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
            7 => asm!("svc #7", lateout("x0") ret, inlateout("x1") arg1 => x1_out, in("x2") arg2, options(nostack, nomem, preserves_flags)),
            10 => asm!("svc #10", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            11 => asm!("svc #11", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, in("x2") arg2, options(nostack, nomem, preserves_flags)),
            16 => asm!("svc #16", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            24 => asm!("svc #24", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            42 => asm!("svc #42", lateout("x0") ret, inlateout("x1") arg1 => x1_out, in("x2") arg2, in("x3") _arg3, options(nostack, nomem, preserves_flags)),
             47 => asm!("svc #47", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            59 => asm!("svc #59", lateout("x0") ret, lateout("x1") x1_out, in("x1") arg1, options(nostack, nomem, preserves_flags)),
            75 => asm!("svc #75", lateout("x0") ret, lateout("x1") x1_out, options(nostack, nomem, preserves_flags)),
            _ => panic!("syscall {:?} has no svc3_x1 emitter", imm),
        }
    }
    (ret, x1_out)
}

#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc3_x2(imm: SyscallNumber, arg1: u64, arg2: u64, _arg3: u64) -> (u64, u64, u64) {
    let ret: u64;
    let x1_out: u64;
    let x2_out: u64;
    unsafe {
        match imm as u16 {
            24 => asm!("mov x1, x4", "svc #24", lateout("x0") ret, lateout("x1") x1_out, lateout("x2") x2_out, in("x4") arg1, options(nostack, nomem, preserves_flags)),
            54 => asm!("mov x1, x4", "svc #54", lateout("x0") ret, lateout("x1") x1_out, lateout("x2") x2_out, in("x4") arg1, options(nostack, nomem, preserves_flags)),
            66 => asm!("svc #66", lateout("x0") ret, lateout("x1") x1_out, lateout("x2") x2_out, in("x1") arg1, in("x2") arg2, options(nostack, nomem, preserves_flags)),
            67 => asm!("svc #67", lateout("x0") ret, lateout("x1") x1_out, lateout("x2") x2_out, in("x1") arg1, in("x2") arg2, options(nostack, nomem, preserves_flags)),
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
            70 => asm!("svc #70", lateout("x0") ret, lateout("x1") x1_out, lateout("x2") x2_out, lateout("x3") x3_out, options(nostack, nomem, preserves_flags)),
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
    /// Exact generation of the sender's recyclable address-space slot, or
    /// zero when returned by a legacy receive syscall.
    pub sender_generation: u64,
    /// Stable principal assigned by the trusted loader from signed metadata,
    /// or zero for a legacy receive.
    pub sender_principal: u64,
    /// Supervisor-assigned [`domain_roles`] bits; zero for a legacy receive.
    pub sender_roles: u32,
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
            options(preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation: 0,
        sender_principal: 0,
        sender_roles: 0,
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
            options(preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation: 0,
        sender_principal: 0,
        sender_roles: 0,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

/// Receive using the explicit authenticated-envelope ABI.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc_ipc_recv_authenticated(endpoint: u64) -> IpcMessage {
    let status: u64;
    let opcode: u64;
    let arg0: u64;
    let reply: u64;
    let sender: u64;
    let interface: u64;
    let version: u64;
    let memory: u64;
    let connection: u64;
    let sender_generation: u64;
    let sender_principal: u64;
    let sender_roles: u64;
    unsafe {
        asm!(
            "svc #71",
            lateout("x0") status,
            inlateout("x1") endpoint => opcode,
            lateout("x2") arg0,
            lateout("x3") reply,
            lateout("x4") sender,
            lateout("x5") interface,
            lateout("x6") version,
            lateout("x7") memory,
            lateout("x8") connection,
            lateout("x9") sender_generation,
            lateout("x10") sender_principal,
            lateout("x11") sender_roles,
            options(preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation,
        sender_principal,
        sender_roles: sender_roles as u32,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

/// Block and receive using the explicit authenticated-envelope ABI.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
unsafe fn svc_ipc_recv_block_authenticated(endpoint: u64) -> IpcMessage {
    let status: u64;
    let opcode: u64;
    let arg0: u64;
    let reply: u64;
    let sender: u64;
    let interface: u64;
    let version: u64;
    let memory: u64;
    let connection: u64;
    let sender_generation: u64;
    let sender_principal: u64;
    let sender_roles: u64;
    unsafe {
        asm!(
            "svc #72",
            lateout("x0") status,
            inlateout("x1") endpoint => opcode,
            lateout("x2") arg0,
            lateout("x3") reply,
            lateout("x4") sender,
            lateout("x5") interface,
            lateout("x6") version,
            lateout("x7") memory,
            lateout("x8") connection,
            lateout("x9") sender_generation,
            lateout("x10") sender_principal,
            lateout("x11") sender_roles,
            options(preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation,
        sender_principal,
        sender_roles: sender_roles as u32,
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

/// Emit a kernel debug log line carrying a UTF-8 string. `len` bytes are read
/// from `ptr` (a pointer into the caller's address space, up to a kernel
/// capped maximum) and rendered on the serial log.
#[inline(always)]
pub fn el0_log_str(ptr: *const u8, len: usize) {
    unsafe {
        svc3(SyscallNumber::LogStr, ptr as u64, len as u64, 0);
    }
}

/// Submit an async operation that needs no buffer. Returns a completion
/// capability, or [`COMPLETION_SUBMIT_FAILED`] when bounded admission fails.
/// Use [`submit_read`] for [`OpCode::Read`].
#[inline(always)]
pub fn submit(op: OpCode) -> u64 {
    unsafe { svc3(SyscallNumber::CompletionSubmit, op as u64, 0, 0) }
}

/// Submit a timer operation that completes after `timeout_ms` milliseconds.
/// Returns a completion capability that auto-completes when the timer fires,
/// or [`COMPLETION_SUBMIT_FAILED`] when submission fails.
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
/// Returns the capability, or [`COMPLETION_SUBMIT_FAILED`] if the complete
/// four-byte destination is not EL0-writable or submission is backpressured.
/// Unaligned and cross-page destinations are supported.
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
/// Returns `(`[`completion_status::READY`]`, result_code)` when ready,
/// `(`[`completion_status::PENDING_OR_TIMEOUT`]`, 0)` while pending, and
/// `(`[`completion_status::INVALID_CAPABILITY`]`, 0)` for an invalid cap.
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
    unsafe { spawn_thread_with_generation(entry_vaddr, target_lp).0 }
}

/// Spawn a new EL0 thread and return its recyclable numeric id together with
/// the monotonic generation captured at publication time.
///
/// The generation must accompany the id when registering a delayed exit
/// observer, otherwise TID reuse could attach the observer to a replacement.
///
/// # Safety
/// `entry_vaddr` must point to valid executable code in the caller's address
/// space. `target_lp` must be a valid LP id.
#[inline(always)]
pub unsafe fn spawn_thread_with_generation(entry_vaddr: usize, target_lp: u32) -> (u64, u64) {
    unsafe { svc3_x1(SyscallNumber::SpawnThread, entry_vaddr as u64, target_lp as u64, 0) }
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

/// Terminate every thread in the calling address space. Never returns.
///
/// This is the process-level failure primitive used by the Rust panic
/// runtime. Kernel-side resource reclamation remains generation-safe and is
/// completed after all remotely running threads have switched away.
#[inline(always)]
pub fn domain_abort() -> ! {
    unsafe {
        svc3(SyscallNumber::DomainAbort, 0, 0, 0);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Register a completion capability that fires when the EL0 thread `tid` is
/// reaped by the kernel, exposing thread joining through the ordinary
/// completion ABI (`wait`/`poll`/`close`).
///
/// Returns the completion capability, or `u64::MAX` when `tid` is invalid or
/// the capability table is full.
///
/// # Safety
/// `tid` must be a thread id previously returned by
/// [`spawn_thread`](spawn_thread) in the caller's address space.
#[inline(always)]
pub unsafe fn observe_thread_exit(tid: u64) -> u64 {
    unsafe { svc3(SyscallNumber::ObserveThreadExit, tid, 0, 0) }
}

/// Register a generation-bound completion for an EL0 thread's exit.
///
/// If the TID has already been reaped or recycled, the returned completion is
/// completed immediately rather than observing the replacement thread.
#[inline(always)]
pub fn observe_thread_exit_generation(tid: u64, generation: u64) -> u64 {
    unsafe { svc3(SyscallNumber::ObserveThreadExit, tid, generation, 0) }
}

/// Return the calling EL0 thread's numeric kernel thread id.
#[inline(always)]
pub fn get_tid() -> u64 {
    unsafe { svc3(SyscallNumber::GetTid, 0, 0, 0) }
}

/// Return the exact address-space identity and supervisor-assigned policy
/// principal of the calling domain. None of these values are supplied by
/// userspace request bytes.
#[inline(always)]
pub fn get_domain_identity() -> DomainIdentityInfo {
    let (asid, generation, principal, roles) =
        unsafe { svc3_x3(SyscallNumber::GetDomainIdentity, 0, 0, 0) };
    DomainIdentityInfo {
        asid,
        generation,
        principal,
        roles: roles as u32,
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
/// Returns `(`[`completion_status::READY`]`, result_code)` on completion,
/// `(`[`completion_status::PENDING_OR_TIMEOUT`]`, 0)` on timeout, and
/// `(`[`completion_status::INVALID_CAPABILITY`]`, 0)` for an invalid cap.
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

/// Receive a message with kernel-authenticated sender generation, principal,
/// and roles. This explicit ABI leaves the legacy receive register contract
/// unchanged for existing runtimes.
#[inline(always)]
pub fn ipc_recv_authenticated(endpoint: u64) -> IpcMessage {
    unsafe { svc_ipc_recv_authenticated(endpoint) }
}

/// Block until a message is readable and return its kernel-authenticated
/// sender envelope.
#[inline(always)]
pub fn ipc_recv_block_authenticated(endpoint: u64) -> IpcMessage {
    unsafe { svc_ipc_recv_block_authenticated(endpoint) }
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

/// Create a completion capability that resolves when the connection's
/// endpoint closes. `u64::MAX` reports an invalid connection or completion
/// backpressure; capability zero is valid.
#[inline]
pub fn ipc_connection_watch_closed(connection: u64) -> u64 {
    unsafe { svc3(SyscallNumber::IpcConnectionWatchClosed, connection, 0, 0) }
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

/// Return the capacity, in bytes, of a memory-object capability, or zero for
/// an invalid capability. This lets receivers validate length prefixes before
/// touching potentially unbacked pages.
#[inline(always)]
pub fn memory_size(cap: u64) -> usize {
    unsafe { svc3(SyscallNumber::MemorySize, cap, 0, 0) as usize }
}

/// Map a memory object at `base_vaddr`. Returns a memory status code.
#[inline(always)]
pub fn memory_map(cap: u64, base_vaddr: usize, writable: bool) -> MemoryStatusCode {
    unsafe { svc3(SyscallNumber::MemoryMap, cap, base_vaddr as u64, writable as u64) }
}

/// Map a memory object at a kernel-assigned scratch address in the caller's
/// address space. Returns `(MemoryStatusCode, vaddr)`; the vaddr is valid
/// only when the status is `OK`.
/// Map a device MMIO region at a kernel-assigned scratch address in the
/// caller's address space. Returns `(status, vaddr)`; the vaddr is valid
/// only when the status is `OK`.
pub fn device_mmio_map_any(cap: u64, writable: bool) -> (MemoryStatusCode, usize) {
    let (status, vaddr, _) =
        unsafe { svc3_x2(SyscallNumber::DeviceMmioMapAny, cap, writable as u64, 0) };
    (status as MemoryStatusCode, vaddr as usize)
}

pub fn memory_map_any(cap: u64, writable: bool) -> (MemoryStatusCode, usize) {
    let (status, vaddr, _) =
        unsafe { svc3_x2(SyscallNumber::MemoryMapAny, cap, writable as u64, 0) };
    (status as MemoryStatusCode, vaddr as usize)
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
/// `cap`, or 0 on error. Only the object's current owner may query it;
/// IPC-borrowed caps must be mapped through [`dma_map`] for device access.
#[inline(always)]
pub fn memory_get_phys(cap: u64) -> u64 {
    memory_get_phys_page(cap, 0)
}

/// Return the physical address of page `page_index` in memory object `cap`,
/// or 0 if the capability is invalid, is borrowed, or the index is out of
/// bounds. Device drivers should use [`dma_map`] rather than raw addresses.
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

/// Pin `memory` into `domain` and return its base I/O virtual address for a
/// driver-managed coherent-sharing protocol.
///
/// # Safety
/// The caller must ensure that CPU accesses, Rust references, device accesses,
/// cache maintenance, and completion are synchronized for `direction`. The
/// memory capability and its backing pages must remain valid until
/// [`dma_unmap`] succeeds. Prefer [`dma_map_exclusive`] for ordinary buffers.
#[inline(always)]
pub unsafe fn dma_map(domain: u64, memory: u64, direction: DmaDirection) -> u64 {
    unsafe { svc3(SyscallNumber::DmaMap, domain, memory, direction as u64) }
}

/// Transfer an unmapped memory object exclusively to `domain`.
///
/// Unlike [`dma_map`], the kernel rejects this operation if any CPU mapping,
/// lend, or DMA pin exists and prevents new mappings/lends until `dma_unmap`.
#[inline(always)]
pub fn dma_map_exclusive(domain: u64, memory: u64, direction: DmaDirection) -> u64 {
    unsafe { svc3(SyscallNumber::DmaMapExclusive, domain, memory, direction as u64) }
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

/// Start the exact signed ELF held by `elf_cap` under `artifact_name`.
/// This succeeds only for the uniquely delegated node deployment agent and
/// only when the CLS2 signed identity matches the packed name. The kernel
/// consumes `elf_cap` on both success and failure.
#[inline(always)]
pub fn spawn_artifact(elf_cap: u64, elf_size: usize, artifact_name: u64) -> u64 {
    unsafe { svc3(SyscallNumber::SpawnArtifact, elf_cap, elf_size as u64, artifact_name) }
}

/// Start a signed ELF with a signed deployment descriptor. The launched
/// application receives only the capability-grant controller as bootstrap and
/// the descriptor as an immutable profile. The kernel consumes both memory
/// capabilities on success and failure.
#[inline(always)]
pub fn spawn_artifact_scoped(
    elf_cap: u64,
    elf_size: usize,
    artifact_name: u64,
    descriptor_cap: u64,
    descriptor_size: usize,
) -> u64 {
    unsafe {
        svc5(
            SyscallNumber::SpawnArtifactScoped,
            elf_cap,
            elf_size as u64,
            artifact_name,
            descriptor_cap,
            descriptor_size as u64,
        )
    }
}

/// Submit one `COPSPK01` encrypted connector pickup package. The kernel
/// consumes the memory capability on every submitted outcome and returns the
/// connector ASID only after re-verification, HPKE open, and profile transfer.
#[inline(always)]
pub fn spawn_operational_connector(
    package_cap: u64,
    package_size: usize,
    artifact_principal: u64,
) -> u64 {
    unsafe {
        svc3(
            SyscallNumber::SpawnOperationalConnector,
            package_cap,
            package_size as u64,
            artifact_principal,
        )
    }
}

/// Retire the domain created by [`spawn_artifact`]. Returns 1 while thread
/// retirement is in progress, 0 once resources and endpoints are reclaimed,
/// and `u64::MAX` when the caller lacks deployment authority.
#[inline(always)]
pub fn retire_artifact() -> u64 {
    unsafe { svc4(SyscallNumber::RetireArtifact, 0, 0, 0, 0) }
}

/// Retire the deployed domain identified by its signed artifact principal.
/// Returns 1 while retirement is in progress, 0 once it is absent/reclaimed,
/// and `u64::MAX` when the caller lacks deployment authority.
#[inline(always)]
pub fn retire_artifact_named(principal: u64) -> u64 {
    unsafe { svc4(SyscallNumber::RetireArtifact, principal, 0, 0, 0) }
}

/// Retire a deployed domain as part of a whole-node drain. `deadline_ms` is
/// in the kernel monotonic millisecond epoch exposed through the domain
/// lifecycle page. The kernel caps the artifact's signed grace period at this
/// enclosing deadline; the caller cannot use it to extend an earlier request.
#[inline(always)]
pub fn retire_artifact_for_node_shutdown(principal: u64, deadline_ms: u64) -> u64 {
    unsafe {
        svc4(
            SyscallNumber::RetireArtifact,
            principal,
            0,
            artifact_retirement_reason::NODE_SHUTDOWN as u64,
            deadline_ms,
        )
    }
}

/// Immediately abort retirement of the named deployed domain. This is the
/// owner-drop fallback; normal control paths should use cooperative
/// [`retire_artifact_named`] polling instead.
#[inline(always)]
pub fn force_retire_artifact_named(principal: u64) -> u64 {
    unsafe { svc4(SyscallNumber::RetireArtifact, principal, 1, 0, 0) }
}

/// Transfer an operator-signed shutdown envelope to the kernel supervisor.
/// This succeeds only for the uniquely delegated deployment agent and after
/// independent signature, target-node, and policy verification. The kernel
/// consumes `envelope_cap` on every outcome and derives the monotonic deadline
/// from the signed relative duration.
///
/// Returns zero when accepted, one when shutdown is already in progress, and
/// `u64::MAX` when authority or policy validation fails.
#[inline(always)]
pub fn request_node_shutdown(envelope_cap: u64, envelope_size: usize) -> u64 {
    unsafe { svc3(SyscallNumber::RequestNodeShutdown, envelope_cap, envelope_size as u64, 0) }
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
/// `x1` = endpoint cap, `x3` = result-page memory capability. The page need
/// not be mapped, and either an owned or BorrowWrite capability is accepted.
/// Returns the legacy 9-register envelope used by [`ipc_recv`], and the result
/// page contents are updated with a little-endian `u16` count followed
/// immediately by that many little-endian `u64` capability IDs. An invalid,
/// read-only, or undersized result page leaves the message queued and returns
/// [`ipc_status::MEMORY_TRANSFER_FAILED`].
#[cfg(target_arch = "aarch64")]
#[inline(always)]
/// # Safety
///
/// `result_page` must name an owned or BorrowWrite memory-object capability
/// containing enough space for the kernel's packed capability-vector result.
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
            lateout("x2") arg0,
            inlateout("x3") result_page => reply,
            lateout("x4") sender,
            lateout("x5") interface,
            lateout("x6") version,
            lateout("x7") memory,
            lateout("x8") connection,
            // The kernel writes through `result_page`; omitting `nomem` makes
            // this syscall a compiler memory barrier for surrounding accesses.
            options(nostack, preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation: 0,
        sender_principal: 0,
        sender_roles: 0,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
/// # Safety
///
/// `result_page` must name an owned or BorrowWrite memory-object capability
/// containing enough space for the kernel's packed capability-vector result.
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
            "syscall",
            inlateout("rax") SyscallNumber::IpcRecvVec as u64 => status,
            inlateout("rdi") endpoint => opcode,
            lateout("rsi") arg0,
            inlateout("rdx") result_page => reply,
            lateout("r10") sender,
            lateout("r8") interface,
            lateout("r9") version,
            lateout("r11") memory,
            lateout("rcx") connection,
            options(preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation: 0,
        sender_principal: 0,
        sender_roles: 0,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
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
        sender_generation: 0,
        sender_principal: 0,
        sender_roles: 0,
        interface: 0,
        version: 0,
        memory: 0,
        connection: 0,
    }
}

/// Receive a capability vector plus the kernel-authenticated sender envelope.
/// Unlike [`ipc_recv_vec`], this explicit ABI also returns x9--x11.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
/// # Safety
///
/// `result_page` must name an owned or BorrowWrite memory-object capability
/// containing enough space for the kernel's packed capability-vector result.
pub unsafe fn ipc_recv_vec_authenticated(endpoint: u64, result_page: u64) -> IpcMessage {
    let status: u64;
    let opcode: u64;
    let arg0: u64;
    let reply: u64;
    let sender: u64;
    let interface: u64;
    let version: u64;
    let memory: u64;
    let connection: u64;
    let sender_generation: u64;
    let sender_principal: u64;
    let sender_roles: u64;
    unsafe {
        asm!(
            "svc #73",
            lateout("x0") status,
            inlateout("x1") endpoint => opcode,
            lateout("x2") arg0,
            inlateout("x3") result_page => reply,
            lateout("x4") sender,
            lateout("x5") interface,
            lateout("x6") version,
            lateout("x7") memory,
            lateout("x8") connection,
            lateout("x9") sender_generation,
            lateout("x10") sender_principal,
            lateout("x11") sender_roles,
            options(nostack, preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation,
        sender_principal,
        sender_roles: sender_roles as u32,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
/// # Safety
///
/// `result_page` must name an owned or BorrowWrite memory-object capability
/// containing enough space for the kernel's packed capability-vector result.
pub unsafe fn ipc_recv_vec_authenticated(endpoint: u64, result_page: u64) -> IpcMessage {
    let status: u64;
    let opcode: u64;
    let arg0: u64;
    let reply: u64;
    let sender: u64;
    let interface: u64;
    let version: u64;
    let memory: u64;
    let connection: u64;
    let sender_generation: u64;
    let sender_principal: u64;
    let sender_roles: u64;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") SyscallNumber::IpcRecvVecAuthenticated as u64 => status,
            inlateout("rdi") endpoint => opcode,
            lateout("rsi") arg0,
            inlateout("rdx") result_page => reply,
            lateout("r10") sender,
            lateout("r8") interface,
            lateout("r9") version,
            lateout("r11") memory,
            lateout("rcx") connection,
            lateout("r12") sender_generation,
            lateout("r13") sender_principal,
            lateout("r14") sender_roles,
            options(preserves_flags),
        );
    }
    IpcMessage {
        status,
        opcode: opcode as u32,
        arg0,
        reply,
        sender,
        sender_generation,
        sender_principal,
        sender_roles: sender_roles as u32,
        interface,
        version: version as u32,
        memory,
        connection,
    }
}

#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
/// # Safety
///
/// Mirrors the AArch64 authenticated vector-receive ABI.
pub unsafe fn ipc_recv_vec_authenticated(endpoint: u64, result_page: u64) -> IpcMessage {
    unsafe { ipc_recv_vec(endpoint, result_page) }
}

/// Snapshot scheduler statistics. With capability zero, the result contains
/// only threads owned by the caller. A delegated system-observer capability
/// authorizes a machine-wide snapshot. Returns
/// `(memory_object_capability, exact_byte_length)`.
#[inline]
pub fn thread_statistics_snapshot(system_observer: u64) -> (u64, u64) {
    unsafe { svc3_x1(SyscallNumber::ThreadStatistics, system_observer, 0, 0) }
}

/// Return one cryptographically random word from a kernel-provided source.
///
/// This is intentionally fallible: cryptographic callers must fail closed
/// rather than silently substituting a low-entropy source.
#[inline]
pub fn random_u64() -> Option<u64> {
    let (ok, value) = unsafe { svc3_x1(SyscallNumber::RandomU64, 0, 0, 0) };
    (ok == 1).then_some(value)
}
