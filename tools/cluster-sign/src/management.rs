//! Complete, bounded management startup resources. Private inputs are file-only;
//! erasing owners precede reads/encoding. Verification does not install trust.

use charlotte_launch::{
    management::{
        self,
        sealed,
        Enrollment,
        Limits,
        ProfileInput,
        Role,
    },
    trust::AdmissionTrust,
};

use super::*;

pub(super) fn limits_default(args: &[String]) -> Result<()> {
    if args.len() != 1 {
        return Err("usage: cluster-sign management-limits-default <output>".into());
    }
    let mut text = String::new();
    for (name, value) in
        management::LIMIT_NAMES.iter().zip(management::limit_values(Limits::DEFAULT))
    {
        text.push_str(&format!("{name}={value}\n"));
    }
    write_new_file(&args[0], text.as_bytes(), false)
}

fn limits(path: &str) -> Result<Limits> {
    let bytes = read_bounded_regular(path, management::MAX_PROFILE_BYTES)?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| "management limits must be UTF-8".to_owned())?;
    let mut values = [None; management::LIMIT_NAMES.len()];
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once('=').ok_or("expected management limit NAME=NUMBER")?;
        let index = management::LIMIT_NAMES
            .iter()
            .position(|candidate| *candidate == name)
            .ok_or("unknown management limit")?;
        if values[index].is_some() {
            return Err("duplicate management limit".into());
        }
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("management limit must be an unsigned decimal integer".into());
        }
        values[index] = Some(value.parse::<u64>().map_err(|_| "management limit overflow")?);
    }
    let mut complete = [0; management::LIMIT_NAMES.len()];
    for (target, source) in complete.iter_mut().zip(values) {
        *target = source.ok_or("missing management limit")?;
    }
    management::limits_from_values(complete)
        .and_then(Limits::validate)
        .map_err(|error| format!("invalid management limits: {error:?}"))
}

