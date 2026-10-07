//! Offline signing and inspection for CharlotteOS cluster artifacts.
//!
//! The tool deliberately uses `charlotte-launch`'s parser, metadata encoder,
//! SHA-256, and verifier.  Host tooling and the kernel therefore cannot drift
//! into subtly different interpretations of the signed byte stream.

mod trust_policy;

#[cfg(unix)]
use std::os::unix::fs::{
    MetadataExt,
    OpenOptionsExt,
};
use std::{
    env,
    fs::{
        self,
        OpenOptions,
    },
    io::{
        Read,
        Write,
    },
    net::TcpStream,
    process::ExitCode,
    thread,
    time::{
        Duration,
        Instant,
    },
};

use charlotte_launch::{
    deployment::{
        self,
        CapabilityGrant,
        DescriptorFields,
    },
    ingress,
    ingress_policy,
    operations,
    operations_bundle,
    release,
    shutdown,
    signature_note::{
        self,
        ArtifactClass,
        ArtifactMetadata,
        DESCRIPTOR_LEN,
        NOTE_NAME,
        NOTE_TYPE_SIGNATURE,
        SIGNATURE_LEN,
    },
};
use ed25519_compact::{
    KeyPair,
    PublicKey,
    SecretKey,
    Signature,
};
use rand_core::UnwrapErr;
use zeroize::Zeroizing;

const NOTE_SECTION_NAME: &[u8] = b".note.charlotte-sig";
const SHT_NOTE: u32 = 7;
const ELF64_SECTION_HEADER_LEN: usize = 64;

type Result<T> = std::result::Result<T, String>;

fn align(value: usize, alignment: usize) -> Result<usize> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or_else(|| "ELF offset overflow".to_owned())
}

fn read_u16(image: &[u8], offset: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(
        image
            .get(offset..offset + 2)
            .ok_or_else(|| "truncated ELF".to_owned())?
            .try_into()
            .map_err(|_| "truncated ELF".to_owned())?,
    ))
}

fn read_u64(image: &[u8], offset: usize) -> Result<u64> {
    Ok(u64::from_le_bytes(
        image
            .get(offset..offset + 8)
            .ok_or_else(|| "truncated ELF".to_owned())?
            .try_into()
            .map_err(|_| "truncated ELF".to_owned())?,
    ))
}

/// `(shoff, shentsize, shnum, shstrndx)` for a conventional ELF64 image.
fn elf_section_table(image: &[u8]) -> Result<(usize, usize, usize, usize)> {
    if image.len() < 64 || image.get(0..4) != Some(b"\x7fELF") || image[4] != 2 || image[5] != 1 {
        return Err("input is not a little-endian ELF64 image".to_owned());
    }
    let shoff = usize::try_from(read_u64(image, 0x28)?)
        .map_err(|_| "section table offset does not fit usize".to_owned())?;
    let shentsize = usize::from(read_u16(image, 0x3a)?);
    let shnum = usize::from(read_u16(image, 0x3c)?);
    let shstrndx = usize::from(read_u16(image, 0x3e)?);
    if shentsize != ELF64_SECTION_HEADER_LEN || shnum == 0 || shstrndx >= shnum {
        return Err("unsupported ELF64 section table".to_owned());
    }
    let table_len =
        shentsize.checked_mul(shnum).ok_or_else(|| "section table size overflow".to_owned())?;
    let table_end =
        shoff.checked_add(table_len).ok_or_else(|| "section table offset overflow".to_owned())?;
    if table_end > image.len() {
        return Err("truncated ELF64 section table".to_owned());
    }
    Ok((shoff, shentsize, shnum, shstrndx))
}

fn build_note(metadata: ArtifactMetadata) -> Vec<u8> {
    let mut note = Vec::with_capacity(12 + 12 + DESCRIPTOR_LEN);
    note.extend_from_slice(&((NOTE_NAME.len() + 1) as u32).to_le_bytes());
    note.extend_from_slice(&(DESCRIPTOR_LEN as u32).to_le_bytes());
    note.extend_from_slice(&NOTE_TYPE_SIGNATURE.to_le_bytes());
    note.extend_from_slice(NOTE_NAME);
    note.push(0);
    while note.len() % 4 != 0 {
        note.push(0);
    }
    note.extend_from_slice(&signature_note::encode_descriptor(metadata));
    note
}

fn add_note_section(image: &[u8], metadata: ArtifactMetadata) -> Result<Vec<u8>> {
    let (shoff, shentsize, shnum, shstrndx) = elf_section_table(image)?;
    if shnum == usize::from(u16::MAX) {
        return Err("ELF has no room for another section header".to_owned());
    }
    let str_header = shoff + shstrndx * shentsize;
    let str_off = usize::try_from(read_u64(image, str_header + 0x18)?)
        .map_err(|_| "string table offset does not fit usize".to_owned())?;
    let str_size = usize::try_from(read_u64(image, str_header + 0x20)?)
        .map_err(|_| "string table size does not fit usize".to_owned())?;
    let str_end =
        str_off.checked_add(str_size).ok_or_else(|| "string table range overflow".to_owned())?;
    let old_strtab = image
        .get(str_off..str_end)
        .ok_or_else(|| "truncated section-name string table".to_owned())?;
    let old_table = &image[shoff..shoff + shentsize * shnum];
    let note = build_note(metadata);
    let note_offset = align(image.len(), 4)?;
    let new_str_offset = note_offset + note.len();
    let new_str_size = old_strtab
        .len()
        .checked_add(NOTE_SECTION_NAME.len() + 1)
        .ok_or_else(|| "string table size overflow".to_owned())?;
    let table_offset = align(new_str_offset + new_str_size, 8)?;

    let mut output = image.to_vec();
    output.resize(note_offset, 0);
    output.extend_from_slice(&note);
    output.extend_from_slice(old_strtab);
    output.extend_from_slice(NOTE_SECTION_NAME);
    output.push(0);
    output.resize(table_offset, 0);
    output.extend_from_slice(old_table);

    let mut new_header = [0u8; ELF64_SECTION_HEADER_LEN];
    new_header[0..4].copy_from_slice(&(old_strtab.len() as u32).to_le_bytes());
    new_header[4..8].copy_from_slice(&SHT_NOTE.to_le_bytes());
    new_header[0x18..0x20].copy_from_slice(&(note_offset as u64).to_le_bytes());
    new_header[0x20..0x28].copy_from_slice(&(note.len() as u64).to_le_bytes());
    new_header[0x30..0x38].copy_from_slice(&4u64.to_le_bytes());
    output.extend_from_slice(&new_header);

    output[0x28..0x30].copy_from_slice(&(table_offset as u64).to_le_bytes());
    output[0x3c..0x3e].copy_from_slice(&((shnum + 1) as u16).to_le_bytes());
    let moved_str_header = table_offset + shstrndx * shentsize;
    output[moved_str_header + 0x18..moved_str_header + 0x20]
        .copy_from_slice(&(new_str_offset as u64).to_le_bytes());
    output[moved_str_header + 0x20..moved_str_header + 0x28]
        .copy_from_slice(&(new_str_size as u64).to_le_bytes());
    Ok(output)
}

fn hex_decode(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return Err("hex input must contain an even number of digits".to_owned());
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .enumerate()
        .map(|(index, digits)| {
            decode_hex_pair(digits)
                .ok_or_else(|| format!("invalid hex digit at byte {}", index * 2))
        })
        .collect()
}

fn decode_hex_pair(digits: &[u8]) -> Option<u8> {
    fn nibble(byte: u8) -> Option<u8> {
        match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            b'A'..=b'F' => Some(byte - b'A' + 10),
            _ => None,
        }
    }
    Some(nibble(digits[0])? * 16 + nibble(digits[1])?)
}

fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[usize::from(byte >> 4)] as char);
        encoded.push(DIGITS[usize::from(byte & 15)] as char);
    }
    encoded
}

fn parse_class(value: Option<&String>) -> Result<ArtifactClass> {
    match value.map(String::as_str).unwrap_or("service") {
        "service" => Ok(ArtifactClass::Service),
        "driver" => Ok(ArtifactClass::Driver),
        "bootstrap" => Ok(ArtifactClass::Bootstrap),
        "admin" => Ok(ArtifactClass::Administration),
        other => Err(format!("unknown artifact class {other:?}")),
    }
}

fn parse_u64(value: Option<&String>, label: &str, default: u64) -> Result<u64> {
    value.map_or(Ok(default), |value| {
        value.parse().map_err(|_| format!("invalid {label}: {value:?}"))
    })
}

fn parse_u32(value: Option<&String>, label: &str, default: u32) -> Result<u32> {
    value.map_or(Ok(default), |value| {
        value
            .strip_prefix("0x")
            .map_or_else(|| value.parse(), |hex| u32::from_str_radix(hex, 16))
            .map_err(|_| format!("invalid {label}: {value:?}"))
    })
}

fn parse_digest(value: Option<&String>) -> Result<[u8; 32]> {
    let Some(value) = value else {
        return Ok([0; 32]);
    };
    if value == "-" {
        return Ok([0; 32]);
    }
    hex_decode(value)?
        .try_into()
        .map_err(|_| "provenance digest must contain exactly 32 bytes".to_owned())
}

fn parse_fixed_hex(value: &str, length: usize, label: &str) -> Result<Vec<u8>> {
    let bytes = hex_decode(value)?;
    if bytes.len() != length {
        return Err(format!("{label} must contain exactly {length} bytes"));
    }
    Ok(bytes)
}

fn read_hex_key(path: &str, length: usize, label: &str) -> Result<Vec<u8>> {
    let contents =
        Zeroizing::new(fs::read_to_string(path).map_err(|error| format!("read {path}: {error}"))?);
    let encoded = Zeroizing::new(
        contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect::<String>(),
    );
    parse_fixed_hex(&encoded, length, label)
}

// These are PUBLIC test fixtures, not a backdoor for arbitrary argv secrets.
// Keep old out-of-tree demo scripts working while refusing real hex keys.
fn development_fixture_hex(contents: &str) -> String {
    // Reserve once so filtering a private file cannot leave earlier growth
    // allocations containing fragments of its secret text.
    let mut encoded = String::with_capacity(contents.len());
    for line in contents.lines().map(str::trim) {
        if !line.is_empty() && !line.starts_with('#') {
            encoded.push_str(line);
        }
    }
    encoded
}

fn is_development_secret(encoded: &str) -> bool {
    [
        include_str!("../dev-key.hex"),
        include_str!("../dev-operations-key.hex"),
        include_str!("../dev-recipient-key.hex"),
    ]
    .iter()
    .any(|fixture| encoded.eq_ignore_ascii_case(&development_fixture_hex(fixture)))
}

fn read_private_key(path: &str, length: usize) -> Result<Zeroizing<Vec<u8>>> {
    const MAX_KEY_FILE_BYTES: u64 = 4096;
    let mut options = OpenOptions::new();
    options.read(true);
    // Inspect metadata on the opened descriptor, not a pathname checked and
    // reopened later. Nonblocking/no-follow also prevents FIFO/symlink traps.
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options.open(path).map_err(|_| "cannot open private-key file".to_owned())?;
    let metadata = file.metadata().map_err(|_| "cannot inspect private-key file".to_owned())?;
    if !metadata.is_file() || metadata.len() > MAX_KEY_FILE_BYTES {
        return Err("private key must be a regular file of at most 4096 bytes".to_owned());
    }
    let mut contents = Zeroizing::new(Vec::with_capacity(MAX_KEY_FILE_BYTES as usize + 1));
    file.take(MAX_KEY_FILE_BYTES + 1)
        .read_to_end(&mut contents)
        .map_err(|_| "cannot read private-key file".to_owned())?;
    if contents.len() as u64 > MAX_KEY_FILE_BYTES {
        return Err("private-key file exceeds 4096 bytes".to_owned());
    }
    let text =
        std::str::from_utf8(&contents).map_err(|_| "private-key file is not UTF-8".to_owned())?;
    let encoded = Zeroizing::new(development_fixture_hex(text));
    if !is_development_secret(&encoded) {
        #[cfg(unix)]
        {
            // Host tooling only; no Charlotte capability is acquired here.
            let effective_uid = unsafe { libc::geteuid() };
            if metadata.uid() != effective_uid || metadata.mode() & 0o077 != 0 {
                return Err("private-key file must be owned by this user with no group/other \
                            permissions (use mode 0600 or 0400)"
                    .to_owned());
            }
        }
        #[cfg(not(unix))]
        return Err("private-key file ACL enforcement is not implemented on this host".to_owned());
    }
    if encoded.len() != length * 2 {
        return Err(format!("private-key file must encode exactly {length} bytes"));
    }
    let mut decoded = Zeroizing::new(Vec::with_capacity(length));
    for digits in encoded.as_bytes().as_chunks::<2>().0 {
        decoded.push(decode_hex_pair(digits).ok_or_else(|| "invalid private-key hex".to_owned())?);
    }
    Ok(decoded)
}

