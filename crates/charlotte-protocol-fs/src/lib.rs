//! Native CharlotteOS filesystem protocol v1.
//!
//! Built on top of the persistent object store. Directories and files are
//! objects; the root directory has a fixed object ID (100). Paths are
//! walked client-side by chaining OP_LOOKUP calls.
#![no_std]

pub const INTERFACE: u64 = 0x0000_5346_5346; // "FFS" LE padded
pub const VERSION: u32 = 1;
pub const NAME: u64 = 0x0000_7366; // "fs" LE

pub const OP_LOOKUP: u32 = 1;
pub const OP_CREATE: u32 = 2;
pub const OP_READ: u32 = 3;
pub const OP_WRITE: u32 = 4;
pub const OP_DELETE: u32 = 5;
pub const OP_LIST: u32 = 6;
pub const OP_FLUSH: u32 = 7;
/// Set the exact byte length used by the next `OP_WRITE`.
pub const OP_SET_SIZE: u32 = 8;

pub const FLAG_DIR: u32 = 1 << 0;
pub const FLAG_FILE: u32 = 0;

pub const ERR_OK: i64 = 0;
pub const ERR_NOT_FOUND: i64 = 1;
pub const ERR_EXISTS: i64 = 2;
pub const ERR_NO_SPACE: i64 = 3;
pub const ERR_IO_ERROR: i64 = 4;
pub const ERR_NOT_DIR: i64 = 5;
pub const ERR_DIR_NOT_EMPTY: i64 = 6;
