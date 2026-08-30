use charlotte_launch::trust::{
    self,
    AdmissionTrust,
};

fn fixture() -> AdmissionTrust {
    AdmissionTrust {
        sequence: 7,
        cluster_id: [0x11; 32],
        artifact_key: [0x22; 32],
        deployment_key: [0x33; 32],
        operations_key: [0x44; 32],
        recipient_key: [0x55; 32],
    }
}

#[test]
fn role_aware_trust_round_trips() {
    let trust = fixture();
    let bytes = trust.encode().unwrap();
    assert_eq!(AdmissionTrust::decode(&bytes), Some(trust));
    assert_eq!(trust::cluster_id(b"orders"), trust::cluster_id(b"orders"));
    assert_ne!(trust::cluster_id(b"orders"), trust::cluster_id(b"payments"));
}

#[test]
fn trust_rejects_zero_and_cross_domain_key_reuse() {
    let mut trust = fixture();
    trust.sequence = 0;
    assert!(trust.encode().is_none());

    let mut trust = fixture();
    trust.operations_key = trust.deployment_key;
    assert!(trust.encode().is_none());

    let mut trust = fixture();
    trust.recipient_key = trust.artifact_key;
    assert!(trust.encode().is_none());

    let mut bytes = fixture().encode().unwrap();
    bytes[12] = 1;
    assert!(AdmissionTrust::decode(&bytes).is_none());
}