pub(super) fn profile_create(args: &[String]) -> Result<()> {
    if args.len() < 6 {
        return Err(concat!(
            "usage: cluster-sign management-profile-create <output> <limits-file> \
             <server-key.der> ",
            "chain=<server-cert.der>... root=<client-ca.der>... ",
            "(reader=<operator-cert.der>|administrator=<operator-cert.der>)..."
        )
        .into());
    }
    let limits = limits(&args[1])?;
    let mut chain_paths = Vec::new();
    let mut root_paths = Vec::new();
    let mut operator_paths = Vec::new();
    for argument in &args[3..] {
        let (kind, path) = argument
            .split_once('=')
            .filter(|(_, path)| !path.is_empty())
            .ok_or("expected management FILE assignment")?;
        match kind {
            "chain" => chain_paths.push(path),
            "root" => root_paths.push(path),
            "reader" => operator_paths.push((Role::Reader, path)),
            "administrator" => operator_paths.push((Role::Administrator, path)),
            _ => return Err("unknown management resource assignment".into()),
        }
    }
    if chain_paths.is_empty()
        || chain_paths.len() > limits.max_chain_certificates
        || root_paths.is_empty()
        || root_paths.len() > limits.max_roots
        || operator_paths.is_empty()
        || operator_paths.len() > limits.max_enrollments
    {
        return Err("management resource count exceeds configured capacity".into());
    }
    let maximum = limits.max_certificate_bytes.min(management::MAX_PROFILE_BYTES);
    let key = profile_input::read_bounded_private_file(&args[2], maximum)
        .map_err(|error| format!("read management key: {error}"))?;
    let chain = chain_paths
        .iter()
        .map(|path| read_bounded_regular(path, maximum))
        .collect::<Result<Vec<_>>>()?;
    let roots = root_paths
        .iter()
        .map(|path| read_bounded_regular(path, maximum))
        .collect::<Result<Vec<_>>>()?;
    let operators = operator_paths
        .iter()
        .map(|(role, path)| {
            let certificate = read_bounded_regular(path, maximum)?;
            if certificate.is_empty() {
                return Err("empty management operator certificate".into());
            }
            Ok(Enrollment {
                certificate_sha256: charlotte_launch::sha256::digest(&certificate),
                role: *role,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let chain = chain.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let roots = roots.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let input = ProfileInput {
        limits,
        server_chain: &chain,
        server_key: key.as_bytes(),
        client_roots: &roots,
        operators: &operators,
    };
    let length = management::encoded_len(&input)
        .map_err(|error| format!("invalid management profile: {error:?}"))?;
    let mut bytes = Zeroizing::new(vec![0; length]);
    management::encode(&input, &mut bytes)
        .map_err(|error| format!("encode management profile: {error:?}"))?;
    write_new_file(&args[0], &bytes, true)?;
    println!("wrote private management profile: {} (mode 0600)", args[0]);
    Ok(())
}

fn policy(path: &str) -> Result<AdmissionTrust> {
    let bytes = read_bounded_regular(path, charlotte_launch::trust::ENCODED_LEN)?;
    AdmissionTrust::decode(&bytes).ok_or_else(|| "malformed public admission policy".into())
}

pub(super) fn development_policy(args: &[String]) -> Result<()> {
    if args.len() != 2 {
        return Err("usage: cluster-sign management-policy-development <output> \
                    <cluster-mnemonic>"
            .into());
    }
    let trust = charlotte_launch::development_admission_trust(args[1].as_bytes())
        .ok_or("invalid development cluster")?;
    write_new_file(&args[0], &trust.encode().unwrap(), false)?;
    println!("exported public development fixture policy; not production enrollment");
    Ok(())
}

pub(super) fn resource_seal(args: &[String]) -> Result<()> {
    if args.len() != 4 {
        return Err("usage: cluster-sign management-resource-seal <output> <public-policy> \
                    <operations-private-key-file> <private-profile>"
            .into());
    }
    let trust = policy(&args[1])?;
    let private = read_private_key(&args[2], 64)?;
    let signing = SecretKey::from_slice(&private).map_err(|_| "invalid management signing key")?;
    if signing.public_key().as_ref() != trust.operations_key {
        return Err("management signing key does not match policy".into());
    }
    let profile = profile_input::read_bounded_private_file(&args[3], management::MAX_PROFILE_BYTES)
        .map_err(|error| format!("read management profile: {error}"))?;
    let mut resource = Zeroizing::new(vec![0; sealed::HEADER_LEN + profile.as_bytes().len()]);
    sealed::seal_unsigned(
        profile.as_bytes(),
        &trust,
        &mut UnwrapErr(getrandom::SysRng),
        &mut resource,
    )
    .map_err(|error| format!("seal management resource: {error:?}"))?;
    let signature = signing.sign(
        sealed::signature_digest(&resource)
            .map_err(|error| format!("management signature input: {error:?}"))?,
        None,
    );
    sealed::set_signature(&mut resource, signature.as_ref().try_into().unwrap())
        .map_err(|error| format!("management signature: {error:?}"))?;
    sealed::verify(&resource, &trust)
        .map_err(|error| format!("management resource self-check: {error:?}"))?;
    write_new_file(&args[0], &resource, false)?;
    println!(
        "sealed management resource {} policy={} sha256={}",
        args[0],
        hex_encode(&sealed::policy_digest(&trust).unwrap()),
        hex_encode(&charlotte_launch::sha256::digest(&resource))
    );
    Ok(())
}

pub(super) fn resource_verify(args: &[String]) -> Result<()> {
    if args.len() != 2 {
        return Err(
            "usage: cluster-sign management-resource-verify <resource> <public-policy>".into()
        );
    }
    let trust = policy(&args[1])?;
    let resource = read_bounded_regular(&args[0], sealed::MAX_RESOURCE_BYTES)?;
    sealed::verify(&resource, &trust)
        .map_err(|error| format!("management resource rejected: {error:?}"))?;
    println!(
        "management signature/context verified against supplied public policy; no installation or \
         custody proof"
    );
    Ok(())
}
