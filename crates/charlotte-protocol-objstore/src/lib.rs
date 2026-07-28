//! Persistent object store protocol v1, backed by on-disk format v2.
//!
//! The object store is a userspace service that provides crash-safe,
//! dynamically-sized blob storage on top of a block device. Objects have
//! 64-bit IDs and an extensible metadata header followed by their data extent.
//!
//! ## On-disk format
//!
//! Two checksummed superblock slots occupy LBAs 0 and 1. The valid slot with
//! the newest generation selects the bitmap and fixed 512-entry directory.
//! Directory entries are atomic locators:
//! ```
//! id: u64 | flags: u32 | generation: u32 |
//! header_lba: u64 | header_blocks: u32 | crc32: u32
//! ```
//!
//! Each object allocation begins with a versioned header. `header_len` and
//! `header_blocks` reserve room for compatible metadata growth:
//! ```
//! magic/version/header_len/flags
//! id/generation
//! data_len/allocated_len/data_offset
//! extent_count/hash_algorithm/header_blocks
//! data_lba/data_blocks
//! mandatory content hash
//! header crc32
//! ```
//!
//! The current implementation uses one contiguous data extent, but the header
//! is explicitly extensible. Replacement is copy-on-write: new data, header,
//! and allocation state are flushed before the directory locator changes.
//! Mount rebuilds allocation state from reachable validated headers, reclaiming
//! abandoned pre-commit allocations. The FNV-1a content hash detects accidental
//! corruption; it is not cryptographic authentication.
#![no_std]

pub const INTERFACE: u64 = 0x525453424a4f; // "OBJSTR" packed LE (6 chars)
pub const VERSION: u32 = 1;
pub const NAME: u64 = 0x6a626f; // "obj" LE
/// One-shot self-test completion publication ("objdone").
///
/// Registration is the synchronization event; the client's status page is
/// retained only for diagnostics and result values.
pub const TEST_DONE_NAME: u64 = u64::from_le_bytes(*b"objdone\0");

pub const OP_CREATE: u32 = 1;
pub const OP_DELETE: u32 = 2;
pub const OP_WRITE: u32 = 3;
pub const OP_READ: u32 = 4;
pub const OP_RESIZE: u32 = 5;
pub const OP_FLUSH: u32 = 6;
pub const OP_INFO: u32 = 7;
/// Create an object with the caller-supplied stable ID in `arg0`.
///
/// Returns [`ERR_EXISTS`] when the object is already present.
pub const OP_CREATE_AT: u32 = 8;

/// Set the exact byte length used by the next whole-object write.
///
/// `arg0` is the object ID and the attached read-only memory object contains
/// an eight-byte little-endian length. A separate operation is used because
/// object IDs occupy the complete scalar argument.
pub const OP_SET_SIZE: u32 = 9;

pub const ERR_OK: i64 = 0;
pub const ERR_NOT_FOUND: i64 = 1;
pub const ERR_NO_SPACE: i64 = 2;
pub const ERR_INVALID_ID: i64 = 3;
pub const ERR_IO_ERROR: i64 = 4;
pub const ERR_EXISTS: i64 = 5;
pub const ERR_TOO_LARGE: i64 = 6;

/// Well-known object containing the installed AArch64 `echo` service ELF.
///
/// The high namespace is reserved for executable packages and does not
/// collide with monotonically allocated application objects.
pub const EXECUTABLE_ECHO_ID: u64 = 0xffff_0000_0000_0001;
