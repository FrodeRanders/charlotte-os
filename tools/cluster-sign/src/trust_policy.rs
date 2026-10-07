//! Bounded, public-only trust-policy preparation. This does not provision keys.

use charlotte_launch::trust::{
    self,
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
