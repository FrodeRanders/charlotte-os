//! Offline signing and inspection for CharlotteOS cluster artifacts.
//!
//! The tool deliberately uses `charlotte-launch`'s parser, metadata encoder,
//! SHA-256, and verifier.  Host tooling and the kernel therefore cannot drift
//! into subtly different interpretations of the signed byte stream.

use std::{
    env,
    fs,
    process::ExitCode,
};

use charlotte_launch::signature_note::{
    self,
    ArtifactClass,
    ArtifactMetadata,
    DESCRIPTOR_LEN,
    NOTE_NAME,
    NOTE_TYPE_SIGNATURE,
    SIGNATURE_LEN,
};
use ed25519_compact::{
    KeyPair,
    PublicKey,
    SecretKey,
    Signature,
};

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
            let decoded = signature_note::decode_metadata(&signature_note::encode_descriptor(metadata))
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
            println!("SHA-256, CLS2 metadata, and placement-policy self-tests pass");
            Ok(())
        }
        _ => Err("usage: cluster-sign generate | elf-sign <elf> <name> <privkey-hex> \
                  [service|driver|bootstrap|admin] [version] [rollback] [flags] \
                  [provenance-sha256|-] | elf-verify <elf> <name> <pubkey-hex> | sha256 <file> | \
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
