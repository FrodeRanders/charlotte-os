//! Offline policy preflight. Validation is not authenticated provisioning.

use curve25519_dalek::{
    edwards::CompressedEdwardsY,
    montgomery::MontgomeryPoint,
};

use super::{
    AdmissionTrust,
    ENCODED_LEN,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductionTrustError {
    MalformedPolicy,
    InvalidRevisionFloor,
    ClusterMismatch,
    RevisionBelowFloor,
    DevelopmentKey,
    SharedRoleKey,
    InvalidSigningKey,
    InvalidRecipientKey,
}

/// Public policy that has passed production key-separation preflight.
///
/// This type proves neither authenticity nor private-key custody. It must not
/// authorize a boot, launch or rotation. A future provisioning boundary must
/// authenticate these exact bytes and obtain the cluster/revision floor from
/// protected state. Production builds remain disabled independently of this API.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductionTrustCandidate(AdmissionTrust);

impl ProductionTrustCandidate {
    pub fn validate(
        trust: AdmissionTrust,
        expected_cluster: &[u8; 32],
        minimum_sequence: u64,
    ) -> Result<Self, ProductionTrustError> {
        use ProductionTrustError::*;
        if !trust.is_valid() {
            return Err(MalformedPolicy);
        }
        if minimum_sequence == 0 {
            return Err(InvalidRevisionFloor);
        }
        if &trust.cluster_id != expected_cluster {
            return Err(ClusterMismatch);
        }
        if trust.sequence < minimum_sequence {
            return Err(RevisionBelowFloor);
        }

        let keys =
            [trust.artifact_key, trust.deployment_key, trust.operations_key, trust.recipient_key];
        let fixtures = [
            crate::CLUSTER_PUBLIC_KEY,
            crate::DEVELOPMENT_OPERATIONS_PUBLIC_KEY,
            crate::DEVELOPMENT_RECIPIENT_PUBLIC_KEY,
        ];
        if keys.iter().any(|key| fixtures.contains(key)) {
            return Err(DevelopmentKey);
        }
        for (index, key) in keys.iter().enumerate() {
            if keys[..index].contains(key) {
                return Err(SharedRoleKey);
            }
        }

        // Validate Ed25519 using the curve implementation already used by
        // HPKE, rather than PublicKey::from_slice (which checks only length).
        // Montgomery identities also expose sign-negated signing keys and
        // Ed25519-to-X25519 conversion of one role's underlying key material.
        let mut identities = [[0; 32]; 4];
        for index in 0..3 {
            identities[index] = signing_identity(&keys[index])?;
        }
        if !canonical_montgomery(&trust.recipient_key)
            // This fixed scalar is a public validation probe, not key material
            // used for an envelope. A zero DH result identifies a
            // non-contributory recipient; no randomness or allocation is needed.
            || MontgomeryPoint(trust.recipient_key).mul_clamped([0; 32]).to_bytes() == [0; 32]
        {
            return Err(InvalidRecipientKey);
        }
        identities[3] = trust.recipient_key;

        // Reject known signing fixtures after conversion too, including the
        // opposite Edwards sign and use as an X25519 recipient.
        for fixture in &fixtures[..2] {
            let point = CompressedEdwardsY(*fixture).decompress().ok_or(InvalidSigningKey)?;
            if identities.contains(&point.to_montgomery().to_bytes()) {
                return Err(DevelopmentKey);
            }
        }
        if identities.contains(&crate::DEVELOPMENT_RECIPIENT_PUBLIC_KEY) {
            return Err(DevelopmentKey);
        }
        for (index, identity) in identities.iter().enumerate() {
            if identities[..index].contains(identity) {
                return Err(SharedRoleKey);
            }
        }
        Ok(Self(trust))
    }

    pub fn decode(
        bytes: &[u8],
        expected_cluster: &[u8; 32],
        minimum_sequence: u64,
    ) -> Result<Self, ProductionTrustError> {
        let trust = AdmissionTrust::decode(bytes).ok_or(ProductionTrustError::MalformedPolicy)?;
        Self::validate(trust, expected_cluster, minimum_sequence)
    }

    pub fn public(&self) -> &AdmissionTrust {
        &self.0
    }

    /// The unchanged CTRUST1 public wire format. There is no signature or
    /// private material in this preflight output.
    pub fn encode(&self) -> [u8; ENCODED_LEN] {
        self.0.encode_fields()
    }
}

pub(super) fn signing_identity(key: &[u8; 32]) -> Result<[u8; 32], ProductionTrustError> {
    let point =
        CompressedEdwardsY(*key).decompress().ok_or(ProductionTrustError::InvalidSigningKey)?;
    if point.compress().to_bytes() != *key || point.is_small_order() || !point.is_torsion_free() {
        return Err(ProductionTrustError::InvalidSigningKey);
    }
    Ok(point.to_montgomery().to_bytes())
}

pub(super) fn reject_development_signing_key(
    key: &[u8; 32],
    identity: &[u8; 32],
) -> Result<(), ProductionTrustError> {
    let fixtures = [
        crate::CLUSTER_PUBLIC_KEY,
        crate::DEVELOPMENT_OPERATIONS_PUBLIC_KEY,
        crate::DEVELOPMENT_RECIPIENT_PUBLIC_KEY,
    ];
    if fixtures.contains(key) || *identity == crate::DEVELOPMENT_RECIPIENT_PUBLIC_KEY {
        return Err(ProductionTrustError::DevelopmentKey);
    }
    for fixture in &fixtures[..2] {
        if signing_identity(fixture)? == *identity {
            return Err(ProductionTrustError::DevelopmentKey);
        }
    }
    Ok(())
}

fn canonical_montgomery(key: &[u8; 32]) -> bool {
    // Canonical little-endian field encoding: strictly below 2^255 - 19.
    // Reject aliases before fixture checks or key-ID derivation.
    let mut modulus = [0xff; 32];
    modulus[0] = 0xed;
    modulus[31] = 0x7f;
    key.iter().rev().cmp(modulus.iter().rev()).is_lt()
}
