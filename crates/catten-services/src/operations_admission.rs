//! Pure leader-side admission for encrypted operational bundles.
//!
//! Keeping signature/context/expiry verification in one testable function
//! prevents the HTTP ingress, follower relay, and Raft command construction
//! from drifting into three subtly different policies.

extern crate alloc;

use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    Expired,
    Invalid,
    TooLarge,
    WrongCluster,
    WrongOperationsKey,
    WrongRecipient,
    WrongReleaseKey,
}

/// Verify both authorities and all operational context, resolve automatic
/// component placement, and return the compact command suitable for Raft.
///
/// This function must be called by the current leader immediately before
/// submission. `now_unix_seconds` must come from the trusted cluster time
/// service; callers fail closed when UTC is unavailable.
pub fn verify_and_encode(
    bundle_bytes: &[u8],
    trust: &charlotte_launch::trust::AdmissionTrust,
    now_unix_seconds: u64,
    automatic_node: u64,
) -> Result<Vec<u8>, AdmissionError> {
    use charlotte_launch::operations_bundle::VerifyOutcome;

    if bundle_bytes.len() > charlotte_launch::operations_bundle::MAX_BUNDLE_LEN {
        return Err(AdmissionError::TooLarge);
    }
    let outcome = charlotte_launch::operations_bundle::verify(
        bundle_bytes,
        &trust.deployment_key,
        &trust.operations_key,
        &trust.recipient_key,
        &trust.cluster_id,
        now_unix_seconds,
    );
    match outcome {
        VerifyOutcome::Valid => {}
        VerifyOutcome::Expired => return Err(AdmissionError::Expired),
        VerifyOutcome::Invalid => return Err(AdmissionError::Invalid),
        VerifyOutcome::WrongCluster => return Err(AdmissionError::WrongCluster),
        VerifyOutcome::WrongOperationalKey => return Err(AdmissionError::WrongOperationsKey),
        VerifyOutcome::WrongRecipient => return Err(AdmissionError::WrongRecipient),
        VerifyOutcome::WrongReleaseKey => return Err(AdmissionError::WrongReleaseKey),
    }
    let bundle =
        charlotte_launch::operations_bundle::decode(bundle_bytes).ok_or(AdmissionError::Invalid)?;
    let release =
        charlotte_launch::release::decode(bundle.release).ok_or(AdmissionError::Invalid)?;
    let nodes = release
        .descriptors()
        .map(|bytes| {
            let descriptor = charlotte_launch::deployment::decode(bytes)?;
            Some(
                if descriptor.node_key == 0 {
                    automatic_node
                } else {
                    descriptor.node_key
                },
            )
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(AdmissionError::Invalid)?;
    crate::name_catalog::encode_release_with_operations(bundle_bytes, &nodes)
        .ok_or(AdmissionError::TooLarge)
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use catten_graft::state_machine::StateMachine;
    use charlotte_launch::{
        deployment::{
            self,
            DescriptorFields,
        },
        operations::{
            self,
            EnvelopeFields,
            PROFILE_KIND_KAFKA,
        },
        operations_bundle::{
            self,
            BindingFields,
            BundleFields,
        },
        release::{
            self,
            ReleaseFields,
        },
        trust::AdmissionTrust,
    };
    use ed25519_compact::{
        KeyPair,
        Signature,
    };
    use rand_core_10::UnwrapErr;

    use super::*;
    use crate::{
        name_catalog::NameCatalog,
        roperations,
    };

    struct Fixture {
        bundle: Vec<u8>,
        trust: AdmissionTrust,
    }

    fn sign_deployment(bytes: &mut [u8], pair: &KeyPair) {
        let signature: Signature = pair.sk.sign(deployment::signature_digest(bytes).unwrap(), None);
        assert!(deployment::set_signature(bytes, signature.as_ref().try_into().unwrap()));
    }

    fn fixture(profile_len: usize, expiry: u64) -> Fixture {
        let release_pair = KeyPair::from_seed([0x31; 32].into());
        let operational_pair = KeyPair::from_seed([0x42; 32].into());
        let release_public: &[u8; 32] = release_pair.pk.as_ref().try_into().unwrap();
        let operational_public: &[u8; 32] = operational_pair.pk.as_ref().try_into().unwrap();
        let descriptor_fields = DescriptorFields {
            sequence: 1,
            node_key: 0,
            artifact_digest: [0x77; 32],
            artifact_name: b"kafka",
            stack_pages_per_thread: 32,
            max_threads: 16,
            shutdown_grace_ms: charlotte_launch::DEFAULT_SHUTDOWN_GRACE_MS,
            object_key: b"releases/kafka.elf",
            grants: &[],
        };
        let mut descriptor = vec![0; deployment::encoded_len(&descriptor_fields).unwrap()];
        deployment::encode_unsigned(&descriptor_fields, release_public, &mut descriptor).unwrap();
        sign_deployment(&mut descriptor, &release_pair);
        let descriptors = [descriptor.as_slice()];
        let release_fields = ReleaseFields {
            sequence: 1,
            release_name: b"orders",
            descriptors: &descriptors,
        };
        let mut release = vec![0; release::encoded_len(&release_fields).unwrap()];
        release::encode_unsigned(&release_fields, release_public, &mut release).unwrap();
        let signature: Signature =
            release_pair.sk.sign(release::signature_digest(&release).unwrap(), None);
        assert!(release::set_signature(&mut release, signature.as_ref().try_into().unwrap()));

        let mut rng = UnwrapErr(getrandom::SysRng);
        let (_, recipient_public) = operations::generate_recipient_keypair(&mut rng);
        let cluster_id = [0x11; 32];
        let envelope_fields = EnvelopeFields {
            sequence: 1,
            expires_unix_seconds: expiry,
            profile_kind: PROFILE_KIND_KAFKA,
            cluster_id,
            release_digest: charlotte_launch::sha256::digest(&release),
            profile_name: b"kafka/orders/transactional",
        };
        let profile = vec![0x5a; profile_len];
        let mut envelope =
            vec![0; operations::encoded_len(&envelope_fields, profile.len()).unwrap()];
        operations::seal_unsigned(
            &envelope_fields,
            &profile,
            &recipient_public,
            operational_public,
            &mut rng,
            &mut envelope,
        )
        .unwrap();
        let signature: Signature =
            operational_pair.sk.sign(operations::signature_digest(&envelope).unwrap(), None);
        assert!(operations::set_signature(&mut envelope, signature.as_ref().try_into().unwrap()));
        let bindings = [BindingFields {
            target_artifact: b"kafka",
            object_key: b"operations/orders-kafka.cops",
            envelope: &envelope,
        }];
        let bundle_fields = BundleFields {
            sequence: 1,
            cluster_id,
            release: &release,
            bindings: &bindings,
        };
        let mut bundle = vec![0; operations_bundle::encoded_len(&bundle_fields).unwrap()];
        operations_bundle::encode_unsigned(
            &bundle_fields,
            operational_public,
            &recipient_public,
            &mut bundle,
        )
        .unwrap();
        let binding_signature: Signature = operational_pair
            .sk
            .sign(operations_bundle::binding_signature_digest(&bundle, 0).unwrap(), None);
        assert!(operations_bundle::set_binding_signature(
            &mut bundle,
            0,
            binding_signature.as_ref().try_into().unwrap()
        ));
        let signature: Signature =
            operational_pair.sk.sign(operations_bundle::signature_digest(&bundle).unwrap(), None);
        assert!(operations_bundle::set_signature(
            &mut bundle,
            signature.as_ref().try_into().unwrap()
        ));
        Fixture {
            bundle,
            trust: AdmissionTrust {
                sequence: 1,
                cluster_id,
                artifact_key: [0x22; 32],
                deployment_key: *release_public,
                operations_key: *operational_public,
                recipient_key: recipient_public,
            },
        }
    }

    #[test]
    fn leader_verification_produces_only_compact_replay_fenced_state() {
        let fixture = fixture(128, 2_000_000_000);
        let command =
            verify_and_encode(&fixture.bundle, &fixture.trust, 1_900_000_000, 0x1234).unwrap();
        assert!(command.len() <= catten_graft::types::MAX_COMMAND_BYTES);
        assert!(!command.windows(32).any(|window| window.iter().all(|byte| *byte == 0x5a)));

        let catalog = NameCatalog::new_with_deployment_key(fixture.trust.deployment_key);
        assert_eq!(catalog.apply_with_result(1, &command), 1i64.to_le_bytes());
        let binding = catalog.operational_binding(b"kafka/orders/transactional").unwrap();
        assert_eq!(binding.target_artifact, b"kafka");
        assert_eq!(binding.object_key, b"operations/orders-kafka.cops");
        assert_eq!(binding.sequence, 1);
    }

    #[test]
    fn leader_fails_closed_on_expiry_and_wrong_authority_context() {
        let fixture = fixture(32, 2_000_000_000);
        assert_eq!(
            verify_and_encode(&fixture.bundle, &fixture.trust, 2_000_000_001, 1),
            Err(AdmissionError::Expired)
        );
        let mut wrong = fixture.trust;
        wrong.operations_key = [0x99; 32];
        assert_eq!(
            verify_and_encode(&fixture.bundle, &wrong, 1_900_000_000, 1),
            Err(AdmissionError::WrongOperationsKey)
        );
        let mut wrong = fixture.trust;
        wrong.cluster_id = [0x88; 32];
        assert_eq!(
            verify_and_encode(&fixture.bundle, &wrong, 1_900_000_000, 1),
            Err(AdmissionError::WrongCluster)
        );
    }

    #[test]
    fn follower_relay_uses_32_bit_framing_above_64_kib() {
        let fixture = fixture(operations::MAX_PROFILE_LEN, 2_000_000_000);
        assert!(fixture.bundle.len() > usize::from(u16::MAX));
        let request = roperations::Request {
            session: 3,
            request_id: 9,
            caller: b"node-a".to_vec(),
            bundle: fixture.bundle,
        };
        let body = roperations::encode_request(&request).unwrap();
        let mut frame = vec![roperations::TAG_REQUEST];
        frame.extend_from_slice(&body);
        assert_eq!(roperations::decode_request(&frame), Some(request));
    }
}
