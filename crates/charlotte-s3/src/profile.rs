//! Service-local snapshot of the immutable S3 launch profile.
use alloc::{
    string::String,
    vec::Vec,
};
use core::fmt::Write;

use charlotte_protocol_s3::Profile;
use zeroize::Zeroizing;

/// One erasing allocation retains the complete validated profile. The original
/// launch memory stays borrowed by the caller and has its own release policy.
///
/// ```compile_fail
/// fn requires_clone<T: Clone>() {}
/// requires_clone::<charlotte_s3::profile::OwnedProfile>();
/// ```
/// ```compile_fail
/// fn requires_debug<T: core::fmt::Debug>() {}
/// requires_debug::<charlotte_s3::profile::OwnedProfile>();
/// ```
pub struct OwnedProfile {
    bytes: Zeroizing<Vec<u8>>,
}

/// Borrowed fields share their profile owner's lifetime. Formatting the whole
/// view is deliberately unavailable because it includes credentials.
pub struct ProfileView<'a> {
    pub ip: [u8; 4],
    pub host: &'a str,
    pub port: u16,
    pub tls: bool,
    pub ca_der: &'a [u8],
    pub region: &'a str,
    pub bucket: &'a str,
    pub prefix: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a [u8],
    pub namespace: Option<&'a str>,
    pub rights: u64,
}

impl OwnedProfile {
    /// Reject malformed records before allocating. Reserve once and install
    /// the owner before copying any profile bytes out of the borrowed input.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        decode_view(bytes)?;
        let mut snapshot = Zeroizing::new(Vec::with_capacity(bytes.len()));
        snapshot.extend_from_slice(bytes);
        Some(Self {
            bytes: snapshot,
        })
    }

    /// Reuse the current wire decoder to lend fields without a second set of
    /// heap copies or a self-referential representation. The frame is private
    /// and immutable after validation; this recheck allocates nothing.
    pub fn view(&self) -> ProfileView<'_> {
        decode_view(&self.bytes).expect("owned S3 profile remains validated")
    }

    pub fn has(&self, right: u64) -> bool {
        self.view().rights & right != 0
    }
}

impl ProfileView<'_> {
    /// Retain the HTTP authority under an erasing owner through signing and
    /// serialization. Six reserved bytes cover ':' and any u16 decimal port.
    pub fn authority(&self) -> Option<Zeroizing<String>> {
        let include_port = !((self.tls && self.port == 443) || (!self.tls && self.port == 80));
        let len = self.host.len().checked_add(
            if include_port {
                6
            } else {
                0
            },
        )?;
        let mut authority = Zeroizing::new(String::with_capacity(len));
        authority.push_str(self.host);
        if include_port {
            write!(authority, ":{}", self.port).ok()?;
        }
        Some(authority)
    }
}

fn decode_view(bytes: &[u8]) -> Option<ProfileView<'_>> {
    let profile = Profile::decode(bytes)?;
    Some(ProfileView {
        ip: profile.endpoint_ipv4,
        host: core::str::from_utf8(profile.host).ok()?,
        port: profile.port,
        tls: profile.tls,
        ca_der: profile.ca_certificate_der,
        region: core::str::from_utf8(profile.region).ok()?,
        bucket: core::str::from_utf8(profile.bucket).ok()?,
        prefix: core::str::from_utf8(profile.prefix).ok()?,
        access_key: core::str::from_utf8(profile.access_key).ok()?,
        secret_key: profile.secret_key,
        namespace: if profile.namespace.is_empty() {
            None
        } else {
            Some(core::str::from_utf8(profile.namespace).ok()?)
        },
        rights: profile.rights,
    })
}
