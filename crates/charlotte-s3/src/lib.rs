//! Transport-independent S3 data-plane support for CharlotteOS.
//!
//! This crate deliberately performs no I/O and owns no capabilities. It
//! canonicalizes S3 paths and queries, signs requests with AWS Signature
//! Version 4, and parses the bounded part of HTTP responses. The userspace S3
//! service supplies the transport adapter (currently TCP, with verified TLS
//! still pending) and exposes capability-oriented object streams to
//! applications.
#![no_std]

extern crate alloc;

pub mod http;
pub mod sigv4;
