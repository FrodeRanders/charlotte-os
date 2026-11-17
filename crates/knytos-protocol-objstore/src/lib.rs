//! Persistent object store protocol v1, backed by on-disk format v4.
//!
//! The object store is a userspace service that provides crash-safe,
//! dynamically-sized blob storage on top of a block device. Objects have
//! 64-bit IDs and a fixed identity/ownership header followed by their data extent.
//!
//! ## On-disk format
//!
//! Two checksummed superblock slots occupy LBAs 0 and 1. The valid slot with
//! the newest generation selects the bitmap and fixed 512-entry directory.
//! Directory entries are atomic locators:
//! ```text
//! id: u64 | flags: u32 | generation: u32 |
//! header_lba: u64 | header_blocks: u32 | crc32: u32
//! ```
//!
//! Each allocation uses a fixed 384-byte `CCOBJHD4` header with explicit
//! nonzero owner, known attributes, exact lengths, up to 16 extents, content
//! hash and checksum. Superblocks are `COBJSTR4`; older stores reject before
//! mutation. Replacement is copy-on-write: new data, header, and allocation
//! state are flushed before the directory locator changes.
//! Mount rebuilds allocation state from reachable validated headers, reclaiming
//! abandoned pre-commit allocations. The FNV-1a content hash detects accidental
//! corruption; it is not cryptographic authentication.
#![no_std]

pub mod disk;

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
/// Create an object with `ATTR_SHARED_READ`: any domain authenticated by the
/// kernel may read it. The creating principal remains the owner for mutation.
pub const OP_CREATE_AT_SHARED: u32 = 10;
/// Install an operator scope table. The attached read-only memory object
/// holds the encoded table (see [`SCOPE_HEADER_LEN`]/[`SCOPE_ENTRY_LEN`]);
/// only the kernel and the signed administration service may apply it.
/// The table is persisted and re-applied at mount.
pub const OP_APPLY_SCOPE: u32 = 11;

/// Object attribute: readable by every authenticated domain, including non-owners.
pub const ATTR_SHARED_READ: u32 = 1;

/// Reserved object holding the persisted scope table.
pub const SCOPE_OBJECT_ID: u64 = 0xffff_0000_0000_0002;

/// Scope grant bit: read and size query.
pub const SCOPE_RIGHT_READ: u32 = 1;
/// Scope grant bit: resize, overwrite and delete.
pub const SCOPE_RIGHT_WRITE: u32 = 2;
/// Encoded table header:
/// `magic:u32 | version:u16 | entry_count:u16 | reserved:u64`.
pub const SCOPE_MAGIC: u32 = 0x5043_534f; // "OSCP" LE
pub const SCOPE_VERSION: u16 = 1;
pub const SCOPE_HEADER_LEN: usize = 16;
/// Encoded entry: `principal:u64 | first:u64 | last:u64 | rights:u32 | reserved:u32`.
pub const SCOPE_ENTRY_LEN: usize = 32;
pub const SCOPE_MAX_ENTRIES: usize = 64;
pub const SCOPE_MAX_LEN: usize = SCOPE_HEADER_LEN + SCOPE_MAX_ENTRIES * SCOPE_ENTRY_LEN;

pub const ERR_OK: i64 = 0;
pub const ERR_NOT_FOUND: i64 = 1;
pub const ERR_NO_SPACE: i64 = 2;
pub const ERR_INVALID_ID: i64 = 3;
pub const ERR_IO_ERROR: i64 = 4;
pub const ERR_EXISTS: i64 = 5;
pub const ERR_TOO_LARGE: i64 = 6;
/// The caller is not authorized to mutate this object. Reserved artifact and
/// executable objects are mutated only by the kernel and the deployment
/// administration service.
pub const ERR_DENIED: i64 = 7;

/// Well-known object containing the installed AArch64 `echo` service ELF.
///
/// The high namespace is reserved for executable packages and does not
/// collide with monotonically allocated application objects.
pub const EXECUTABLE_ECHO_ID: u64 = 0xffff_0000_0000_0001;