fn read_signing_key(reference: &str) -> Result<SecretKey> {
    let bytes = if reference.len() == SecretKey::BYTES * 2
        && reference.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        if !reference.eq_ignore_ascii_case(&development_fixture_hex(include_str!("../dev-key.hex")))
        {
            return Err("private keys in argv are no longer accepted; pass a mode-0600 key-file \
                        path (not its contents)"
                .to_owned());
        }
        eprintln!("warning: legacy PUBLIC development fixture in argv; migrate to a key-file path");
        Zeroizing::new(hex_decode(reference)?)
    } else {
        read_private_key(reference, SecretKey::BYTES)?
    };
    SecretKey::from_slice(&bytes).map_err(|_| "invalid Ed25519 private-key file".to_owned())
}

fn write_new_file(path: &str, bytes: &[u8], secret: bool) -> Result<()> {
    #[cfg(not(unix))]
    if secret {
        return Err("private-file ACL enforcement is not implemented on this host".to_owned());
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(
        if secret {
            0o600
        } else {
            0o644
        },
    );
    let mut file = options.open(path).map_err(|error| format!("create {path}: {error}"))?;
    if let Err(error) = file.write_all(bytes) {
        drop(file);
        let cleanup = fs::remove_file(path);
        return Err(match cleanup {
            Ok(()) => format!("write {path}: {error}"),
            Err(cleanup) => format!("write {path}: {error}; remove partial file: {cleanup}"),
        });
    }
    Ok(())
}

fn write_new_hex_key(path: &str, bytes: &[u8], secret: bool) -> Result<()> {
    let mut encoded = Zeroizing::new(hex_encode(bytes));
    encoded.push('\n');
    write_new_file(path, encoded.as_bytes(), secret)
}

fn parse_profile_kind(value: &str) -> Result<u16> {
    match value {
        "s3" => Ok(operations::PROFILE_KIND_S3),
        "kafka" => Ok(operations::PROFILE_KIND_KAFKA),
        other => Err(format!("profile kind must be s3 or kafka, got {other:?}")),
    }
}

fn operations_recipient_generate(args: &[String]) -> Result<()> {
    let private_path = args.first().ok_or_else(|| "missing private-key output path".to_owned())?;
    let public_path = args.get(1).ok_or_else(|| "missing public-key output path".to_owned())?;
    let mut rng = UnwrapErr(getrandom::SysRng);
    let (private_key, public_key) = operations::generate_recipient_keypair(&mut rng);
    let private_key = Zeroizing::new(private_key);
    write_new_hex_key(private_path, &*private_key, true)?;
    if let Err(error) = write_new_hex_key(public_path, &public_key, false) {
        let _ = fs::remove_file(private_path);
        return Err(error);
    }
    println!(
        "generated distinct cluster-recipient key: public={} key-id={}",
        public_path,
        hex_encode(&operations::recipient_key_id(&public_key))
    );
    println!("private key written mode 0600 to {private_path}; never use it as an Ed25519 key");
    Ok(())
}

fn operations_signing_generate(args: &[String]) -> Result<()> {
    let private_path = args.first().ok_or_else(|| "missing private-key output path".to_owned())?;
    let public_path = args.get(1).ok_or_else(|| "missing public-key output path".to_owned())?;
    let pair = KeyPair::generate();
    write_new_hex_key(private_path, pair.sk.as_ref(), true)?;
    if let Err(error) = write_new_hex_key(public_path, pair.pk.as_ref(), false) {
        let _ = fs::remove_file(private_path);
        return Err(error);
    }
    println!(
        "generated distinct operational signing key: public={} key-id={}",
        public_path,
        hex_encode(&operations::signing_key_id(
            pair.pk.as_ref().try_into().map_err(|_| "invalid public key".to_owned())?
        ))
    );
    println!("private key written mode 0600 to {private_path}; do not use the artifact key here");
    Ok(())
}

fn signing_generate(args: &[String]) -> Result<()> {
    if args.len() != 2 {
        return Err("usage: cluster-sign generate <private-key-file> <public-key-file>".to_owned());
    }
    let pair = KeyPair::generate();
    write_new_hex_key(&args[0], pair.sk.as_ref(), true)?;
    if let Err(error) = write_new_hex_key(&args[1], pair.pk.as_ref(), false) {
        let _ = fs::remove_file(&args[0]);
        return Err(error);
    }
    println!("generated Ed25519 key: public={} key-id={}", args[1], hex_encode(pair.pk.as_ref()));
    println!("private key written mode 0600 to {}; private bytes are never printed", args[0]);
    Ok(())
}

fn operations_seal(args: &[String]) -> Result<()> {
    let output = args.first().ok_or_else(|| "missing envelope output path".to_owned())?;
    let profile_name = args.get(1).ok_or_else(|| "missing profile name".to_owned())?;
    let profile_kind =
        parse_profile_kind(args.get(2).ok_or_else(|| "missing profile kind".to_owned())?)?;
    let cluster_id: [u8; 32] = parse_fixed_hex(
        args.get(3).ok_or_else(|| "missing cluster id".to_owned())?,
        32,
        "cluster id",
    )?
    .try_into()
    .unwrap();
    let release_digest: [u8; 32] = parse_fixed_hex(
        args.get(4).ok_or_else(|| "missing release digest".to_owned())?,
        32,
        "release digest",
    )?
    .try_into()
    .unwrap();
    let sequence = parse_required_u64(
        args.get(5).ok_or_else(|| "missing operational sequence".to_owned())?,
        "operational sequence",
    )?;
    let expires_unix_seconds =
        parse_required_u64(args.get(6).ok_or_else(|| "missing expiry".to_owned())?, "expiry")?;
    let recipient_public: [u8; 32] = read_hex_key(
        args.get(7).ok_or_else(|| "missing recipient public-key path".to_owned())?,
        32,
        "recipient public key",
    )?
    .try_into()
    .unwrap();
    let operational_secret_bytes = read_private_key(
        args.get(8).ok_or_else(|| "missing operational signing-key path".to_owned())?,
        64,
    )?;
    let operational_secret = SecretKey::from_slice(&operational_secret_bytes)
        .map_err(|_| "invalid operational Ed25519 secret key".to_owned())?;
    let profile_path = args.get(9).ok_or_else(|| "missing plaintext profile path".to_owned())?;
    let profile = Zeroizing::new(
        fs::read(profile_path).map_err(|error| format!("read {profile_path}: {error}"))?,
    );
    let operational_public = operational_secret.public_key();
    let operational_public: &[u8; 32] =
        operational_public.as_ref().try_into().map_err(|_| "invalid public key".to_owned())?;
    let fields = operations::EnvelopeFields {
        sequence,
        expires_unix_seconds,
        profile_kind,
        cluster_id,
        release_digest,
        profile_name: profile_name.as_bytes(),
    };
    let len = operations::encoded_len(&fields, profile.len())
        .map_err(|error| format!("invalid operational envelope: {error:?}"))?;
    let mut envelope = vec![0u8; len];
    let mut rng = UnwrapErr(getrandom::SysRng);
    operations::seal_unsigned(
        &fields,
        &profile,
        &recipient_public,
        operational_public,
        &mut rng,
        &mut envelope,
    )
    .map_err(|error| format!("encrypt operational profile: {error:?}"))?;
    let digest = operations::signature_digest(&envelope)
        .ok_or_else(|| "encrypted operational envelope did not decode".to_owned())?;
    let signature: Signature = operational_secret.sign(digest, None);
    let signature: &[u8; operations::SIGNATURE_LEN] =
        signature.as_ref().try_into().map_err(|_| "invalid Ed25519 signature length".to_owned())?;
    if !operations::set_signature(&mut envelope, signature) {
        return Err("failed to install operational signature".to_owned());
    }
    write_new_file(output, &envelope, false)?;
    println!(
        "sealed operational profile {output}: name={profile_name:?} sequence={sequence} \
         recipient={} signing-key={} ciphertext={} bytes",
        hex_encode(&operations::recipient_key_id(&recipient_public)),
        hex_encode(&operations::signing_key_id(operational_public)),
        profile.len()
    );
    Ok(())
}

fn operations_verify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing envelope path".to_owned())?;
    let public_key: [u8; 32] = read_hex_key(
        args.get(1).ok_or_else(|| "missing operational public-key path".to_owned())?,
        32,
        "operational Ed25519 public key",
    )?
    .try_into()
    .unwrap();
    let envelope = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    if operations::verify(&envelope, &public_key) != operations::VerifyOutcome::Valid {
        return Err("operational envelope signature verification failed".to_owned());
    }
    let decoded = operations::decode(&envelope)
        .ok_or_else(|| "operational envelope is malformed".to_owned())?;
    println!(
        "VERIFY OK: name={:?} kind={} sequence={} expires={} recipient={}",
        String::from_utf8_lossy(decoded.profile_name),
        decoded.profile_kind,
        decoded.sequence,
        decoded.expires_unix_seconds,
        hex_encode(&decoded.recipient_key_id)
    );
    Ok(())
}

fn operations_open(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing envelope path".to_owned())?;
    let cluster_id: [u8; 32] = parse_fixed_hex(
        args.get(1).ok_or_else(|| "missing cluster id".to_owned())?,
        32,
        "cluster id",
    )?
    .try_into()
    .unwrap();
    let release_digest: [u8; 32] = parse_fixed_hex(
        args.get(2).ok_or_else(|| "missing release digest".to_owned())?,
        32,
        "release digest",
    )?
    .try_into()
    .unwrap();
    let now = parse_required_u64(
        args.get(3).ok_or_else(|| "missing current Unix time".to_owned())?,
        "current Unix time",
    )?;
    let recipient_bytes = read_private_key(
        args.get(4).ok_or_else(|| "missing recipient private-key path".to_owned())?,
        32,
    )?;
    let recipient_private =
        Zeroizing::new(<[u8; 32]>::try_from(recipient_bytes.as_slice()).unwrap());
    let operational_public: [u8; 32] = read_hex_key(
        args.get(5).ok_or_else(|| "missing operational public-key path".to_owned())?,
        32,
        "operational Ed25519 public key",
    )?
    .try_into()
    .unwrap();
    let output = args.get(6).ok_or_else(|| "missing plaintext output path".to_owned())?;
    let envelope = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    let decoded = operations::decode(&envelope)
        .ok_or_else(|| "operational envelope is malformed".to_owned())?;
    let mut plaintext = Zeroizing::new(vec![0u8; decoded.ciphertext.len()]);
    let len = operations::open(
        &envelope,
        &recipient_private,
        &operational_public,
        &cluster_id,
        &release_digest,
        now,
        &mut plaintext,
    )
    .map_err(|error| format!("open operational envelope: {error:?}"))?;
    write_new_file(output, &plaintext[..len], true)?;
    println!("opened operational profile to mode-0600 file {output}");
    Ok(())
}

