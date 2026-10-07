//! Bounded, public-only trust-policy preparation. This does not provision keys.

use charlotte_launch::trust::{
    self,
    signed_policy::{
        self,
        BootstrapKey,
        PolicyExpectation,
    },
    AdmissionTrust,
    ProductionTrustCandidate,
};

use super::*;

fn read_bounded_regular(path: &str, maximum: usize) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options.open(path).map_err(|error| format!("open {path}: {error}"))?;
    let metadata = file.metadata().map_err(|error| format!("inspect {path}: {error}"))?;
    if !metadata.is_file() || metadata.len() > maximum as u64 {
        return Err(format!("{path}: expected a regular file of at most {maximum} bytes"));
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {path}: {error}"))?;
    if bytes.len() > maximum {
        return Err(format!("{path}: input grew beyond {maximum} bytes"));
    }
    Ok(bytes)
}

fn read_public(path: &str) -> Result<[u8; 32]> {
    let bytes = read_bounded_regular(path, 4096)?;
    let contents =
        std::str::from_utf8(&bytes).map_err(|_| "public key must be UTF-8 hex".to_owned())?;
    let encoded = development_fixture_hex(contents);
    parse_fixed_hex(&encoded, 32, "public key")?
        .try_into()
        .map_err(|_| "public key must contain exactly 32 bytes".to_owned())
}

fn identity(mnemonic: &str) -> Result<[u8; 32]> {
    trust::cluster_id(mnemonic.as_bytes())
        .ok_or_else(|| "cluster mnemonic must not be empty".to_owned())
}

pub(super) fn create(args: &[String]) -> Result<()> {
    if args.len() != 7 {
        return Err(concat!(
            "usage: cluster-sign trust-policy-create <output> <cluster-mnemonic> <sequence> ",
            "<artifact-public-file> <deployment-public-file> ",
            "<operations-public-file> <recipient-public-file>"
        )
        .to_owned());
    }
    let cluster_id = identity(&args[1])?;
    let trust = AdmissionTrust {
        sequence: parse_u64(args.get(2), "policy sequence", 1)?,
        cluster_id,
        artifact_key: read_public(&args[3])?,
        deployment_key: read_public(&args[4])?,
        operations_key: read_public(&args[5])?,
        recipient_key: read_public(&args[6])?,
    };
    let candidate = ProductionTrustCandidate::validate(trust, &cluster_id, 1)
        .map_err(|error| format!("trust-policy preflight failed: {error:?}"))?;
    let bytes = candidate.encode();
    write_new_file(&args[0], &bytes, false)?;
    println!(
        "wrote public trust candidate: {} sha256={}",
        args[0],
        hex_encode(&charlotte_launch::sha256::digest(&bytes))
    );
    println!(concat!(
        "unsigned preflight only; authenticated provisioning and ",
        "private-key custody are still required"
    ));
    Ok(())
}

pub(super) fn check(args: &[String]) -> Result<()> {
    if args.len() != 3 {
        return Err(concat!(
            "usage: cluster-sign trust-policy-check <input> ",
            "<cluster-mnemonic> <minimum-sequence>"
        )
        .to_owned());
    }
    let bytes = read_bounded_regular(&args[0], trust::ENCODED_LEN)?;
    let cluster_id = identity(&args[1])?;
    let minimum_sequence = parse_u64(args.get(2), "minimum policy sequence", 1)?;
    let candidate = ProductionTrustCandidate::decode(&bytes, &cluster_id, minimum_sequence)
        .map_err(|error| format!("trust-policy preflight failed: {error:?}"))?;
    println!(
        "public trust candidate passed preflight: sequence={} cluster={} sha256={}",
        candidate.public().sequence,
        hex_encode(&cluster_id),
        hex_encode(&charlotte_launch::sha256::digest(&bytes))
    );
    println!(concat!(
        "cluster and revision floor are caller-supplied; ",
        "this check does not authenticate or install the policy"
    ));
    Ok(())
}

