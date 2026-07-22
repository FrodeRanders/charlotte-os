//! # Userspace Service Infrastructure
//!
//! Kernel-side support for loading, bootstrapping, and supervising isolated
//! EL0 service protection domains (architecture doc §16.7, §19.1, Phase 3).
//!
//! The kernel deliberately knows nothing about service *names*: naming and
//! lookup policy live in the userspace name service. This module only
//! provides mechanism:
//!
//! - [`loader`] maps an ELF image and the standard runtime pages into a fresh user address space;
//! - [`bootstrap`] defines the config-page contract through which a spawned domain receives its
//!   initial capability;
//! - [`supervisor`] spawns, observes, and tears down service domains.

pub mod bootstrap;
pub mod loader;
pub mod supervisor;