fn operations_bundle_sign(args: &[String]) -> Result<()> {
    let output = args.first().ok_or_else(|| "missing bundle output path".to_owned())?;
    let sequence = parse_required_u64(
        args.get(1).ok_or_else(|| "missing bundle sequence".to_owned())?,
        "bundle sequence",
    )?;
    let cluster_id: [u8; 32] = parse_fixed_hex(
        args.get(2).ok_or_else(|| "missing cluster id".to_owned())?,
        32,
        "cluster id",
    )?
    .try_into()
    .unwrap();
    let release_public: [u8; 32] = parse_fixed_hex(
        args.get(3).ok_or_else(|| "missing release public key".to_owned())?,
        32,
        "release Ed25519 public key",
    )?
    .try_into()
    .unwrap();
    let operational_secret_bytes = read_private_key(
        args.get(4).ok_or_else(|| "missing operational signing-key path".to_owned())?,
        64,
    )?;
    let operational_secret = SecretKey::from_slice(&operational_secret_bytes)
        .map_err(|_| "invalid operational Ed25519 secret key".to_owned())?;
    let operational_public = operational_secret.public_key();
    let operational_public: &[u8; 32] = operational_public
        .as_ref()
        .try_into()
        .map_err(|_| "invalid operational public key".to_owned())?;
    let recipient_public: [u8; 32] = read_hex_key(
        args.get(5).ok_or_else(|| "missing recipient public-key path".to_owned())?,
        32,
        "recipient public key",
    )?
    .try_into()
    .unwrap();
    let release_path = args.get(6).ok_or_else(|| "missing signed release path".to_owned())?;
    let release =
        fs::read(release_path).map_err(|error| format!("read {release_path}: {error}"))?;
    if release::verify(&release, &release_public) != release::VerifyOutcome::Valid {
        return Err("release is not valid under the supplied release public key".to_owned());
    }
    let triples = args
        .get(7..)
        .filter(|values| !values.is_empty() && values.len() % 3 == 0)
        .ok_or_else(|| {
            "operations-bundle-sign requires target-artifact object-key envelope triples".to_owned()
        })?;
    let (triples, remainder) = triples.as_chunks::<3>();
    debug_assert!(remainder.is_empty());
    let envelope_paths = triples.iter().map(|triple| &triple[2]).collect::<Vec<_>>();
    let envelopes = envelope_paths
        .iter()
        .map(|path| fs::read(path).map_err(|error| format!("read {path}: {error}")))
        .collect::<Result<Vec<_>>>()?;
    let bindings = triples
        .iter()
        .zip(&envelopes)
        .map(|(triple, envelope)| operations_bundle::BindingFields {
            target_artifact: triple[0].as_bytes(),
            object_key: triple[1].as_bytes(),
            envelope,
        })
        .collect::<Vec<_>>();
    let fields = operations_bundle::BundleFields {
        sequence,
        cluster_id,
        release: &release,
        bindings: &bindings,
    };
    let len = operations_bundle::encoded_len(&fields)
        .map_err(|error| format!("invalid operational bundle: {error:?}"))?;
    let mut bundle = vec![0u8; len];
    operations_bundle::encode_unsigned(&fields, operational_public, &recipient_public, &mut bundle)
        .map_err(|error| format!("encode operational bundle: {error:?}"))?;
    for index in 0..bindings.len() {
        let digest = operations_bundle::binding_signature_digest(&bundle, index)
            .ok_or_else(|| "encoded compact binding did not decode".to_owned())?;
        let signature: Signature = operational_secret.sign(digest, None);
        let signature: &[u8; operations_bundle::BINDING_SIGNATURE_LEN] = signature
            .as_ref()
            .try_into()
            .map_err(|_| "invalid compact-binding signature length".to_owned())?;
        if !operations_bundle::set_binding_signature(&mut bundle, index, signature) {
            return Err("failed to install compact-binding signature".to_owned());
        }
    }
    let digest = operations_bundle::signature_digest(&bundle)
        .ok_or_else(|| "encoded operational bundle did not decode".to_owned())?;
    let signature: Signature = operational_secret.sign(digest, None);
    let signature: &[u8; operations_bundle::SIGNATURE_LEN] = signature
        .as_ref()
        .try_into()
        .map_err(|_| "invalid operational bundle signature length".to_owned())?;
    if !operations_bundle::set_signature(&mut bundle, signature) {
        return Err("failed to install operational bundle signature".to_owned());
    }
    write_new_file(output, &bundle, false)?;
    println!(
        "signed operational admission bundle {output}: release={release_path:?} \
         sequence={sequence} bindings={} bytes={}",
        bindings.len(),
        bundle.len()
    );
    Ok(())
}

fn operations_bundle_verify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing bundle path".to_owned())?;
    let cluster_id: [u8; 32] = parse_fixed_hex(
        args.get(1).ok_or_else(|| "missing cluster id".to_owned())?,
        32,
        "cluster id",
    )?
    .try_into()
    .unwrap();
    let release_public: [u8; 32] = parse_fixed_hex(
        args.get(2).ok_or_else(|| "missing release public key".to_owned())?,
        32,
        "release Ed25519 public key",
    )?
    .try_into()
    .unwrap();
    let operational_public: [u8; 32] = read_hex_key(
        args.get(3).ok_or_else(|| "missing operational public-key path".to_owned())?,
        32,
        "operational Ed25519 public key",
    )?
    .try_into()
    .unwrap();
    let recipient_public: [u8; 32] = read_hex_key(
        args.get(4).ok_or_else(|| "missing recipient public-key path".to_owned())?,
        32,
        "recipient public key",
    )?
    .try_into()
    .unwrap();
    let now = parse_required_u64(
        args.get(5).ok_or_else(|| "missing current Unix time".to_owned())?,
        "current Unix time",
    )?;
    let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    let outcome = operations_bundle::verify(
        &bytes,
        &release_public,
        &operational_public,
        &recipient_public,
        &cluster_id,
        now,
    );
    if outcome != operations_bundle::VerifyOutcome::Valid {
        return Err(format!("operational bundle verification failed: {outcome:?}"));
    }
    let bundle = operations_bundle::decode(&bytes)
        .ok_or_else(|| "operational bundle is malformed".to_owned())?;
    let release = release::decode(bundle.release)
        .ok_or_else(|| "operational bundle release is malformed".to_owned())?;
    println!(
        "VERIFY OK: release={:?} release-sha256={} sequence={} bindings={}",
        String::from_utf8_lossy(release.release_name),
        hex_encode(&bundle.release_digest),
        bundle.sequence,
        bundle.bindings().count()
    );
    for binding in bundle.bindings() {
        let envelope = operations::decode(binding.envelope)
            .ok_or_else(|| "operational binding envelope is malformed".to_owned())?;
        println!(
            "binding target={:?} profile={:?} object={:?} sequence={} expires={}",
            String::from_utf8_lossy(binding.target_artifact),
            String::from_utf8_lossy(envelope.profile_name),
            String::from_utf8_lossy(binding.object_key),
            envelope.sequence,
            envelope.expires_unix_seconds
        );
    }
    Ok(())
}

fn operations_bundle_notify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing bundle path".to_owned())?;
    let endpoint = args.get(1).map_or("127.0.0.1:8081", String::as_str);
    let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    operations_bundle::decode(&bytes)
        .ok_or_else(|| "operational bundle is malformed".to_owned())?;
    let mut stream = TcpStream::connect(endpoint)
        .map_err(|error| format!("connect to deployment ingress {endpoint}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| format!("set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| format!("set write timeout: {error}"))?;
    let header = format!(
        "POST /v1/operations HTTP/1.1\r\nHost: {endpoint}\r\nContent-Type: \
         application/vnd.charlotte.operations-bundle\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n",
        bytes.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|()| stream.write_all(&bytes))
        .map_err(|error| format!("send operational bundle: {error}"))?;
    let mut response = String::new();
    stream
        .take(8192)
        .read_to_string(&mut response)
        .map_err(|error| format!("read operational admission response: {error}"))?;
    let status = response.lines().next().unwrap_or_default();
    if !status.starts_with("HTTP/1.1 202 ") {
        return Err(format!("operational admission failed: {}", response.trim()));
    }
    println!("{}", response.split("\r\n\r\n").nth(1).unwrap_or(response.as_str()).trim());
    Ok(())
}

fn elf_sign(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing ELF path".to_owned())?;
    let name = args.get(1).ok_or_else(|| "missing artifact name".to_owned())?;
    let key_file = args.get(2).ok_or_else(|| "missing private-key file".to_owned())?;
    let class = parse_class(args.get(3))?;
    let version = parse_u64(args.get(4), "artifact version", 1)?;
    let rollback = parse_u64(args.get(5), "rollback counter", version)?;
    let flags = parse_u32(args.get(6), "artifact flags", 0)?;
    let provenance_digest = parse_digest(args.get(7))?;
    let secret = read_signing_key(key_file)?;
    let public = secret.public_key();
    let metadata = ArtifactMetadata::new(name.as_bytes(), class, version, rollback, flags)
        .ok_or_else(|| "artifact name must be 1..=48 non-NUL bytes".to_owned())?
        .with_key_id(public.as_ref().try_into().map_err(|_| "invalid public key".to_owned())?)
        .with_provenance_digest(provenance_digest);
    let mut image = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;

    if let Some(location) = signature_note::signature_location(&image) {
        image[location.descriptor_offset..location.descriptor_offset + DESCRIPTOR_LEN]
            .copy_from_slice(&signature_note::encode_descriptor(metadata));
    } else {
        image = add_note_section(&image, metadata)?;
    }
    let location = signature_note::signature_location(&image)
        .ok_or_else(|| "failed to construct canonical signature note".to_owned())?;
    let digest =
        charlotte_launch::sha256::digest_skipping(&image, location.signature_offset, SIGNATURE_LEN);
    let signature: Signature = secret.sign(digest, None);
    image[location.signature_offset..location.signature_offset + SIGNATURE_LEN]
        .copy_from_slice(signature.as_ref());
    fs::write(path, &image).map_err(|error| format!("write {path}: {error}"))?;
    println!(
        "signed {path} as {name:?} class={class:?} version={version} rollback={rollback} \
         flags={flags:#x}"
    );
    println!("public key id: {}", hex_encode(&metadata.key_id));
    println!("artifact sha256: {}", hex_encode(&charlotte_launch::sha256::digest(&image)));
    Ok(())
}

fn elf_verify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing ELF path".to_owned())?;
    let name = args.get(1).ok_or_else(|| "missing expected artifact name".to_owned())?;
    let key_hex = args.get(2).ok_or_else(|| "missing public key".to_owned())?;
    let key_bytes = hex_decode(key_hex)?;
    let public = PublicKey::from_slice(&key_bytes)
        .map_err(|_| "public key must contain 32 bytes".to_owned())?;
    let key: &[u8; 32] =
        public.as_ref().try_into().map_err(|_| "public key must contain 32 bytes".to_owned())?;
    let image = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    match signature_note::verify_elf_for_name(&image, key, name.as_bytes()) {
        signature_note::VerifyOutcome::Valid => {
            let metadata = signature_note::artifact_metadata(&image)
                .ok_or_else(|| "valid signature lacks metadata".to_owned())?;
            println!(
                "VERIFY OK: {path} is blessed as {:?}, class={:?}, version={}, rollback={}, \
                 flags={:#x}, provenance={}",
                String::from_utf8_lossy(metadata.name()),
                metadata.class,
                metadata.artifact_version,
                metadata.rollback_counter,
                metadata.flags,
                hex_encode(&metadata.provenance_digest),
            );
            Ok(())
        }
        outcome => Err(format!("verification failed: {outcome:?}")),
    }
}

fn parse_required_u64(value: &str, label: &str) -> Result<u64> {
    value
        .strip_prefix("0x")
        .map_or_else(|| value.parse(), |hex| u64::from_str_radix(hex, 16))
        .map_err(|_| format!("invalid {label}: {value:?}"))
}

fn parse_grant(value: &str) -> Result<(&[u8], u16)> {
    let (service, rights) = value
        .split_once('=')
        .ok_or_else(|| format!("grant must be SERVICE=RIGHTS, got {value:?}"))?;
    let rights = match rights {
        "send" => deployment::RIGHT_SEND,
        "call" => deployment::RIGHT_CALL,
        "client" => deployment::CLIENT_RIGHTS,
        "publish" => deployment::RIGHT_PUBLISH,
        _ => {
            return Err(format!(
                "grant rights must be send, call, client, or publish, got {rights:?}"
            ));
        }
    };
    Ok((service.as_bytes(), rights))
}

fn parse_u16_option(value: &str, label: &str) -> Result<u16> {
    parse_required_u64(value, label)?
        .try_into()
        .map_err(|_| format!("{label} exceeds the descriptor width"))
}

type ParsedPlacement<'a> = (charlotte_launch::placement::PlacementPolicy, Vec<(&'a [u8], u16)>);

fn parse_placement_and_grants(values: &[String]) -> Result<ParsedPlacement<'_>> {
    use charlotte_launch::placement::{
        PlacementPolicy,
        COLOCATE_AFFINITY_GROUP,
        EVERY_ELIGIBLE_NODE,
        SPREAD_REPLICAS,
    };

    let mut policy = PlacementPolicy::singleton();
    let mut min_distinct_explicit = false;
    let mut grants = Vec::new();
    for value in values {
        if let Some(raw) = value.strip_prefix("--replicas=") {
            policy.replicas = parse_u16_option(raw, "replica count")?;
            if !min_distinct_explicit {
                policy.min_distinct_nodes = policy.replicas;
            }
        } else if value == "--every-eligible-node" {
            policy.flags |= EVERY_ELIGIBLE_NODE;
            policy.replicas = 0;
            policy.min_distinct_nodes = 0;
        } else if let Some(raw) = value.strip_prefix("--max-instances-per-node=") {
            policy.max_instances_per_node = parse_u16_option(raw, "per-node instance limit")?;
        } else if let Some(raw) = value.strip_prefix("--min-distinct-nodes=") {
            policy.min_distinct_nodes = parse_u16_option(raw, "minimum distinct node count")?;
            min_distinct_explicit = true;
        } else if value == "--spread-replicas" {
            policy.flags |= SPREAD_REPLICAS;
        } else if let Some(raw) = value.strip_prefix("--affinity-group=") {
            policy.affinity_group = parse_required_u64(raw, "affinity group")?;
            policy.flags |= COLOCATE_AFFINITY_GROUP;
        } else if let Some(raw) = value.strip_prefix("--anti-affinity-group=") {
            policy.anti_affinity_group = parse_required_u64(raw, "anti-affinity group")?;
        } else {
            grants.push(parse_grant(value)?);
        }
    }
    policy.validate_shape().map_err(|error| format!("invalid placement policy: {error:?}"))?;
    Ok((policy, grants))
}

