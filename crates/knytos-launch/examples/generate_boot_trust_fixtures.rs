//! Regenerate the kernel's public boot-trust test records with the current
//! cluster identity and signing contexts. All seeds below are public test data.

use std::{
    fs,
    path::Path,
};

use knytos_launch::{
    operations,
    trust::{
        self,
        AdmissionTrust,
        ProductionTrustCandidate,
        signed_policy::{
            self,
            BootstrapKey,
        },
    },
};
use ed25519_compact::{
    KeyPair,
    Seed,
};

fn pair(seed: u8) -> KeyPair {
    KeyPair::from_seed(Seed::new([seed; 32]))
}

fn record(
    sequence: u64,
    operations_seed: u8,
    predecessor: [u8; 32],
) -> [u8; signed_policy::ENCODED_LEN] {
    let public = AdmissionTrust {
        sequence,
        cluster_id: trust::cluster_id(b"boot-trust-tests").unwrap(),
        artifact_key: *pair(20).pk,
        deployment_key: *pair(21).pk,
        operations_key: *pair(operations_seed).pk,
        recipient_key: operations::recipient_public_key(&[77; 32]).unwrap(),
    };
    let policy = ProductionTrustCandidate::validate(public, &public.cluster_id, 1).unwrap();
    let bootstrap = pair(31);
    let mut bytes = signed_policy::encode_unsigned(
        &policy,
        &BootstrapKey::new(*bootstrap.pk).unwrap(),
        predecessor,
    )
    .unwrap();
    let signature = bootstrap.sk.sign(signed_policy::signature_digest(&bytes).unwrap(), None);
    signed_policy::set_signature(&mut bytes, signature.as_ref().try_into().unwrap()).unwrap();
    bytes
}

fn main() -> std::io::Result<()> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../catten/src/service/admission");
    let current = record(7, 22, [0; 32]);
    let predecessor = signed_policy::signature_digest(&current).unwrap();
    for (name, bytes) in [
        ("test-policy.bin", current),
        ("test-policy-previous.bin", record(6, 22, [0; 32])),
        ("test-policy-conflict.bin", record(7, 23, [0; 32])),
        ("test-policy-next.bin", record(8, 22, predecessor)),
    ] {
        let path = directory.join(name);
        fs::write(&path, bytes)?;
        println!("{}", path.display());
    }
    Ok(())
}
