//! `charlotte-protocol-block` — the block device protocol v1.
//!
//! This crate defines the interface between a block device consumer
//! (filesystem, Raft log store, etc.) and a block device driver. The
//! driver knows nothing about filesystems, partitions, or higher-level
//! storage semantics — it reads and writes fixed-size blocks at linear
//! block addresses.
#![no_std]

pub const INTERFACE: u64 = 0x4b434f4c42; // "BLOCK" packed as little-endian ASCII
pub const VERSION: u32 = 1;
pub const NAME: u64 = 0x306b6c62; // "blk0" little-endian

pub const OP_INFO: u32 = 1;
pub const OP_READ: u32 = 2;
pub const OP_WRITE: u32 = 3;
pub const OP_FLUSH: u32 = 4;
pub const OP_TRIM: u32 = 5;

pub const ERR_OK: i64 = 0;
pub const ERR_IO_ERROR: i64 = 1;
pub const ERR_INVALID_RANGE: i64 = 2;
pub const ERR_UNALIGNED: i64 = 3;
pub const ERR_DEVICE_GONE: i64 = 4;

#[inline]
pub fn pack_lba_count(lba: u64, count: u32) -> u64 {
    (lba << 32) | (count as u64)
}

#[inline]
pub fn unpack_lba_count(arg: u64) -> (u64, u32) {
    let lba = arg >> 32;
    let count = (arg & 0xffff_ffff) as u32;
    (lba, count)
}

#[inline]
pub fn pack_info(block_size: u32, total_blocks: u32) -> i64 {
    ((block_size as u64) | ((total_blocks as u64) << 32)) as i64
}

#[inline]
pub fn unpack_info(reply: i64) -> (u32, u32) {
    let v = reply as u64;
    let block_size = (v & 0xffff_ffff) as u32;
    let total_blocks = (v >> 32) as u32;
    (block_size, total_blocks)
}