fn deployment_sign(args: &[String]) -> Result<()> {
    let output = args.first().ok_or_else(|| "missing descriptor output path".to_owned())?;
    let artifact_name = args.get(1).ok_or_else(|| "missing artifact name".to_owned())?;
    let object_key = args.get(2).ok_or_else(|| "missing object key".to_owned())?;
    let artifact_digest: [u8; 32] =
        hex_decode(args.get(3).ok_or_else(|| "missing artifact SHA-256".to_owned())?)?
            .try_into()
            .map_err(|_| "artifact SHA-256 must contain exactly 32 bytes".to_owned())?;
    let node_key =
        parse_required_u64(args.get(4).ok_or_else(|| "missing node key".to_owned())?, "node key")?;
    let sequence = parse_required_u64(
        args.get(5).ok_or_else(|| "missing deployment sequence".to_owned())?,
        "deployment sequence",
    )?;
    let stack_pages_per_thread = parse_required_u64(
        args.get(6).ok_or_else(|| "missing per-thread stack pages".to_owned())?,
        "per-thread stack pages",
    )?
    .try_into()
    .map_err(|_| "per-thread stack pages exceed the descriptor width".to_owned())?;
    let max_threads = parse_required_u64(
        args.get(7).ok_or_else(|| "missing maximum thread count".to_owned())?,
        "maximum thread count",
    )?
    .try_into()
    .map_err(|_| "maximum thread count exceeds the descriptor width".to_owned())?;
    let shutdown_grace_ms = parse_required_u64(
        args.get(8).ok_or_else(|| "missing shutdown grace milliseconds".to_owned())?,
        "shutdown grace milliseconds",
    )?
    .try_into()
    .map_err(|_| "shutdown grace milliseconds exceed the descriptor width".to_owned())?;
    let secret =
        read_signing_key(args.get(9).ok_or_else(|| "missing private-key file".to_owned())?)?;
    let (placement, parsed_grants) = parse_placement_and_grants(&args[10..])?;
    let grants = parsed_grants
        .iter()
        .map(|(service, rights)| CapabilityGrant {
            service,
            rights: *rights,
        })
        .collect::<Vec<_>>();
    let fields = DescriptorFields {
        sequence,
        node_key,
        artifact_digest,
        artifact_name: artifact_name.as_bytes(),
        stack_pages_per_thread,
        max_threads,
        shutdown_grace_ms,
        placement,
        object_key: object_key.as_bytes(),
        grants: &grants,
    };
    let public = secret.public_key();
    let public_key: &[u8; 32] =
        public.as_ref().try_into().map_err(|_| "invalid public key".to_owned())?;
    let len = deployment::encoded_len(&fields)
        .map_err(|error| format!("invalid deployment descriptor: {error:?}"))?;
    let mut bytes = vec![0; len];
    deployment::encode_unsigned(&fields, public_key, &mut bytes)
        .map_err(|error| format!("encode deployment descriptor: {error:?}"))?;
    let digest = deployment::signature_digest(&bytes)
        .ok_or_else(|| "encoded deployment descriptor did not decode".to_owned())?;
    let signature: Signature = secret.sign(digest, None);
    let signature: &[u8; deployment::SIGNATURE_LEN] =
        signature.as_ref().try_into().map_err(|_| "invalid Ed25519 signature length".to_owned())?;
    if !deployment::set_signature(&mut bytes, signature) {
        return Err("failed to install deployment signature".to_owned());
    }
    fs::write(output, &bytes).map_err(|error| format!("write {output}: {error}"))?;
    println!(
        "signed deployment {output}: artifact={artifact_name:?} object={object_key:?} \
         node={node_key:#x} sequence={sequence} stack_pages_per_thread={} max_threads={} \
         shutdown_grace_ms={} replicas={} placement_flags={:#x} grants={}",
        stack_pages_per_thread,
        max_threads,
        shutdown_grace_ms,
        placement.replicas,
        placement.flags,
        grants.len()
    );
    Ok(())
}

fn deployment_verify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing descriptor path".to_owned())?;
    let key_bytes: [u8; 32] =
        hex_decode(args.get(1).ok_or_else(|| "missing public key".to_owned())?)?
            .try_into()
            .map_err(|_| "public key must contain 32 bytes".to_owned())?;
    let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    if deployment::verify(&bytes, &key_bytes) != deployment::VerifyOutcome::Valid {
        return Err("deployment descriptor signature verification failed".to_owned());
    }
    let descriptor = deployment::decode(&bytes)
        .ok_or_else(|| "deployment descriptor is malformed".to_owned())?;
    println!(
        "VERIFY OK: artifact={:?} object={:?} node={:#x} sequence={} stack_pages_per_thread={} \
         max_threads={} shutdown_grace_ms={} replicas={} placement_flags={:#x}",
        String::from_utf8_lossy(descriptor.artifact_name),
        String::from_utf8_lossy(descriptor.object_key),
        descriptor.node_key,
        descriptor.sequence,
        descriptor.stack_pages_per_thread,
        descriptor.max_threads,
        descriptor.shutdown_grace_ms,
        descriptor.placement.replicas,
        descriptor.placement.flags
    );
    for grant in descriptor.grants() {
        println!("grant {:?} rights={:#x}", String::from_utf8_lossy(grant.service), grant.rights);
    }
    Ok(())
}

fn release_sign(args: &[String]) -> Result<()> {
    let output = args.first().ok_or_else(|| "missing release output path".to_owned())?;
    let release_name = args.get(1).ok_or_else(|| "missing release name".to_owned())?;
    let sequence = parse_required_u64(
        args.get(2).ok_or_else(|| "missing release sequence".to_owned())?,
        "release sequence",
    )?;
    let secret =
        read_signing_key(args.get(3).ok_or_else(|| "missing private-key file".to_owned())?)?;
    let paths = args.get(4..).filter(|paths| !paths.is_empty()).ok_or_else(|| {
        "release-sign requires at least one signed deployment descriptor".to_owned()
    })?;
    let descriptors = paths
        .iter()
        .map(|path| fs::read(path).map_err(|error| format!("read {path}: {error}")))
        .collect::<Result<Vec<_>>>()?;
    let descriptor_refs = descriptors.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let public = secret.public_key();
    let public_key: &[u8; 32] =
        public.as_ref().try_into().map_err(|_| "invalid public key".to_owned())?;
    for (path, descriptor) in paths.iter().zip(&descriptor_refs) {
        if deployment::verify(descriptor, public_key) != deployment::VerifyOutcome::Valid {
            return Err(format!("deployment descriptor {path:?} is not signed by the release key"));
        }
    }
    let fields = release::ReleaseFields {
        sequence,
        release_name: release_name.as_bytes(),
        descriptors: &descriptor_refs,
    };
    let len = release::encoded_len(&fields)
        .map_err(|error| format!("invalid release envelope: {error:?}"))?;
    let mut bytes = vec![0; len];
    release::encode_unsigned(&fields, public_key, &mut bytes)
        .map_err(|error| format!("encode release envelope: {error:?}"))?;
    let digest = release::signature_digest(&bytes)
        .ok_or_else(|| "encoded release envelope did not decode".to_owned())?;
    let signature: Signature = secret.sign(digest, None);
    let signature: &[u8; release::SIGNATURE_LEN] =
        signature.as_ref().try_into().map_err(|_| "invalid Ed25519 signature length".to_owned())?;
    if !release::set_signature(&mut bytes, signature) {
        return Err("failed to install release signature".to_owned());
    }
    fs::write(output, &bytes).map_err(|error| format!("write {output}: {error}"))?;
    println!(
        "signed release {output}: name={release_name:?} sequence={sequence} components={}",
        descriptor_refs.len()
    );
    Ok(())
}

fn release_verify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing release path".to_owned())?;
    let key_bytes: [u8; 32] =
        hex_decode(args.get(1).ok_or_else(|| "missing public key".to_owned())?)?
            .try_into()
            .map_err(|_| "public key must contain 32 bytes".to_owned())?;
    let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    if release::verify(&bytes, &key_bytes) != release::VerifyOutcome::Valid {
        return Err("release envelope signature verification failed".to_owned());
    }
    let envelope =
        release::decode(&bytes).ok_or_else(|| "release envelope is malformed".to_owned())?;
    println!(
        "VERIFY OK: release={:?} sequence={} components={}",
        String::from_utf8_lossy(envelope.release_name),
        envelope.sequence,
        envelope.descriptors().count()
    );
    for descriptor in envelope.descriptors() {
        let descriptor = deployment::decode(descriptor)
            .ok_or_else(|| "nested deployment descriptor is malformed".to_owned())?;
        println!(
            "component {:?} deployment-sequence={}",
            String::from_utf8_lossy(descriptor.artifact_name),
            descriptor.sequence
        );
    }
    Ok(())
}

fn shutdown_sign(args: &[String]) -> Result<()> {
    let output = args.first().ok_or_else(|| "missing shutdown-intent output path".to_owned())?;
    let sequence = parse_required_u64(
        args.get(1).ok_or_else(|| "missing shutdown sequence".to_owned())?,
        "shutdown sequence",
    )?;
    let target_node = parse_required_u64(
        args.get(2).ok_or_else(|| "missing target node".to_owned())?,
        "target node",
    )?;
    let not_before_unix_seconds = parse_required_u64(
        args.get(3).ok_or_else(|| "missing not-before UTC".to_owned())?,
        "not-before UTC",
    )?;
    let expires_unix_seconds = parse_required_u64(
        args.get(4).ok_or_else(|| "missing expiry UTC".to_owned())?,
        "expiry UTC",
    )?;
    let node_grace_ms = parse_required_u64(
        args.get(5).ok_or_else(|| "missing node grace milliseconds".to_owned())?,
        "node grace milliseconds",
    )?
    .try_into()
    .map_err(|_| "node grace milliseconds exceed the intent width".to_owned())?;
    let phase_grace_ms = parse_required_u64(
        args.get(6).ok_or_else(|| "missing phase grace milliseconds".to_owned())?,
        "phase grace milliseconds",
    )?
    .try_into()
    .map_err(|_| "phase grace milliseconds exceed the intent width".to_owned())?;
    let secret =
        read_signing_key(args.get(7).ok_or_else(|| "missing private-key file".to_owned())?)?;
    let fields = shutdown::ShutdownFields {
        sequence,
        target_node,
        not_before_unix_seconds,
        expires_unix_seconds,
        node_grace_ms,
        phase_grace_ms,
        reason: shutdown::REASON_POWER_OFF,
    };
    let public = secret.public_key();
    let public_key: &[u8; 32] =
        public.as_ref().try_into().map_err(|_| "invalid public key".to_owned())?;
    let mut bytes = vec![0; shutdown::ENCODED_LEN];
    shutdown::encode_unsigned(&fields, public_key, &mut bytes)
        .map_err(|error| format!("encode shutdown intent: {error:?}"))?;
    let digest = shutdown::signature_digest(&bytes)
        .ok_or_else(|| "encoded shutdown intent did not decode".to_owned())?;
    let signature: Signature = secret.sign(digest, None);
    let signature: &[u8; shutdown::SIGNATURE_LEN] =
        signature.as_ref().try_into().map_err(|_| "invalid Ed25519 signature length".to_owned())?;
    if !shutdown::set_signature(&mut bytes, signature) {
        return Err("failed to install shutdown signature".to_owned());
    }
    fs::write(output, &bytes).map_err(|error| format!("write {output}: {error}"))?;
    println!(
        "signed shutdown intent {output}: target={target_node:#x} sequence={sequence} \
         valid={not_before_unix_seconds}..={expires_unix_seconds} node_grace_ms={node_grace_ms} \
         phase_grace_ms={phase_grace_ms}"
    );
    Ok(())
}

fn shutdown_verify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing shutdown-intent path".to_owned())?;
    let key_bytes: [u8; 32] =
        hex_decode(args.get(1).ok_or_else(|| "missing public key".to_owned())?)?
            .try_into()
            .map_err(|_| "public key must contain 32 bytes".to_owned())?;
    let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    if shutdown::verify(&bytes, &key_bytes) != shutdown::VerifyOutcome::Valid {
        return Err("shutdown-intent signature verification failed".to_owned());
    }
    let fields =
        shutdown::decode(&bytes).ok_or_else(|| "shutdown intent is malformed".to_owned())?;
    println!(
        "VERIFY OK: target={:#x} sequence={} valid={}..={} node_grace_ms={} phase_grace_ms={} \
         reason={}",
        fields.target_node,
        fields.sequence,
        fields.not_before_unix_seconds,
        fields.expires_unix_seconds,
        fields.node_grace_ms,
        fields.phase_grace_ms,
        fields.reason
    );
    Ok(())
}

