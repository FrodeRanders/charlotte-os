//! Complete, bounded outbound OTLP/HTTP launch policy. No ambient destination.

use alloc::{
    string::{
        String,
        ToString,
    },
    vec::Vec,
};

pub const KEY: u64 = crate::manifest_key(b"otlp");
pub const MAGIC: &[u8; 8] = b"COTLP001";
pub const HEADER: usize = 40;
pub const MAX_PATH: usize = 256;
pub const DEFAULT_INTERVAL_MS: u64 = 30_000;
pub const DEFAULT_TIMEOUT_MS: u64 = 3_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub address: [u8; 4],
    pub port: u16,
    pub path: String,
    pub interval_ms: u64,
    pub timeout_ms: u64,
}

impl Profile {
    /// Initial transport is explicit cleartext HTTP to an IPv4 collector.
    /// HTTPS is rejected rather than silently downgraded or left unverified.
    pub fn from_endpoint(endpoint: &str, interval_ms: u64, timeout_ms: u64) -> Option<Self> {
        let rest = endpoint.strip_prefix("http://")?;
        let (authority, path) =
            rest.split_once('/').map_or((rest, "/v1/metrics".to_string()), |(host, path)| {
                (host, alloc::format!("/{path}"))
            });
        let (host, port) =
            authority.split_once(':').map_or(Some((authority, 4318)), |(host, port)| {
                decimal(port).and_then(|port| u16::try_from(port).ok()).map(|port| (host, port))
            })?;
        let mut address = [0; 4];
        let mut parts = host.split('.');
        for part in &mut address {
            *part = u8::try_from(decimal(parts.next()?)?).ok()?;
        }
        if parts.next().is_some() {
            return None;
        }
        let profile = Self {
            address,
            port,
            path,
            interval_ms,
            timeout_ms,
        };
        profile.valid().then_some(profile)
    }

    pub fn valid(&self) -> bool {
        self.port != 0
            && self.address[0] != 0
            && self.address[0] < 224
            && (1_000..=86_400_000).contains(&self.interval_ms)
            && (100..=60_000).contains(&self.timeout_ms)
            && self.timeout_ms <= self.interval_ms
            && self.path.starts_with('/')
            && self.path.len() <= MAX_PATH
            && self.path.bytes().all(|b| (0x21..=0x7e).contains(&b) && b != b'#' && b != b'?')
    }

    pub fn encode(&self) -> Option<Vec<u8>> {
        if !self.valid() {
            return None;
        }
        let mut bytes = alloc::vec![0; HEADER + self.path.len()];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..10].copy_from_slice(&1u16.to_le_bytes());
        bytes[10..12].copy_from_slice(&(HEADER as u16).to_le_bytes());
        let len = bytes.len() as u32;
        bytes[12..16].copy_from_slice(&len.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.address);
        bytes[20..22].copy_from_slice(&self.port.to_le_bytes());
        bytes[22..24].copy_from_slice(&(self.path.len() as u16).to_le_bytes());
        bytes[24..32].copy_from_slice(&self.interval_ms.to_le_bytes());
        bytes[32..40].copy_from_slice(&self.timeout_ms.to_le_bytes());
        bytes[HEADER..].copy_from_slice(self.path.as_bytes());
        Some(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER
            || bytes.len() > HEADER + MAX_PATH
            || &bytes[..8] != MAGIC
            || u16::from_le_bytes(bytes[8..10].try_into().ok()?) != 1
            || usize::from(u16::from_le_bytes(bytes[10..12].try_into().ok()?)) != HEADER
            || u32::from_le_bytes(bytes[12..16].try_into().ok()?) as usize != bytes.len()
            || usize::from(u16::from_le_bytes(bytes[22..24].try_into().ok()?))
                != bytes.len() - HEADER
        {
            return None;
        }
        let profile = Self {
            address: bytes[16..20].try_into().ok()?,
            port: u16::from_le_bytes(bytes[20..22].try_into().ok()?),
            interval_ms: u64::from_le_bytes(bytes[24..32].try_into().ok()?),
            timeout_ms: u64::from_le_bytes(bytes[32..40].try_into().ok()?),
            path: core::str::from_utf8(&bytes[HEADER..]).ok()?.to_string(),
        };
        profile.valid().then_some(profile)
    }
}

pub fn decimal(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_endpoint_and_complete_policy_roundtrip() {
        let p = Profile::from_endpoint("http://10.0.2.2:4318/v1/metrics", 30_000, 3_000).unwrap();
        let bytes = p.encode().unwrap();
        assert_eq!(Profile::decode(&bytes), Some(p));
        for n in 0..bytes.len() {
            assert!(Profile::decode(&bytes[..n]).is_none());
        }
        let mut extra = bytes;
        extra.push(0);
        assert!(Profile::decode(&extra).is_none());
    }
    #[test]
    fn invalid_destination_or_budget_never_falls_back() {
        for url in [
            "",
            "https://10.0.2.2:4318/v1/metrics",
            "http://collector:4318/v1/metrics",
            "http://a@10.0.2.2:4318/x",
            "http://10.0.2.2:0/x",
            "http://224.0.0.1/x",
            "http://10.0.2.2/x\r\nX: y",
            "http://10.0.2.2/x?secret=yes",
        ] {
            assert!(Profile::from_endpoint(url, 30_000, 3_000).is_none(), "{url}");
        }
        assert!(Profile::from_endpoint("http://10.0.2.2", 999, 100).is_none());
        assert!(Profile::from_endpoint("http://10.0.2.2", 1000, 1001).is_none());
    }
}
