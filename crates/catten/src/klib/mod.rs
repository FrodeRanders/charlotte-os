//! # Common Utilities
//!
//! Utilities used throughout the kernel, including collections, time, statistics
//! and observer registration. Prefer `core` or an existing `no_std` library when
//! it already provides what we need; keep kernel-specific behavior here.
pub mod bitwise;
pub(crate) mod charged_allocator;
pub mod collections;
pub mod constants;
pub mod integer;
pub mod io;
pub mod observer;
pub mod size;
pub mod spin;
pub mod statistics;
pub mod sync_cell;
pub mod time;