fn shutdown_notify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing shutdown-intent path".to_owned())?;
    let endpoint = args.get(1).map_or("127.0.0.1:8081", String::as_str);
    let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    shutdown::decode(&bytes).ok_or_else(|| "shutdown intent is malformed".to_owned())?;
    let mut stream = TcpStream::connect(endpoint)
        .map_err(|error| format!("connect to deployment ingress {endpoint}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| format!("set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| format!("set write timeout: {error}"))?;
    let header = format!(
        "POST /v1/shutdowns HTTP/1.1\r\nHost: {endpoint}\r\nContent-Type: \
         application/vnd.charlotte.shutdown\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|()| stream.write_all(&bytes))
        .map_err(|error| format!("send shutdown notification: {error}"))?;
    let mut response = String::new();
    stream
        .take(8192)
        .read_to_string(&mut response)
        .map_err(|error| format!("read shutdown response: {error}"))?;
    if !response.lines().next().unwrap_or_default().starts_with("HTTP/1.1 202 ") {
        return Err(format!("shutdown notification failed: {}", response.trim()));
    }
    println!("{}", response.split("\r\n\r\n").nth(1).unwrap_or_default().trim());
    Ok(())
}

fn parse_ingress_assignment(value: &str) -> Result<ingress::ServiceBinding<'_>> {
    let (backend_name, endpoint) = value
        .split_once('=')
        .map_or((None, value), |(name, endpoint)| (Some(name.as_bytes()), endpoint));
    let (address, port) = endpoint.rsplit_once(':').ok_or_else(|| {
        format!("invalid ingress assignment {value:?}; expected [NAME=]IPv4:PORT")
    })?;
    let octets = address
        .split('.')
        .map(|part| part.parse::<u8>().map_err(|_| format!("invalid IPv4 address {address:?}")))
        .collect::<Result<Vec<_>>>()?;
    let address: [u8; 4] =
        octets.try_into().map_err(|_| format!("invalid IPv4 address {address:?}"))?;
    let port = port.parse::<u16>().map_err(|_| format!("invalid TCP port in {value:?}"))?;
    let binding = ingress::ServiceBinding {
        service: ingress::ServiceId::tcp_v4(address, port),
        backend_name,
    };
    binding
        .is_valid()
        .then_some(binding)
        .ok_or_else(|| format!("invalid ingress assignment {value:?}"))
}

fn ingress_policy_sign(args: &[String]) -> Result<()> {
    let output = args.first().ok_or_else(|| "missing ingress-policy output path".to_owned())?;
    let sequence = parse_required_u64(
        args.get(1).ok_or_else(|| "missing ingress-policy sequence".to_owned())?,
        "ingress-policy sequence",
    )?;
    let not_before_unix_seconds = parse_required_u64(
        args.get(2).ok_or_else(|| "missing not-before UTC".to_owned())?,
        "not-before UTC",
    )?;
    let expires_unix_seconds = parse_required_u64(
        args.get(3).ok_or_else(|| "missing expiry UTC".to_owned())?,
        "expiry UTC",
    )?;
    let cluster_id: [u8; 32] = parse_fixed_hex(
        args.get(4).ok_or_else(|| "missing cluster id".to_owned())?,
        32,
        "cluster id",
    )?
    .try_into()
    .unwrap();
    let secret_bytes = read_private_key(
        args.get(5).ok_or_else(|| "missing operational signing-key path".to_owned())?,
        64,
    )?;
    let secret = SecretKey::from_slice(&secret_bytes)
        .map_err(|_| "invalid operational Ed25519 secret key".to_owned())?;
    let assignment_args = args.get(6..).unwrap_or_default();
    let assignments = if assignment_args == ["--clear"] {
        Vec::new()
    } else if assignment_args.is_empty() {
        return Err("provide at least one [NAME=]VIP:PORT assignment or --clear".to_owned());
    } else {
        assignment_args
            .iter()
            .map(|value| parse_ingress_assignment(value))
            .collect::<Result<Vec<_>>>()?
    };
    let fields = ingress_policy::PolicyFields {
        sequence,
        not_before_unix_seconds,
        expires_unix_seconds,
        cluster_id,
        assignments: &assignments,
    };
    let public = secret.public_key();
    let public: &[u8; 32] =
        public.as_ref().try_into().map_err(|_| "invalid public key".to_owned())?;
    let mut bytes = vec![
        0;
        ingress_policy::encoded_len(&fields)
            .map_err(|error| format!("invalid ingress policy: {error:?}"))?
    ];
    ingress_policy::encode_unsigned(&fields, public, &mut bytes)
        .map_err(|error| format!("encode ingress policy: {error:?}"))?;
    let signature: Signature = secret.sign(
        ingress_policy::signature_digest(&bytes)
            .ok_or_else(|| "encoded ingress policy did not decode".to_owned())?,
        None,
    );
    if !ingress_policy::set_signature(
        &mut bytes,
        signature.as_ref().try_into().map_err(|_| "invalid signature length".to_owned())?,
    ) {
        return Err("failed to install ingress-policy signature".to_owned());
    }
    write_new_file(output, &bytes, false)?;
    println!(
        "signed ingress policy {output}: sequence={sequence} assignments={} \
         valid={not_before_unix_seconds}..={expires_unix_seconds}",
        assignments.len()
    );
    Ok(())
}

fn ingress_policy_verify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing ingress-policy path".to_owned())?;
    let cluster_id: [u8; 32] = parse_fixed_hex(
        args.get(1).ok_or_else(|| "missing cluster id".to_owned())?,
        32,
        "cluster id",
    )?
    .try_into()
    .unwrap();
    let public_key: [u8; 32] = read_hex_key(
        args.get(2).ok_or_else(|| "missing operational public-key path".to_owned())?,
        32,
        "operational Ed25519 public key",
    )?
    .try_into()
    .unwrap();
    let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    if ingress_policy::verify(&bytes, &cluster_id, &public_key)
        != ingress_policy::VerifyOutcome::Valid
    {
        return Err("ingress-policy signature verification failed".to_owned());
    }
    let policy =
        ingress_policy::decode(&bytes).ok_or_else(|| "ingress policy is malformed".to_owned())?;
    println!(
        "VERIFY OK: sequence={} assignments={} valid={}..={}",
        policy.sequence,
        policy.assignments().count(),
        policy.not_before_unix_seconds,
        policy.expires_unix_seconds
    );
    Ok(())
}

