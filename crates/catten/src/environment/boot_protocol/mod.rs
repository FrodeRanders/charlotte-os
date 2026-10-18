//! # Boot Protocol
//!
//! This module contains the kernel's Limine boot-protocol support. We target
//! UEFI firmware, including the subset required by EBBR. On AArch64, the
//! platform requirements follow ARM SystemReady.
//!
//! We keep boot support within these firmware standards. Support for devices
//! that require a different boot path is outside the project's scope.

pub mod limine;
