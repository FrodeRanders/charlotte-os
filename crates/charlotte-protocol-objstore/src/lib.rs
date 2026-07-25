//! Persistent object store protocol v1.
//!
//! The object store is a userspace service that provides crash-safe,
//! dynamically-sized blob storage on top of a block device. Objects are
//! identified by 64-bit monotonically-increasing IDs and stored as linked
//! lists of extents on disk.
//!
//! ## On-disk format
//!
//! Superblock (LBA 0, one block):
//! ```
//! [0..8):   magic: u64 = 0x525453424A4F43 ("COBJSTR" LE)
//! [8..12):  version: u32 = 1
//! [12..16): generation: u32
//! [16..20): block_size: u32
//! [20..24): total_blocks: u32
//! [24..28): object_count: u32
//! [28..32): next_object_id: u32  (monotonically increasing)
//! [32..40): object_dir_lba: u64  (LBA of the object directory)
//! [40..48): free_bitmap_lba: u64
//! [48..56): free_bitmap_blocks: u64
//! ```
//!
//! Object directory (linked list of directory blocks):
//! Each entry is 32 bytes:
//! ```
//! [0..8):   id: u64
//! [8..12):  flags: u32 (bit 0 = allocated, bit 1 = deleted)
//! [12..16): size_bytes: u32
//! [16..24): first_extent_lba: u64
//! [24..32): reserved
//! ```
//! A directory block holds N entries, then a next_dir_block_lba (or 0).
//! Slots with id==0 are free.
//!
//! Extent (each extent is one or more blocks, linked list):
//! First block of extent:
//! ```
//! [0..8):   next_extent_lba: u64 (0 = end)
//! [8..12):  extent_blocks: u32
//! [12..block_size): data
//! ```
//!
//! Crash safety: extents are written first, then the directory entry
//! pointer is updated atomically. On mount, extents not reachable from
//! any directory entry are reclaimed.
#![no_std]

pub const INTERFACE: u64 = 0x525453424a4f; // "OBJSTR" packed LE (6 chars)
pub const VERSION: u32 = 1;
pub const NAME: u64 = 0x6a626f; // "obj" LE

pub const OP_CREATE: u32 = 1;
pub const OP_DELETE: u32 = 2;
pub const OP_WRITE: u32 = 3;
pub const OP_READ: u32 = 4;
pub const OP_RESIZE: u32 = 5;
pub const OP_FLUSH: u32 = 6;
pub const OP_INFO: u32 = 7;

pub const ERR_OK: i64 = 0;
pub const ERR_NOT_FOUND: i64 = 1;
pub const ERR_NO_SPACE: i64 = 2;
pub const ERR_INVALID_ID: i64 = 3;
pub const ERR_IO_ERROR: i64 = 4;
pub const ERR_EXISTS: i64 = 5;
