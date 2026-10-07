//! Signed bootstrap public policy and explicit revision-lineage verification.
//!
//! Verification authenticates bytes relative to a supplied bootstrap key and
//! expected state. Neither key validation nor this protocol establishes a
//! protected platform root, persistent rollback protection or private custody.

use ed25519_compact::{
    PublicKey,
    Signature,
};

use super::{
    ProductionTrustCandidate,
    ProductionTrustError,
    production,
};
use crate::sha256;

pub const MAGIC: &[u8; 8] = b"CBTRUST1";
pub const VERSION: u16 = 1;
pub const HEADER_LEN: usize = 80;
pub const SIGNATURE_OFFSET: usize = HEADER_LEN + super::ENCODED_LEN;
pub const SIGNATURE_LEN: usize = 64;
pub const ENCODED_LEN: usize = SIGNATURE_OFFSET + SIGNATURE_LEN;
const KEY_ID_OFFSET: usize = 16;
const PREDECESSOR_OFFSET: usize = 48;
const SIGNING_CONTEXT: &[u8] = b"CharlotteOS bootstrap trust policy v1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    MalformedRecord,
    InvalidPolicy(ProductionTrustError),
    InvalidBootstrapKey(ProductionTrustError),
    SharedBootstrapRole,
    WrongBootstrapKey,
    InvalidSignature,
    InvalidExpectation,
    RevisionRollback,
    RevisionConflict,
    RevisionGap,
    WrongPredecessor,
}

/// A canonical, non-fixture signing key. The caller must independently obtain
/// and protect the trusted anchor; an untrusted record never selects it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootstrapKey {
    public: [u8; 32],
    identity: [u8; 32],
}

impl BootstrapKey {
    pub fn new(public: [u8; 32]) -> Result<Self, PolicyError> {
        let identity =
            production::signing_identity(&public).map_err(PolicyError::InvalidBootstrapKey)?;
        production::reject_development_signing_key(&public, &identity)
            .map_err(PolicyError::InvalidBootstrapKey)?;
        Ok(Self {
            public,
            identity,
        })
    }

