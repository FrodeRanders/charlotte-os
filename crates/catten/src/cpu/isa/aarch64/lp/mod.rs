//! # Logical Processor Control Interface for AArch64
pub mod ops;
pub mod thread_context;

pub type LpId = u32;
pub type CoreId = u32;

pub type EicId = u32;
pub type EicPinNum = u32;
pub type InterruptVectorNum = u32;
