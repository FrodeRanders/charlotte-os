//! Role-separated public trust configuration for deployment admission.
//!
//! The configuration is launch-owned policy, not application input and not a
//! container for private key material.  It deliberately identifies each key
//! by the decision it authorizes even when a development installation uses
//! the same Ed25519 key for more than one role.

pub const MAGIC: &[u8; 8] = b"CTRUST1\0";
pub const VERSION: u16 = 1;
pub const ENCODED_LEN: usize = 184;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionTrust {
    /// Monotonic trust-policy revision. Rotation must advance this value.
    pub sequence: u64,
    /// Stable identity of the cluster receiving operational profiles.
    pub cluster_id: [u8; 32],
    /// Ed25519 authority for executable ELF notes.
    pub artifact_key: [u8; 32],
    /// Ed25519 authority for `CDEPLOY1` and `CRELEASE` decisions.
    pub deployment_key: [u8; 32],
    /// Independent Ed25519 authority for `COPSENC1` and `COPSBND2`.
    pub operations_key: [u8; 32],
    /// X25519 public key naming the privileged cluster decryptor.
    pub recipient_key: [u8; 32],
}

impl AdmissionTrust {
    pub fn is_valid(&self) -> bool {
        self.sequence != 0
            && nonzero(&self.cluster_id)
            && nonzero(&self.artifact_key)
            && nonzero(&self.deployment_key)
            && nonzero(&self.operations_key)
            && nonzero(&self.recipient_key)
            // Ed25519 signing and X25519 recipient material are separate
            // compromise and rotation domains, even in development.
            && self.operations_key != self.artifact_key
            && self.operations_key != self.deployment_key
            && self.recipient_key != self.artifact_key
            && self.recipient_key != self.deployment_key
            && self.recipient_key != self.operations_key
    }

    pub fn encode(&self) -> Option<[u8; ENCODED_LEN]> {
        if !self.is_valid() {
            return None;
        }
        let mut bytes = [0u8; ENCODED_LEN];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
        bytes[10..12].copy_from_slice(&(ENCODED_LEN as u16).to_le_bytes());
        bytes[16..24].copy_from_slice(&self.sequence.to_le_bytes());
        bytes[24..56].copy_from_slice(&self.cluster_id);
        bytes[56..88].copy_from_slice(&self.artifact_key);
        bytes[88..120].copy_from_slice(&self.deployment_key);
        bytes[120..152].copy_from_slice(&self.operations_key);
        bytes[152..184].copy_from_slice(&self.recipient_key);
        Some(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != ENCODED_LEN
            || bytes.get(..8)? != MAGIC
            || u16::from_le_bytes(bytes.get(8..10)?.try_into().ok()?) != VERSION
            || usize::from(u16::from_le_bytes(bytes.get(10..12)?.try_into().ok()?)) != ENCODED_LEN
            || bytes.get(12..16)?.iter().any(|byte| *byte != 0)
        {
            return None;
        }
        let trust = Self {
            sequence: u64::from_le_bytes(bytes.get(16..24)?.try_into().ok()?),
            cluster_id: bytes.get(24..56)?.try_into().ok()?,
            artifact_key: bytes.get(56..88)?.try_into().ok()?,
            deployment_key: bytes.get(88..120)?.try_into().ok()?,
            operations_key: bytes.get(120..152)?.try_into().ok()?,
            recipient_key: bytes.get(152..184)?.try_into().ok()?,
        };
        trust.is_valid().then_some(trust)
    }
}

fn nonzero(value: &[u8; 32]) -> bool {
    value.iter().any(|byte| *byte != 0)
}

/// Derive a stable, domain-separated cluster identity from the cluster
/// mnemonic. The mnemonic is not a secret; this digest prevents accidentally
/// admitting an envelope sealed for another Charlotte cluster.
pub fn cluster_id(mnemonic: &[u8]) -> Option<[u8; 32]> {
    if mnemonic.is_empty() {
        return None;
    }
    let mut hasher = crate::sha256::Sha256::new();
    hasher.update(b"CharlotteOS cluster identity v1\0");
    hasher.update(mnemonic);
    Some(hasher.finalize())
}