fn expectation(args: &[String]) -> Result<PolicyExpectation> {
    let revision = parse_u64(args.get(1), "policy revision", 0)?;
    let state = match (args.first().map(String::as_str), args.len()) {
        (Some("enroll"), 2) => PolicyExpectation::enrollment(revision),
        (Some("installed"), 3) => {
            let digest = parse_fixed_hex(&args[2], 32, "accepted policy digest")?
                .try_into()
                .map_err(|_| "accepted policy digest must be 32 bytes".to_owned())?;
            PolicyExpectation::installed(revision, digest)
        }
        _ => {
            return Err(concat!(
                "expected enroll <minimum-sequence> or ",
                "installed <accepted-sequence> <accepted-digest-hex>"
            )
            .to_owned())
        }
    };
    state.map_err(|error| format!("invalid policy expectation: {error:?}"))
}

pub(super) fn sign(args: &[String]) -> Result<()> {
    if !(6..=7).contains(&args.len()) {
        return Err(concat!(
            "usage: cluster-sign trust-policy-sign <output> <candidate> <bootstrap-private-file> ",
            "<cluster-mnemonic> (enroll <minimum-sequence> | ",
            "installed <accepted-sequence> <accepted-digest-hex>)"
        )
        .to_owned());
    }
    let expected = expectation(&args[4..])?;
    let cluster = identity(&args[3])?;
    let candidate = ProductionTrustCandidate::decode(
        &read_bounded_regular(&args[1], trust::ENCODED_LEN)?,
        &cluster,
        1,
    )
    .map_err(|error| format!("trust-policy preflight failed: {error:?}"))?;
    // This command has no legacy argv-secret exception, even for fixtures.
    // Keep secret text/decoded storage in the existing zeroizing file reader.
    let key = SecretKey::from_slice(&read_private_key(&args[2], SecretKey::BYTES)?)
        .map_err(|_| "invalid bootstrap private key".to_owned())?;
    let public = key.public_key();
    key.validate_public_key(&public)
        .map_err(|_| "bootstrap private/public halves do not match".to_owned())?;
    let bootstrap =
        BootstrapKey::new(*public).map_err(|error| format!("invalid bootstrap key: {error:?}"))?;
    let predecessor = expected
        .signing_predecessor(candidate.public().sequence)
        .map_err(|error| format!("policy cannot be signed as this revision: {error:?}"))?;
    let mut bytes = signed_policy::encode_unsigned(&candidate, &bootstrap, predecessor)
        .map_err(|error| format!("invalid signed policy: {error:?}"))?;
    let signature = key.sign(
        signed_policy::signature_digest(&bytes)
            .map_err(|error| format!("invalid policy signing digest: {error:?}"))?,
        None,
    );
    signed_policy::set_signature(
        &mut bytes,
        signature.as_ref().try_into().map_err(|_| "invalid signature length".to_owned())?,
    )
    .map_err(|error| format!("cannot set policy signature: {error:?}"))?;
    let verified = signed_policy::verify(&bytes, &bootstrap, &cluster, &expected)
        .map_err(|error| format!("signed policy self-verification failed: {error:?}"))?;
    write_new_file(&args[0], &bytes, false)?;
    println!(
        "wrote signed public policy: {} sequence={} accepted-digest={}",
        args[0],
        verified.policy().public().sequence,
        hex_encode(&verified.digest())
    );
    println!("signing does not install trust or advance protected acceptance state");
    Ok(())
}

pub(super) fn verify(args: &[String]) -> Result<()> {
    if !(5..=6).contains(&args.len()) {
        return Err(concat!(
            "usage: cluster-sign trust-policy-verify <signed-policy> <bootstrap-public-file> ",
            "<cluster-mnemonic> (enroll <minimum-sequence> | ",
            "installed <accepted-sequence> <accepted-digest-hex>)"
        )
        .to_owned());
    }
    let expected = expectation(&args[3..])?;
    let cluster = identity(&args[2])?;
    let bootstrap = BootstrapKey::new(read_public(&args[1])?)
        .map_err(|error| format!("invalid bootstrap key: {error:?}"))?;
    let bytes = read_bounded_regular(&args[0], signed_policy::ENCODED_LEN)?;
    let verified = signed_policy::verify(&bytes, &bootstrap, &cluster, &expected)
        .map_err(|error| format!("signed policy verification failed: {error:?}"))?;
    println!(
        "signed public policy verified: sequence={} accepted-digest={}",
        verified.policy().public().sequence,
        hex_encode(&verified.digest())
    );
    println!(concat!(
        "verification uses caller-supplied anchor/state; ",
        "production still requires protected boot, state and recipient custody"
    ));
    Ok(())
}
