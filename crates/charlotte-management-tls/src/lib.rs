//! Native management TLS protocol engine, independent of socket ownership.
//!
//! Provisioning supplies both CA roots and exact enrolled operator leaves.
//! Neither a certificate subject nor CA membership alone grants a role. The
//! embedding service owns its socket, CQ-bound operations and monotonic clock;
//! wire bytes must remain owned until their confirmed send receipt. This crate
//! implements no listener, provisioning trust root or authenticated time source.
#![no_std]

extern crate alloc;

mod crypto;
mod policy;
mod session;

pub use policy::{
    Config,
    Credentials,
    Enrollment,
    Identity,
    Limits,
    Permission,
    PolicyError,
    Role,
    fingerprint,
};
pub use rustls::{
    pki_types::{
        CertificateDer,
        PrivateKeyDer,
        UnixTime,
    },
    time_provider::TimeProvider,
};
pub use session::{
    Error,
    Event,
    Session,
};