    fn qualify(&self, policy: &ProductionTrustCandidate) -> Result<(), PolicyError> {
        let trust = policy.public();
        let keys =
            [trust.artifact_key, trust.deployment_key, trust.operations_key, trust.recipient_key];
        if keys.contains(&self.public) || trust.recipient_key == self.identity {
            return Err(PolicyError::SharedBootstrapRole);
        }
        for key in &keys[..3] {
            if production::signing_identity(key).map_err(PolicyError::InvalidPolicy)?
                == self.identity
            {
                return Err(PolicyError::SharedBootstrapRole);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Expectation {
    Enrollment {
        minimum_sequence: u64,
    },
    Installed {
        sequence: u64,
        digest: [u8; 32],
    },
}

/// Caller-supplied acceptance state, not state discovered from the signed
/// record. Production callers must retain it in rollback-resistant storage and
/// must never fall back to enrollment after a failed installed-state check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyExpectation(Expectation);

impl PolicyExpectation {
    pub fn enrollment(minimum_sequence: u64) -> Result<Self, PolicyError> {
        if minimum_sequence == 0 {
            return Err(PolicyError::InvalidExpectation);
        }
        Ok(Self(Expectation::Enrollment {
            minimum_sequence,
        }))
    }

    pub fn installed(sequence: u64, digest: [u8; 32]) -> Result<Self, PolicyError> {
        if sequence == 0 || digest == [0; 32] {
            return Err(PolicyError::InvalidExpectation);
        }
        Ok(Self(Expectation::Installed {
            sequence,
            digest,
        }))
    }

    /// Choose the predecessor for signing an initial policy or its direct
    /// successor. Re-signing an installed policy is not a rotation.
    pub fn signing_predecessor(&self, next_sequence: u64) -> Result<[u8; 32], PolicyError> {
        match self.0 {
            Expectation::Enrollment {
                minimum_sequence,
            } => {
                if next_sequence < minimum_sequence {
                    return Err(PolicyError::RevisionRollback);
                }
                Ok([0; 32])
            }
            Expectation::Installed {
                sequence,
                digest,
            } => {
                if sequence.checked_add(1) != Some(next_sequence) {
                    return Err(PolicyError::RevisionGap);
                }
                Ok(digest)
            }
        }
    }

    fn accept(
        &self,
        sequence: u64,
        digest: [u8; 32],
        predecessor: [u8; 32],
    ) -> Result<(), PolicyError> {
        match self.0 {
            Expectation::Enrollment {
                minimum_sequence,
            } => {
                if sequence < minimum_sequence {
                    return Err(PolicyError::RevisionRollback);
                }
                if predecessor != [0; 32] {
                    return Err(PolicyError::WrongPredecessor);
                }
            }
            Expectation::Installed {
                sequence: installed,
                digest: accepted,
            } => {
                if sequence < installed {
                    return Err(PolicyError::RevisionRollback);
                }
                if sequence == installed {
                    if digest != accepted {
                        return Err(PolicyError::RevisionConflict);
                    }
                } else {
                    if installed.checked_add(1) != Some(sequence) {
                        return Err(PolicyError::RevisionGap);
                    }
                    if predecessor != accepted {
                        return Err(PolicyError::WrongPredecessor);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Authenticated relative to the caller's key, cluster and acceptance state.
/// This receipt does not authorize boot or install itself; a future protected
/// boundary must commit it together with rollback-resistant state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedPolicy {
    policy: ProductionTrustCandidate,
    digest: [u8; 32],
}

impl VerifiedPolicy {
    pub fn policy(&self) -> &ProductionTrustCandidate {
        &self.policy
    }

    /// Domain-separated digest of all signed fields, excluding the signature.
    /// Record this with the sequence only after authorized installation.
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }

    pub fn installed_expectation(&self) -> Result<PolicyExpectation, PolicyError> {
        PolicyExpectation::installed(self.policy.public().sequence, self.digest)
    }
}

pub fn encode_unsigned(
    policy: &ProductionTrustCandidate,
    bootstrap: &BootstrapKey,
    predecessor: [u8; 32],
) -> Result<[u8; ENCODED_LEN], PolicyError> {
    bootstrap.qualify(policy)?;
    let mut bytes = [0; ENCODED_LEN];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&(HEADER_LEN as u16).to_le_bytes());
    bytes[12..16].copy_from_slice(&(ENCODED_LEN as u32).to_le_bytes());
    bytes[KEY_ID_OFFSET..PREDECESSOR_OFFSET].copy_from_slice(&sha256::digest(&bootstrap.public));
    bytes[PREDECESSOR_OFFSET..HEADER_LEN].copy_from_slice(&predecessor);
    bytes[HEADER_LEN..SIGNATURE_OFFSET].copy_from_slice(&policy.encode());
    Ok(bytes)
}

fn decode_policy(
    bytes: &[u8],
    expected_cluster: &[u8; 32],
) -> Result<ProductionTrustCandidate, PolicyError> {
    if bytes.len() != ENCODED_LEN
        || bytes.get(..8) != Some(MAGIC.as_slice())
        || bytes.get(8..10) != Some(VERSION.to_le_bytes().as_slice())
        || bytes.get(10..12) != Some((HEADER_LEN as u16).to_le_bytes().as_slice())
        || bytes.get(12..16) != Some((ENCODED_LEN as u32).to_le_bytes().as_slice())
    {
        return Err(PolicyError::MalformedRecord);
    }
    ProductionTrustCandidate::decode(&bytes[HEADER_LEN..SIGNATURE_OFFSET], expected_cluster, 1)
        .map_err(PolicyError::InvalidPolicy)
}

fn digest_fields(bytes: &[u8]) -> [u8; 32] {
    let mut hash = sha256::Sha256::new();
    hash.update(SIGNING_CONTEXT);
    hash.update(&bytes[..SIGNATURE_OFFSET]);
    hash.finalize()
}

/// Prepare a signing digest; this does not authenticate an unsigned record.
pub fn signature_digest(bytes: &[u8]) -> Result<[u8; 32], PolicyError> {
    let trust = super::AdmissionTrust::decode(
        bytes.get(HEADER_LEN..SIGNATURE_OFFSET).ok_or(PolicyError::MalformedRecord)?,
    )
    .ok_or(PolicyError::MalformedRecord)?;
    decode_policy(bytes, &trust.cluster_id)?;
    Ok(digest_fields(bytes))
}

pub fn set_signature(bytes: &mut [u8], signature: &[u8; SIGNATURE_LEN]) -> Result<(), PolicyError> {
    signature_digest(bytes)?;
    bytes[SIGNATURE_OFFSET..].copy_from_slice(signature);
    Ok(())
}

/// Verify before use. The caller must pin `bootstrap`, cluster and expectation
/// independently of these bytes; there is no self-selected key or default floor.
pub fn verify(
    bytes: &[u8],
    bootstrap: &BootstrapKey,
    expected_cluster: &[u8; 32],
    expectation: &PolicyExpectation,
) -> Result<VerifiedPolicy, PolicyError> {
    let policy = decode_policy(bytes, expected_cluster)?;
    bootstrap.qualify(&policy)?;
    if bytes[KEY_ID_OFFSET..PREDECESSOR_OFFSET] != sha256::digest(&bootstrap.public) {
        return Err(PolicyError::WrongBootstrapKey);
    }
    let digest = digest_fields(bytes);
    let signature = Signature::from_slice(&bytes[SIGNATURE_OFFSET..])
        .map_err(|_| PolicyError::InvalidSignature)?;
    PublicKey::new(bootstrap.public)
        .verify(digest, &signature)
        .map_err(|_| PolicyError::InvalidSignature)?;
    let predecessor = bytes[PREDECESSOR_OFFSET..HEADER_LEN]
        .try_into()
        .map_err(|_| PolicyError::MalformedRecord)?;
    expectation.accept(policy.public().sequence, digest, predecessor)?;
    Ok(VerifiedPolicy {
        policy,
        digest,
    })
}