fn ingress_policy_notify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing ingress-policy path".to_owned())?;
    let endpoint = args.get(1).map_or("127.0.0.1:8081", String::as_str);
    let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    ingress_policy::decode(&bytes).ok_or_else(|| "ingress policy is malformed".to_owned())?;
    let mut stream = TcpStream::connect(endpoint)
        .map_err(|error| format!("connect to deployment ingress {endpoint}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(30))))
        .map_err(|error| format!("configure ingress-policy connection: {error}"))?;
    let header = format!(
        "POST /v1/ingress-policy HTTP/1.1\r\nHost: {endpoint}\r\nContent-Type: \
         application/vnd.charlotte.ingress-policy\r\nContent-Length: {}\r\nConnection: \
         close\r\n\r\n",
        bytes.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|()| stream.write_all(&bytes))
        .map_err(|error| format!("send ingress-policy notification: {error}"))?;
    let mut response = String::new();
    stream
        .take(8192)
        .read_to_string(&mut response)
        .map_err(|error| format!("read ingress-policy response: {error}"))?;
    if !response.lines().next().unwrap_or_default().starts_with("HTTP/1.1 202 ") {
        return Err(format!("ingress-policy notification failed: {}", response.trim()));
    }
    println!("ingress policy accepted by {endpoint}");
    Ok(())
}

fn ingress_policy_status(args: &[String]) -> Result<()> {
    let endpoint = args.first().map_or("127.0.0.1:8081", String::as_str);
    let mut stream = TcpStream::connect(endpoint)
        .map_err(|error| format!("connect to deployment ingress {endpoint}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(30))))
        .map_err(|error| format!("configure ingress-policy connection: {error}"))?;
    let request =
        format!("GET /v1/ingress-policy HTTP/1.1\r\nHost: {endpoint}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("query ingress-policy status: {error}"))?;
    let mut response = String::new();
    stream
        .take(8192)
        .read_to_string(&mut response)
        .map_err(|error| format!("read ingress-policy status: {error}"))?;
    if !response.lines().next().unwrap_or_default().starts_with("HTTP/1.1 200 ") {
        return Err(format!("ingress-policy status failed: {}", response.trim()));
    }
    let body = response.split_once("\r\n\r\n").map_or("", |(_, body)| body);
    println!("{}", body.trim());
    Ok(())
}

fn node_key(args: &[String]) -> Result<()> {
    let mac = args.first().ok_or_else(|| "missing MAC address".to_owned())?;
    let octets = mac
        .split(':')
        .map(|octet| {
            if octet.len() != 2 {
                return Err(format!("invalid MAC address: {mac:?}"));
            }
            u8::from_str_radix(octet, 16).map_err(|_| format!("invalid MAC address: {mac:?}"))
        })
        .collect::<Result<Vec<_>>>()?;
    if octets.len() != 6 {
        return Err(format!("invalid MAC address: {mac:?}"));
    }
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for octet in octets {
        hash ^= u64::from(octet);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    println!("{:#x}", hash & 0xffff_ffff);
    Ok(())
}

fn deployment_notify_bytes(bytes: &[u8], endpoint: &str) -> Result<String> {
    deployment::decode(bytes).ok_or_else(|| "deployment descriptor is malformed".to_owned())?;
    let mut stream = TcpStream::connect(endpoint)
        .map_err(|error| format!("connect to deployment ingress {endpoint}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| format!("set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| format!("set write timeout: {error}"))?;
    let header = format!(
        "POST /v1/deployments HTTP/1.1\r\nHost: {endpoint}\r\nContent-Type: \
         application/vnd.charlotte.deployment\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|()| stream.write_all(bytes))
        .map_err(|error| format!("send deployment notification: {error}"))?;
    let mut response = String::new();
    stream
        .take(8192)
        .read_to_string(&mut response)
        .map_err(|error| format!("read deployment response: {error}"))?;
    let status = response.lines().next().unwrap_or_default();
    if !status.starts_with("HTTP/1.1 202 ") {
        return Err(format!("deployment notification failed: {}", response.trim()));
    }
    Ok(response.split("\r\n\r\n").nth(1).unwrap_or(response.as_str()).trim().to_owned())
}

fn release_notify_bytes(bytes: &[u8], endpoint: &str) -> Result<String> {
    release::decode(bytes).ok_or_else(|| "release envelope is malformed".to_owned())?;
    let mut stream = TcpStream::connect(endpoint)
        .map_err(|error| format!("connect to deployment ingress {endpoint}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| format!("set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|error| format!("set write timeout: {error}"))?;
    let header = format!(
        "POST /v1/releases HTTP/1.1\r\nHost: {endpoint}\r\nContent-Type: \
         application/vnd.charlotte.release\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|()| stream.write_all(bytes))
        .map_err(|error| format!("send release notification: {error}"))?;
    let mut response = String::new();
    stream
        .take(8192)
        .read_to_string(&mut response)
        .map_err(|error| format!("read release response: {error}"))?;
    let status = response.lines().next().unwrap_or_default();
    if !status.starts_with("HTTP/1.1 202 ") {
        return Err(format!("release notification failed: {}", response.trim()));
    }
    Ok(response.split("\r\n\r\n").nth(1).unwrap_or(response.as_str()).trim().to_owned())
}

fn release_notify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing release path".to_owned())?;
    let endpoint = args.get(1).map_or("127.0.0.1:8081", String::as_str);
    let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    println!("{}", release_notify_bytes(&bytes, endpoint)?);
    Ok(())
}

fn deployment_notify(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing descriptor path".to_owned())?;
    let endpoint = args.get(1).map_or("127.0.0.1:8081", String::as_str);
    let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    println!("{}", deployment_notify_bytes(&bytes, endpoint)?);
    Ok(())
}

fn percent_encode_path_segment(value: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::new();
    for byte in value {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(*byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
    }
    encoded
}

fn deployment_status_once(artifact_name: &str, endpoint: &str) -> Result<(String, String)> {
    if !deployment::valid_artifact_name(artifact_name.as_bytes()) {
        return Err("invalid deployment artifact name".to_owned());
    }
    let mut stream = TcpStream::connect(endpoint)
        .map_err(|error| format!("connect to deployment ingress {endpoint}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("set read timeout: {error}"))?;
    let path = percent_encode_path_segment(artifact_name.as_bytes());
    let request = format!(
        "GET /v1/deployments/{path} HTTP/1.1\r\nHost: {endpoint}\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("send deployment status request: {error}"))?;
    let mut response = String::new();
    stream
        .take(8192)
        .read_to_string(&mut response)
        .map_err(|error| format!("read deployment status response: {error}"))?;
    let status = response.lines().next().unwrap_or_default().to_owned();
    let body = response.split("\r\n\r\n").nth(1).unwrap_or_default().trim().to_owned();
    Ok((status, body))
}

fn deployment_status(args: &[String]) -> Result<()> {
    let artifact_name = args.first().ok_or_else(|| "missing artifact name".to_owned())?;
    let endpoint = args.get(1).map_or("127.0.0.1:8081", String::as_str);
    let timeout = args
        .get(2)
        .map(|value| value.parse::<u64>().map_err(|_| "invalid timeout seconds".to_owned()))
        .transpose()?
        .unwrap_or(0);
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        match deployment_status_once(artifact_name, endpoint) {
            Ok((status, body)) if status.starts_with("HTTP/1.1 200 ") => {
                if timeout == 0 || body.contains("\"state\":\"ready\"") {
                    println!("{body}");
                    return Ok(());
                }
            }
            Ok((status, body)) if timeout == 0 => {
                return Err(format!("deployment status failed: {status}: {body}"));
            }
            Err(error) if timeout == 0 => return Err(error),
            _ => {}
        }
        if Instant::now() >= deadline {
            return Err(format!("deployment {artifact_name:?} did not become ready in {timeout}s"));
        }
        thread::sleep(Duration::from_secs(1));
    }
}

/// Commit a set of independently signed descriptors and wait for all exact
/// generations to become ready. This is intentionally an orchestration layer:
/// artifacts must already be signed and uploaded, and the cluster remains the
/// authority that verifies and admits each descriptor.
fn deployment_apply(args: &[String]) -> Result<()> {
    let endpoint = args.first().ok_or_else(|| "missing deployment ingress host:port".to_owned())?;
    let timeout = args
        .get(1)
        .ok_or_else(|| "missing rollout timeout seconds".to_owned())?
        .parse::<u64>()
        .map_err(|_| "invalid rollout timeout seconds".to_owned())?;
    if timeout == 0 {
        return Err("rollout timeout must be greater than zero".to_owned());
    }
    let paths = args.get(2..).filter(|paths| !paths.is_empty()).ok_or_else(|| {
        "deployment-apply requires at least one signed descriptor path".to_owned()
    })?;

    let mut releases = Vec::with_capacity(paths.len());
    for path in paths {
        let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
        let descriptor = deployment::decode(&bytes)
            .ok_or_else(|| format!("deployment descriptor {path:?} is malformed"))?;
        let name = std::str::from_utf8(descriptor.artifact_name)
            .map_err(|_| format!("descriptor {path:?} has a non-UTF-8 artifact name"))?
            .to_owned();
        if releases.iter().any(|(existing, _, _)| existing == &name) {
            return Err(format!("release contains duplicate artifact name {name:?}"));
        }
        releases.push((name, path.clone(), bytes));
    }

    for (name, path, bytes) in &releases {
        let body = deployment_notify_bytes(bytes, endpoint).map_err(|error| {
            format!(
                "release stopped while notifying {name:?} from {path:?}: {error}; earlier \
                 descriptors may already be committed"
            )
        })?;
        println!("accepted {name:?}: {body}");
    }

    let deadline = Instant::now() + Duration::from_secs(timeout);
    let mut pending = releases.iter().map(|(name, _, _)| name.clone()).collect::<Vec<_>>();
    while !pending.is_empty() {
        let mut index = 0;
        while index < pending.len() {
            let name = &pending[index];
            match deployment_status_once(name, endpoint) {
                Ok((status, body))
                    if status.starts_with("HTTP/1.1 200 ")
                        && body.contains("\"state\":\"ready\"") =>
                {
                    println!("ready {name:?}: {body}");
                    pending.swap_remove(index);
                }
                _ => index += 1,
            }
        }
        if pending.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            pending.sort();
            return Err(format!(
                "release did not become ready in {timeout}s; pending: {}",
                pending.join(", ")
            ));
        }
        thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

fn release_apply(args: &[String]) -> Result<()> {
    let path = args.first().ok_or_else(|| "missing release path".to_owned())?;
    let endpoint = args.get(1).map_or("127.0.0.1:8081", String::as_str);
    let timeout = args
        .get(2)
        .map(|value| value.parse::<u64>().map_err(|_| "invalid rollout timeout seconds".to_owned()))
        .transpose()?
        .unwrap_or(120);
    if timeout == 0 {
        return Err("rollout timeout must be greater than zero".to_owned());
    }
    let bytes = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
    let envelope =
        release::decode(&bytes).ok_or_else(|| "release envelope is malformed".to_owned())?;
    let release_name = String::from_utf8_lossy(envelope.release_name);
    let mut pending = envelope
        .descriptors()
        .map(|bytes| {
            let descriptor = deployment::decode(bytes)
                .ok_or_else(|| "nested deployment descriptor is malformed".to_owned())?;
            std::str::from_utf8(descriptor.artifact_name)
                .map(str::to_owned)
                .map_err(|_| "nested deployment artifact name is not UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>>>()?;
    let body = release_notify_bytes(&bytes, endpoint)?;
    println!("accepted release {release_name:?}: {body}");

    let deadline = Instant::now() + Duration::from_secs(timeout);
    while !pending.is_empty() {
        let mut index = 0;
        while index < pending.len() {
            let name = &pending[index];
            match deployment_status_once(name, endpoint) {
                Ok((status, body))
                    if status.starts_with("HTTP/1.1 200 ")
                        && body.contains("\"state\":\"ready\"") =>
                {
                    println!("ready {name:?}: {body}");
                    pending.swap_remove(index);
                }
                _ => index += 1,
            }
        }
        if pending.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            pending.sort();
            return Err(format!(
                "release {release_name:?} did not become ready in {timeout}s; pending: {}",
                pending.join(", ")
            ));
        }
        thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("generate") => signing_generate(&args[2..]),
        Some("elf-sign") => elf_sign(&args[2..]),
        Some("elf-verify") => elf_verify(&args[2..]),
        Some("deployment-sign") => deployment_sign(&args[2..]),
        Some("deployment-verify") => deployment_verify(&args[2..]),
        Some("deployment-notify") => deployment_notify(&args[2..]),
        Some("deployment-status") => deployment_status(&args[2..]),
        Some("deployment-apply") => deployment_apply(&args[2..]),
        Some("release-sign") => release_sign(&args[2..]),
        Some("release-verify") => release_verify(&args[2..]),
        Some("release-notify") => release_notify(&args[2..]),
        Some("release-apply") => release_apply(&args[2..]),
        Some("shutdown-sign") => shutdown_sign(&args[2..]),
        Some("shutdown-verify") => shutdown_verify(&args[2..]),
        Some("shutdown-notify") => shutdown_notify(&args[2..]),
        Some("ingress-policy-sign") => ingress_policy_sign(&args[2..]),
        Some("ingress-policy-verify") => ingress_policy_verify(&args[2..]),
        Some("ingress-policy-notify") => ingress_policy_notify(&args[2..]),
        Some("ingress-policy-status") => ingress_policy_status(&args[2..]),
        Some("node-key") => node_key(&args[2..]),
        Some("trust-policy-create") => trust_policy::create(&args[2..]),
        Some("trust-policy-check") => trust_policy::check(&args[2..]),
        Some("trust-policy-sign") => trust_policy::sign(&args[2..]),
        Some("trust-policy-verify") => trust_policy::verify(&args[2..]),
        Some("operations-recipient-generate") => operations_recipient_generate(&args[2..]),
        Some("operations-signing-generate") => operations_signing_generate(&args[2..]),
        Some("operations-seal") => operations_seal(&args[2..]),
        Some("operations-verify") => operations_verify(&args[2..]),
        Some("operations-open") => operations_open(&args[2..]),
        Some("operations-bundle-sign") => operations_bundle_sign(&args[2..]),
        Some("operations-bundle-verify") => operations_bundle_verify(&args[2..]),
        Some("operations-bundle-notify") => operations_bundle_notify(&args[2..]),
        Some("cluster-id") => {
            let mnemonic = args.get(2).ok_or_else(|| "missing cluster mnemonic".to_owned())?;
            let id = charlotte_launch::trust::cluster_id(mnemonic.as_bytes())
                .ok_or_else(|| "cluster mnemonic must not be empty".to_owned())?;
            println!("{}", hex_encode(&id));
            Ok(())
        }
        Some("sha256") => {
            let path = args.get(2).ok_or_else(|| "missing file path".to_owned())?;
            let data = fs::read(path).map_err(|error| format!("read {path}: {error}"))?;
            println!("{}", hex_encode(&charlotte_launch::sha256::digest(&data)));
            Ok(())
        }
        Some("selftest") => {
            if charlotte_launch::sha256::digest(b"abc") != charlotte_launch::sha256::TEST_VECTOR_ABC
            {
                return Err("SHA-256 self-test failed".to_owned());
            }
            let metadata = ArtifactMetadata::new(
                b"greet",
                ArtifactClass::Service,
                7,
                7,
                signature_note::FLAG_PARALLEL_INSTANCES,
            )
            .ok_or_else(|| "metadata self-test construction failed".to_owned())?
            .with_key_id(&charlotte_launch::CLUSTER_PUBLIC_KEY)
            .with_provenance_digest([0x5a; 32]);
            let decoded =
                signature_note::decode_metadata(&signature_note::encode_descriptor(metadata))
                    .ok_or_else(|| "metadata self-test decode failed".to_owned())?;
            if decoded != metadata {
                return Err("metadata self-test round trip failed".to_owned());
            }
            let policy = charlotte_launch::placement::PlacementPolicy {
                replicas: 2,
                max_instances_per_node: 2,
                min_distinct_nodes: 1,
                flags: charlotte_launch::placement::COLOCATE_AFFINITY_GROUP,
                affinity_group: 42,
                anti_affinity_group: 0,
            };
            policy
                .validate(&metadata)
                .map_err(|error| format!("placement policy self-test failed: {error:?}"))?;
            let grants = [CapabilityGrant {
                service: b"kafka/orders/transactional",
                rights: deployment::CLIENT_RIGHTS,
            }];
            let fields = DescriptorFields {
                sequence: 7,
                node_key: 0,
                artifact_digest: [0xa5; 32],
                artifact_name: b"orders-step",
                stack_pages_per_thread: 16,
                max_threads: 8,
                shutdown_grace_ms: 5_000,
                placement: policy,
                object_key: b"releases/orders-step-a5.elf",
                grants: &grants,
            };
            let pair = KeyPair::generate();
            let public_key: &[u8; 32] = pair
                .pk
                .as_ref()
                .try_into()
                .map_err(|_| "invalid self-test public key".to_owned())?;
            let mut descriptor = vec![
                0;
                deployment::encoded_len(&fields).map_err(|error| {
                    format!("deployment descriptor self-test length failed: {error:?}")
                })?
            ];
            deployment::encode_unsigned(&fields, public_key, &mut descriptor).map_err(|error| {
                format!("deployment descriptor self-test encoding failed: {error:?}")
            })?;
            let digest = deployment::signature_digest(&descriptor)
                .ok_or_else(|| "deployment descriptor self-test decode failed".to_owned())?;
            let signature: Signature = pair.sk.sign(digest, None);
            let signature: &[u8; deployment::SIGNATURE_LEN] = signature
                .as_ref()
                .try_into()
                .map_err(|_| "invalid self-test signature".to_owned())?;
            if !deployment::set_signature(&mut descriptor, signature)
                || deployment::verify(&descriptor, public_key) != deployment::VerifyOutcome::Valid
            {
                return Err("deployment descriptor self-test verification failed".to_owned());
            }
            descriptor[24] ^= 1;
            if deployment::verify(&descriptor, public_key) == deployment::VerifyOutcome::Valid {
                return Err("mutated deployment descriptor was accepted".to_owned());
            }
            println!(
                "SHA-256, CLS2 metadata, placement-policy, and signed-deployment self-tests pass"
            );
            Ok(())
        }
        _ => {
            Err("usage: cluster-sign generate <private-key-file> <public-key-file> | elf-sign \
                 <elf> <name> <private-key-file> [service|driver|bootstrap|admin] [version] \
                 [rollback] [flags] [provenance-sha256|-] | elf-verify <elf> <name> <pubkey-hex> \
                 | sha256 <file> | deployment-sign <output> <artifact-name> <object-key> \
                 <artifact-sha256> <node-key> <sequence> <stack-pages-per-thread> <max-threads> \
                 <shutdown-grace-ms> <private-key-file> [--replicas=N | --every-eligible-node] \
                 [--min-distinct-nodes=N] [--max-instances-per-node=N] [--spread-replicas] \
                 [--affinity-group=N] [--anti-affinity-group=N] [service=send|call|client|publish \
                 ...] | deployment-verify <descriptor> <pubkey-hex> | deployment-notify \
                 <descriptor> [host:port] | deployment-status <artifact-name> [host:port] \
                 [wait-seconds] | deployment-apply <host:port> <wait-seconds> <descriptor>... | \
                 release-sign <output> <release-name> <sequence> <private-key-file> \
                 <descriptor>... | release-verify <release> <pubkey-hex> | release-notify \
                 <release> [host:port] | release-apply <release> [host:port] [wait-seconds] | \
                 shutdown-sign <output> <sequence> <target-node> <not-before-unix> <expires-unix> \
                 <node-grace-ms> <phase-grace-ms> <private-key-file> | shutdown-verify <intent> \
                 <pubkey-hex> | shutdown-notify <intent> [host:port] | ingress-policy-sign \
                 <output> <sequence> <not-before-unix> <expires-unix> <cluster-id-hex> \
                 <ops-ed25519-private-key-file> ([NAME=]VIP:PORT... | --clear) | \
                 ingress-policy-verify <policy> <cluster-id-hex> <ops-ed25519-public-key-file> | \
                 ingress-policy-notify <policy> [host:port] | ingress-policy-status [host:port] | \
                 node-key <mac-address> | trust-policy-create <output> <cluster-mnemonic> \
                 <sequence> <artifact-public-file> <deployment-public-file> \
                 <operations-public-file> <recipient-public-file> | trust-policy-check <input> \
                 <cluster-mnemonic> <minimum-sequence> | trust-policy-sign <output> <candidate> \
                 <bootstrap-private-file> <cluster-mnemonic> (enroll <minimum-sequence> | \
                 installed <accepted-sequence> <accepted-digest-hex>) | trust-policy-verify \
                 <signed-policy> <bootstrap-public-file> <cluster-mnemonic> (enroll \
                 <minimum-sequence> | installed <accepted-sequence> <accepted-digest-hex>) | \
                 operations-recipient-generate <private-key-file> <public-key-file> | \
                 operations-signing-generate <private-key-file> <public-key-file> | \
                 operations-seal <output> <profile-name> <s3|kafka> <cluster-id-hex> \
                 <release-sha256> <sequence> <expires-unix> <recipient-public-key-file> \
                 <ops-ed25519-private-key-file> <profile-file> | operations-verify <envelope> \
                 <ops-ed25519-public-key-file> | operations-open <envelope> <cluster-id-hex> \
                 <release-sha256> <now-unix> <recipient-private-key-file> \
                 <ops-ed25519-public-key-file> <output> | operations-bundle-sign <output> \
                 <bundle-sequence> <cluster-id-hex> <release-ed25519-public-key-hex> \
                 <ops-ed25519-private-key-file> <recipient-public-key-file> <release> \
                 (<target-artifact> <object-key> <envelope>)... | operations-bundle-verify \
                 <bundle> <cluster-id-hex> <release-ed25519-public-key-hex> \
                 <ops-ed25519-public-key-file> <recipient-public-key-file> <now-unix> | \
                 operations-bundle-notify <bundle> [host:port] | cluster-id <mnemonic> | selftest"
                .to_owned())
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    struct TestDirectory(std::path::PathBuf);

    #[cfg(unix)]
    impl TestDirectory {
        fn new() -> Self {
            use std::{
                os::unix::fs::DirBuilderExt,
                sync::atomic::{
                    AtomicUsize,
                    Ordering,
                },
            };
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = env::temp_dir().join(format!(
                "charlotte-key-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }

        fn path(&self, name: &str) -> String {
            self.0.join(name).to_str().unwrap().to_owned()
        }
    }

    #[cfg(unix)]
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            // Only this test's uniquely created, owned directory is removed.
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn hex_rejects_unicode_without_panicking() {
        for malformed in ["0é0", "é00", "☃x", "zz", "0"] {
            assert!(hex_decode(malformed).is_err());
        }
        assert_eq!(hex_decode("00aBff").unwrap(), [0, 0xab, 0xff]);
    }

    #[cfg(unix)]
    fn trust_policy_args(directory: &TestDirectory) -> Vec<String> {
        let mut args = vec![directory.path("policy.ctrust"), "policy-tests".into(), "7".into()];
        for role in ["artifact", "deployment", "operations"] {
            let path = directory.path(role);
            write_new_hex_key(&path, KeyPair::generate().pk.as_ref(), false).unwrap();
            args.push(path);
        }
        let recipient_path = directory.path("recipient");
        let recipient = operations::recipient_public_key(&[77; 32]).unwrap();
        write_new_hex_key(&recipient_path, &recipient, false).unwrap();
        args.push(recipient_path);
        args
    }

    #[cfg(unix)]
    fn signed_policy_args(directory: &TestDirectory) -> (Vec<String>, String) {
        let candidate = trust_policy_args(directory);
        trust_policy::create(&candidate).unwrap();
        let private = directory.path("bootstrap.hex");
        let public = directory.path("bootstrap.pub");
        signing_generate(&[private.clone(), public.clone()]).unwrap();
        (
            vec![
                directory.path("policy.signed"),
                candidate[0].clone(),
                private,
                candidate[1].clone(),
                "enroll".into(),
                "7".into(),
            ],
            public,
        )
    }

    #[cfg(unix)]
    #[test]
    fn signed_policy_commands_enroll_rotate_and_preserve_existing_output() {
        use charlotte_launch::trust::signed_policy::{
            self,
            BootstrapKey,
            PolicyExpectation,
        };
        let directory = TestDirectory::new();
        let (mut args, public_path) = signed_policy_args(&directory);
        trust_policy::sign(&args).unwrap();
        let original = fs::read(&args[0]).unwrap();
        trust_policy::verify(&[
            args[0].clone(),
            public_path.clone(),
            args[3].clone(),
            "enroll".into(),
            "7".into(),
        ])
        .unwrap();
        assert!(trust_policy::sign(&args).is_err());
        assert_eq!(fs::read(&args[0]).unwrap(), original);
        let public = BootstrapKey::new(
            read_hex_key(&public_path, 32, "public key").unwrap().try_into().unwrap(),
        )
        .unwrap();
        let cluster = charlotte_launch::trust::cluster_id(args[3].as_bytes()).unwrap();
        let accepted = signed_policy::verify(
            &original,
            &public,
            &cluster,
            &PolicyExpectation::enrollment(7).unwrap(),
        )
        .unwrap();
        let mut candidate =
            charlotte_launch::trust::AdmissionTrust::decode(&fs::read(&args[1]).unwrap()).unwrap();
        candidate.sequence = 8;
        fs::write(&args[1], candidate.encode().unwrap()).unwrap();
        args[0] = directory.path("rotated.signed");
        args[4] = "installed".into();
        args.push(hex_encode(&accepted.digest()));
        trust_policy::sign(&args).unwrap();
        trust_policy::verify(&[
            args[0].clone(),
            public_path.clone(),
            args[3].clone(),
            "installed".into(),
            "7".into(),
            args[6].clone(),
        ])
        .unwrap();
        let next = signed_policy::verify(
            &fs::read(&args[0]).unwrap(),
            &public,
            &cluster,
            &accepted.installed_expectation().unwrap(),
        )
        .unwrap();
        assert!(trust_policy::verify(&[
            directory.path("policy.signed"),
            public_path.clone(),
            args[3].clone(),
            "installed".into(),
            "8".into(),
            hex_encode(&next.digest())
        ])
        .is_err());
        trust_policy::verify(&[
            args[0].clone(),
            public_path.clone(),
            args[3].clone(),
            "installed".into(),
            "8".into(),
            hex_encode(&next.digest()),
        ])
        .unwrap();
        assert!(trust_policy::verify(&[
            args[0].clone(),
            public_path.clone(),
            "foreign".into(),
            "installed".into(),
            "8".into(),
            hex_encode(&next.digest())
        ])
        .is_err());
        // A different accepted revision/digest is not silently replaced by the
        // record's own values or retried as fresh enrollment.
        assert!(trust_policy::verify(&[
            args[0].clone(),
            public_path,
            args[3].clone(),
            "installed".into(),
            "8".into(),
            "a5".repeat(32)
        ])
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn signed_policy_signing_rejects_unsafe_keys_context_and_role_reuse_before_output() {
        use std::os::unix::fs::PermissionsExt;
        let directory = TestDirectory::new();
        let (args, public_path) = signed_policy_args(&directory);
        let mut invalid = args.clone();
        invalid[5] = "8".into();
        assert!(trust_policy::sign(&invalid).is_err());
        invalid = args.clone();
        invalid[3] = "foreign".into();
        assert!(trust_policy::sign(&invalid).is_err());
        invalid = args.clone();
        let fixture = Zeroizing::new(development_fixture_hex(include_str!("../dev-key.hex")));
        invalid[2] = fixture.to_string();
        let error = trust_policy::sign(&invalid).unwrap_err();
        assert!(!error.contains(fixture.as_str()));
        assert!(!std::path::Path::new(&args[0]).exists());
        fs::set_permissions(&args[2], fs::Permissions::from_mode(0o644)).unwrap();
        assert!(trust_policy::sign(&args).unwrap_err().contains("permissions"));
        fs::set_permissions(&args[2], fs::Permissions::from_mode(0o600)).unwrap();
        let private = read_private_key(&args[2], 64).unwrap();
        let mut mismatched = private.clone();
        mismatched[32] ^= 1;
        write_new_hex_key(&directory.path("mismatched.hex"), &mismatched, true).unwrap();
        invalid = args.clone();
        invalid[2] = directory.path("mismatched.hex");
        assert!(trust_policy::sign(&invalid).unwrap_err().contains("halves"));
        let fixture_path = directory.path("fixture.hex");
        write_new_file(&fixture_path, include_str!("../dev-key.hex").as_bytes(), true).unwrap();
        invalid[2] = fixture_path;
        assert!(trust_policy::sign(&invalid).unwrap_err().contains("DevelopmentKey"));
        let mut candidate =
            charlotte_launch::trust::AdmissionTrust::decode(&fs::read(&args[1]).unwrap()).unwrap();
        candidate.artifact_key =
            read_hex_key(&public_path, 32, "public key").unwrap().try_into().unwrap();
        fs::write(&args[1], candidate.encode().unwrap()).unwrap();
        assert!(trust_policy::sign(&args).unwrap_err().contains("SharedBootstrapRole"));
        assert!(!std::path::Path::new(&args[0]).exists());
    }

    #[cfg(unix)]
    #[test]
    fn signed_policy_commands_require_explicit_state_and_bounded_regular_records() {
        use std::os::unix::fs::symlink;
        let directory = TestDirectory::new();
        let (args, public_path) = signed_policy_args(&directory);
        let accepted_digest = "a5".repeat(32);
        let zero_digest = "00".repeat(32);
        let modes: &[&[&str]] = &[
            &[],
            &["enroll"],
            &["enroll", "0"],
            &["enroll", "1", "extra"],
            &["installed", "7"],
            &["installed", "0", &accepted_digest],
            &["installed", "7", &zero_digest],
            &["installed", "7", "not-hex"],
            &["installed", "18446744073709551616", &accepted_digest],
            &["unknown", "1"],
        ];
        for mode in modes {
            let mut sign_args = args[..4].to_vec();
            sign_args.extend(mode.iter().map(|value| (*value).to_owned()));
            assert!(trust_policy::sign(&sign_args).is_err());
            let mut verify_args = vec![args[0].clone(), public_path.clone(), args[3].clone()];
            verify_args.extend(mode.iter().map(|value| (*value).to_owned()));
            assert!(trust_policy::verify(&verify_args).is_err());
            assert!(!std::path::Path::new(&args[0]).exists());
        }
        trust_policy::sign(&args).unwrap();
        let original = fs::read(&args[0]).unwrap();
        let verify_args =
            vec![args[0].clone(), public_path, args[3].clone(), "enroll".into(), "7".into()];
        for malformed in
            [original[..327].to_vec(), [original.as_slice(), &[0]].concat(), vec![0; 4097]]
        {
            fs::write(&args[0], malformed).unwrap();
            assert!(trust_policy::verify(&verify_args).is_err());
        }
        fs::write(&args[0], &original).unwrap();
        let alias = directory.path("record-alias");
        symlink(&args[0], &alias).unwrap();
        let fifo = directory.path("record-fifo");
        let fifo_name = std::ffi::CString::new(fifo.as_str()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
        for source in [alias, fifo, directory.0.to_str().unwrap().to_owned()] {
            let mut input = verify_args.clone();
            input[0] = source;
            assert!(trust_policy::verify(&input).is_err());
        }
        let mut tampered = original;
        tampered[327] ^= 1;
        fs::write(&args[0], tampered).unwrap();
        assert!(trust_policy::verify(&verify_args).unwrap_err().contains("InvalidSignature"));
    }

    #[cfg(unix)]
    #[test]
    fn public_trust_policy_creation_is_exclusive_and_checks_exact_cluster_and_revision() {
        let directory = TestDirectory::new();
        let args = trust_policy_args(&directory);
        trust_policy::create(&args).unwrap();
        let bytes = fs::read(&args[0]).unwrap();
        assert_eq!(bytes.len(), charlotte_launch::trust::ENCODED_LEN);
        let decoded = charlotte_launch::trust::AdmissionTrust::decode(&bytes).unwrap();
        assert_eq!(decoded.sequence, 7);
        assert_eq!(decoded.artifact_key.as_slice(), read_hex_key(&args[3], 32, "public").unwrap());
        trust_policy::check(&[args[0].clone(), args[1].clone(), "7".into()]).unwrap();
        assert!(
            trust_policy::check(&[args[0].clone(), "another-cluster".into(), "1".into()]).is_err()
        );
        assert!(trust_policy::check(&[args[0].clone(), args[1].clone(), "8".into()]).is_err());
        assert!(trust_policy::check(&[args[0].clone(), args[1].clone(), "0".into()]).is_err());
        assert!(trust_policy::create(&args).is_err());
        assert_eq!(fs::read(&args[0]).unwrap(), bytes);
        assert!(trust_policy::create(&[]).is_err());
        assert!(trust_policy::check(&[]).is_err());
        for malformed in
            [bytes[..bytes.len() - 1].to_vec(), [bytes.as_slice(), &[0]].concat(), vec![0; 184]]
        {
            fs::write(&args[0], malformed).unwrap();
            assert!(trust_policy::check(&[args[0].clone(), args[1].clone(), "1".into()]).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn public_trust_policy_failures_publish_no_output() {
        let directory = TestDirectory::new();
        let args = trust_policy_args(&directory);
        let artifact = fs::read(&args[3]).unwrap();
        let cases = [
            hex_encode(&charlotte_launch::CLUSTER_PUBLIC_KEY),
            hex_encode(&charlotte_launch::DEVELOPMENT_OPERATIONS_PUBLIC_KEY),
            hex_encode(&charlotte_launch::DEVELOPMENT_RECIPIENT_PUBLIC_KEY),
            "00".repeat(32),
            "ff".repeat(32),
            "x".repeat(65),
            "0".repeat(4097),
            // Publicly known fixture tests private-length rejection without
            // creating a secret in an ordinary String.
            development_fixture_hex(include_str!("../dev-key.hex")),
        ];
        for key in cases {
            fs::write(&args[3], key).unwrap();
            assert!(trust_policy::create(&args).is_err());
            assert!(!std::path::Path::new(&args[0]).exists());
        }
        fs::write(&args[3], artifact).unwrap();
        let mut shared = args.clone();
        shared[4] = shared[3].clone();
        assert!(trust_policy::create(&shared).is_err());
        assert!(!std::path::Path::new(&args[0]).exists());
        let mut invalid = args.clone();
        invalid[1].clear();
        assert!(trust_policy::create(&invalid).is_err());
        for revision in ["0", "-1", "18446744073709551616", "invalid"] {
            invalid = args.clone();
            invalid[2] = revision.into();
            assert!(trust_policy::create(&invalid).is_err());
        }
        assert!(!std::path::Path::new(&args[0]).exists());
    }

    #[cfg(unix)]
    #[test]
    fn public_trust_policy_readers_reject_symlinks_directories_and_fifos() {
        use std::os::unix::fs::symlink;
        let directory = TestDirectory::new();
        let args = trust_policy_args(&directory);
        let alias = directory.path("alias");
        symlink(&args[3], &alias).unwrap();
        let fifo = directory.path("fifo");
        let fifo_name = std::ffi::CString::new(fifo.as_str()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
        for source in [alias, directory.0.to_str().unwrap().to_owned(), fifo] {
            let mut invalid = args.clone();
            invalid[3] = source.clone();
            assert!(trust_policy::create(&invalid).is_err());
            assert!(trust_policy::check(&[source, args[1].clone(), "1".into()]).is_err());
            assert!(!std::path::Path::new(&args[0]).exists());
        }
        // create_new also rejects a symlink output without modifying its target.
        symlink(&args[3], &args[0]).unwrap();
        let before = fs::read(&args[3]).unwrap();
        assert!(trust_policy::create(&args).is_err());
        assert_eq!(fs::read(&args[3]).unwrap(), before);
    }

    #[test]
    fn argv_only_accepts_the_public_legacy_artifact_fixture() {
        let pair = KeyPair::generate();
        let secret_hex = Zeroizing::new(hex_encode(pair.sk.as_ref()));
        let error = read_signing_key(&secret_hex).err().unwrap();
        assert!(error.contains("argv"));
        assert!(!error.contains(secret_hex.as_str()));
        let fixture = development_fixture_hex(include_str!("../dev-key.hex"));
        assert_eq!(
            read_signing_key(&fixture).unwrap().public_key().as_ref(),
            &charlotte_launch::CLUSTER_PUBLIC_KEY
        );
        assert!(read_signing_key(&development_fixture_hex(include_str!(
            "../dev-operations-key.hex"
        )))
        .is_err());
    }

    #[cfg(unix)]
    #[test]
    fn restricted_files_sign_and_never_overwrite_keys() {
        let directory = TestDirectory::new();
        let private = directory.path("signing.hex");
        let public = directory.path("signing.pub");
        signing_generate(&[private.clone(), public.clone()]).unwrap();
        assert_eq!(fs::metadata(&private).unwrap().mode() & 0o777, 0o600);
        let key = read_signing_key(&private).unwrap();
        assert_eq!(read_hex_key(&public, 32, "public key").unwrap(), key.public_key().as_ref());
        assert!(signing_generate(&[private.clone(), public.clone()]).is_err());
        assert_eq!(read_signing_key(&private).unwrap().as_ref(), key.as_ref());
        assert!(signing_generate(&[]).is_err());

        let new_private = directory.path("rollback.hex");
        assert!(signing_generate(&[new_private.clone(), public]).is_err());
        assert!(!std::path::Path::new(&new_private).exists());
    }

    #[cfg(unix)]
    #[test]
    fn private_file_checks_reject_unsafe_sources_and_bad_encoding() {
        use std::os::unix::fs::{
            symlink,
            PermissionsExt,
        };
        let directory = TestDirectory::new();
        let path = directory.path("key.hex");
        let pair = KeyPair::generate();
        write_new_hex_key(&path, pair.sk.as_ref(), true).unwrap();
        for mode in [0o644, 0o640, 0o602] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            assert!(read_private_key(&path, 64).err().unwrap().contains("mode"));
        }
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        assert_eq!(read_private_key(&path, 64).unwrap().as_slice(), pair.sk.as_ref());
        let alias = directory.path("alias.hex");
        symlink(&path, &alias).unwrap();
        assert!(read_private_key(&alias, 64).is_err());
        assert!(read_private_key(directory.0.to_str().unwrap(), 64).is_err());
        let fifo = directory.path("fifo");
        let fifo_name = std::ffi::CString::new(fifo.as_str()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
        assert!(read_private_key(&fifo, 64).is_err());

        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        for malformed in ["0".repeat(8192), "zz".repeat(64), "é".repeat(64), "00".to_owned()] {
            fs::write(&path, malformed).unwrap();
            assert!(read_private_key(&path, 64).is_err());
        }
        // Permissions are exempted by exact PUBLIC fixture contents, not name.
        fs::write(&path, include_str!("../dev-key.hex")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_signing_key(&path).is_ok());
        fs::write(&path, hex_encode(pair.sk.as_ref())).unwrap();
        assert!(read_signing_key(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn all_signed_control_envelopes_use_key_files() {
        let directory = TestDirectory::new();
        let private = directory.path("key.hex");
        let pair = KeyPair::generate();
        write_new_hex_key(&private, pair.sk.as_ref(), true).unwrap();
        let public: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
        let descriptor = directory.path("app.cdep");
        deployment_sign(&[
            descriptor.clone(),
            "app".into(),
            "apps/app.elf".into(),
            "a5".repeat(32),
            "0".into(),
            "1".into(),
            "4".into(),
            "1".into(),
            "5000".into(),
            private.clone(),
        ])
        .unwrap();
        let bytes = fs::read(&descriptor).unwrap();
        assert_eq!(deployment::verify(&bytes, public), deployment::VerifyOutcome::Valid);
        let release_path = directory.path("app.crelease");
        release_sign(&[
            release_path.clone(),
            "release".into(),
            "1".into(),
            private.clone(),
            descriptor,
        ])
        .unwrap();
        assert_eq!(
            release::verify(&fs::read(release_path).unwrap(), public),
            release::VerifyOutcome::Valid
        );
        let shutdown_path = directory.path("node.cshutdown");
        shutdown_sign(&[
            shutdown_path.clone(),
            "1".into(),
            "1".into(),
            "1".into(),
            "301".into(),
            "60000".into(),
            "5000".into(),
            private,
        ])
        .unwrap();
        assert_eq!(
            shutdown::verify(&fs::read(shutdown_path).unwrap(), public),
            shutdown::VerifyOutcome::Valid
        );
    }

    fn signed_descriptor(pair: &KeyPair, name: &[u8], sequence: u64) -> Vec<u8> {
        let fields = DescriptorFields {
            sequence,
            node_key: 0,
            artifact_digest: [sequence as u8; 32],
            artifact_name: name,
            stack_pages_per_thread: charlotte_launch::DEFAULT_USER_STACK_PAGES as u16,
            max_threads: charlotte_launch::DEFAULT_USER_MAX_THREADS as u16,
            shutdown_grace_ms: charlotte_launch::DEFAULT_SHUTDOWN_GRACE_MS,
            placement: charlotte_launch::placement::PlacementPolicy::singleton(),
            object_key: name,
            grants: &[],
        };
        let public_key: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
        let mut bytes = vec![0; deployment::encoded_len(&fields).unwrap()];
        deployment::encode_unsigned(&fields, public_key, &mut bytes).unwrap();
        let signature: Signature =
            pair.sk.sign(deployment::signature_digest(&bytes).unwrap(), None);
        let signature: &[u8; deployment::SIGNATURE_LEN] = signature.as_ref().try_into().unwrap();
        assert!(deployment::set_signature(&mut bytes, signature));
        bytes
    }

    #[test]
    fn percent_encodes_each_non_unreserved_byte_once() {
        assert_eq!(percent_encode_path_segment(b"orders/v2 ready"), "orders%2Fv2%20ready");
        assert_eq!(percent_encode_path_segment(&[0xff]), "%FF");
    }

    #[test]
    fn signed_release_binds_exact_distinct_component_set() {
        let pair = KeyPair::generate();
        let first = signed_descriptor(&pair, b"receive", 3);
        let second = signed_descriptor(&pair, b"publish", 7);
        let descriptors = [first.as_slice(), second.as_slice()];
        let fields = release::ReleaseFields {
            sequence: 11,
            release_name: b"orders-v11",
            descriptors: &descriptors,
        };
        let public_key: &[u8; 32] = pair.pk.as_ref().try_into().unwrap();
        let mut bytes = vec![0; release::encoded_len(&fields).unwrap()];
        release::encode_unsigned(&fields, public_key, &mut bytes).unwrap();
        let signature: Signature = pair.sk.sign(release::signature_digest(&bytes).unwrap(), None);
        let signature: &[u8; release::SIGNATURE_LEN] = signature.as_ref().try_into().unwrap();
        assert!(release::set_signature(&mut bytes, signature));
        assert_eq!(release::verify(&bytes, public_key), release::VerifyOutcome::Valid);
        let envelope = release::decode(&bytes).unwrap();
        assert_eq!(envelope.release_name, b"orders-v11");
        assert_eq!(envelope.descriptors().count(), 2);

        bytes[release::HEADER_LEN + fields.release_name.len() + 2] ^= 1;
        assert_eq!(release::verify(&bytes, public_key), release::VerifyOutcome::Invalid);

        let duplicates = [first.as_slice(), first.as_slice()];
        let duplicate_fields = release::ReleaseFields {
            descriptors: &duplicates,
            ..fields
        };
        assert_eq!(
            release::encoded_len(&duplicate_fields),
            Err(release::EncodeError::DuplicateArtifact)
        );
    }
}
