//! Offline signing and inspection for CharlotteOS cluster artifacts.
//!
//! The tool deliberately uses `charlotte-launch`'s parser, metadata encoder,
//! SHA-256, and verifier.  Host tooling and the kernel therefore cannot drift
//! into subtly different interpretations of the signed byte stream.

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
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
    operations,
    operations_bundle,
    release,
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
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| format!("invalid hex digit at byte {index}"))
        })
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn rust_array(bytes: &[u8]) -> String {
    let mut out = String::from("[\n");
    for chunk in bytes.chunks(16) {
        out.push_str("    ");
        for byte in chunk {
            out.push_str(&format!("0x{byte:02x}, "));
        }
        out.push('\n');
    }
    out.push(']');
    out
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

fn write_new_file(path: &str, bytes: &[u8], secret: bool) -> Result<()> {
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
    let operational_secret_bytes = Zeroizing::new(read_hex_key(
        args.get(8).ok_or_else(|| "missing operational signing-key path".to_owned())?,
        64,
        "operational Ed25519 secret key",
    )?);
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
    let recipient_private: [u8; 32] = read_hex_key(
        args.get(4).ok_or_else(|| "missing recipient private-key path".to_owned())?,
        32,
        "recipient private key",
    )?
    .try_into()
    .unwrap();
    let recipient_private = Zeroizing::new(recipient_private);
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
    let operational_secret_bytes = Zeroizing::new(read_hex_key(
        args.get(4).ok_or_else(|| "missing operational signing-key path".to_owned())?,
        64,
        "operational Ed25519 secret key",
    )?);
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
    let secret_hex = args.get(2).ok_or_else(|| "missing private key".to_owned())?;
    let class = parse_class(args.get(3))?;
    let version = parse_u64(args.get(4), "artifact version", 1)?;
    let rollback = parse_u64(args.get(5), "rollback counter", version)?;
    let flags = parse_u32(args.get(6), "artifact flags", 0)?;
    let provenance_digest = parse_digest(args.get(7))?;
    let secret = SecretKey::from_slice(&hex_decode(secret_hex)?)
        .map_err(|_| "private key must be an Ed25519 secret key".to_owned())?;
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
    let secret = SecretKey::from_slice(&hex_decode(
        args.get(9).ok_or_else(|| "missing private key".to_owned())?,
    )?)
    .map_err(|_| "private key must be an Ed25519 secret key".to_owned())?;
    let parsed_grants =
        args[10..].iter().map(|value| parse_grant(value)).collect::<Result<Vec<_>>>()?;
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
         shutdown_grace_ms={} grants={}",
        stack_pages_per_thread,
        max_threads,
        shutdown_grace_ms,
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
         max_threads={} shutdown_grace_ms={}",
        String::from_utf8_lossy(descriptor.artifact_name),
        String::from_utf8_lossy(descriptor.object_key),
        descriptor.node_key,
        descriptor.sequence,
        descriptor.stack_pages_per_thread,
        descriptor.max_threads,
        descriptor.shutdown_grace_ms
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
    let secret = SecretKey::from_slice(&hex_decode(
        args.get(3).ok_or_else(|| "missing private key".to_owned())?,
    )?)
    .map_err(|_| "private key must be an Ed25519 secret key".to_owned())?;
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
        Some("generate") => {
            let pair = KeyPair::generate();
            println!("public key (hex):  {}", hex_encode(pair.pk.as_ref()));
            println!("private key (hex): {}", hex_encode(pair.sk.as_ref()));
            println!(
                "\npub const CLUSTER_PUBLIC_KEY: [u8; 32] = {};",
                rust_array(pair.pk.as_ref())
            );
            println!(
                "\n// Keep this 64-byte secret outside the cluster:\npub const \
                 CLUSTER_PRIVATE_KEY: [u8; 64] = {};",
                rust_array(pair.sk.as_ref())
            );
            Ok(())
        }
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
                node_key: 0x1234,
                artifact_digest: [0xa5; 32],
                artifact_name: b"orders-step",
                stack_pages_per_thread: 16,
                max_threads: 8,
                shutdown_grace_ms: 5_000,
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
        _ => Err("usage: cluster-sign generate | elf-sign <elf> <name> <privkey-hex> \
                  [service|driver|bootstrap|admin] [version] [rollback] [flags] \
                  [provenance-sha256|-] | elf-verify <elf> <name> <pubkey-hex> | sha256 <file> | \
                  deployment-sign <output> <artifact-name> <object-key> <artifact-sha256> \
                  <node-key> <sequence> <stack-pages-per-thread> <max-threads> \
                  <shutdown-grace-ms> <privkey-hex> [service=send|call|client|publish ...] | \
                  deployment-verify <descriptor> <pubkey-hex> | deployment-notify <descriptor> \
                  [host:port] | deployment-status <artifact-name> [host:port] [wait-seconds] | \
                  deployment-apply <host:port> <wait-seconds> <descriptor>... | release-sign \
                  <output> <release-name> <sequence> <privkey-hex> <descriptor>... | \
                  release-verify <release> <pubkey-hex> | release-notify <release> [host:port] | \
                  release-apply <release> [host:port] [wait-seconds] | \
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
                  operations-bundle-notify <bundle> [host:port] | cluster-id <mnemonic> | \
                  selftest"
            .to_owned()),
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

    fn signed_descriptor(pair: &KeyPair, name: &[u8], sequence: u64) -> Vec<u8> {
        let fields = DescriptorFields {
            sequence,
            node_key: 0,
            artifact_digest: [sequence as u8; 32],
            artifact_name: name,
            stack_pages_per_thread: charlotte_launch::DEFAULT_USER_STACK_PAGES as u16,
            max_threads: charlotte_launch::DEFAULT_USER_MAX_THREADS as u16,
            shutdown_grace_ms: charlotte_launch::DEFAULT_SHUTDOWN_GRACE_MS,
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
