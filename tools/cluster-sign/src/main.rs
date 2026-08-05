//! cluster-sign — the off-cluster signing tool for CharlotteOS cluster
//! artifacts.
//!
//! The cluster's *private* key never enters the cluster: it is held here, at
//! the "IT department" (for development, the version-controlled keypair in
//! `dev-key.hex`; a live key can be substituted via the
//! `CLUSTER_SIGN_PRIVATE_KEY` build-script variable). The matching public
//! key is injected into the cluster (embedded in the kernel build and/or
//! committed to the replicated state by the key ceremony) and used by
//! cluster nodes to validate artifacts.
//!
//! ELF artifacts are signed *in place*: the tool adds a `SHT_NOTE` section
//! (`.note.charlotte-sig`) carrying the 64-byte Ed25519 signature. The
//! signature covers the SHA-256 of the whole file with the note's descriptor
//! bytes treated as zeros (a signature cannot cover itself), so the signed
//! bytes are deterministic. The kernel's EL0 loader and the deploy agent
//! verify the note before accepting an artifact.
//!
//! Usage:
//!   cluster-sign generate
//!       Print a fresh keypair (public and private keys as hex and as Rust
//!       byte-array literals for embedding).
//!
//!   cluster-sign elf-sign <elf> <private-key-hex>
//!       Embed (or update) the signature note in `elf` in place. Prints the
//!       artifact's SHA-256 for embedding as GREET_ARTIFACT_SHA256.
//!
//!   cluster-sign elf-verify <elf> <public-key-hex>
//!       Verify the signature note against the public key.
//!
//!   cluster-sign sha256 <file>
//!       Print the SHA-256 of a file as hex and as a Rust byte array.
//!
//!   cluster-sign selftest
//!       Run the SHA-256 self-test vectors.

use std::env;
use std::fs;

use ed25519_compact::{
    KeyPair,
    PublicKey,
    SecretKey,
    Signature,
};

// ---- SHA-256 (FIPS 180-4, shared with charlotte-launch/src/sha256.rs) ----

struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    total_len: u64,
}

const K: [u32; 64] = [
    0x428a_2f98, 0x7137_4491, 0xb5c0_fbcf, 0xe9b5_dba5, 0x3956_c25b, 0x59f1_11f1, 0x923f_82a4,
    0xab1c_5ed5, 0xd807_aa98, 0x1283_5b01, 0x2431_85be, 0x550c_7dc3, 0x72be_5d74, 0x80de_b1fe,
    0x9bdc_06a7, 0xc19b_f174, 0xe49b_69c1, 0xefbe_4786, 0x0fc1_9dc6, 0x240c_a1cc, 0x2de9_2c6f,
    0x4a74_84aa, 0x5cb0_a9dc, 0x76f9_88da, 0x983e_5152, 0xa831_c66d, 0xb003_27c8, 0xbf59_7fc7,
    0xc6e0_0bf3, 0xd5a7_9147, 0x06ca_6351, 0x1429_2967, 0x27b7_0a85, 0x2e1b_2138, 0x4d2c_6dfc,
    0x5338_0d13, 0x650a_7354, 0x766a_0abb, 0x81c2_c92e, 0x9272_2c85, 0xa2bf_e8a1, 0xa81a_664b,
    0xc24b_8b70, 0xc76c_51a3, 0xd192_e819, 0xd699_0624, 0xf40e_3585, 0x106a_a070, 0x19a4_c116,
    0x1e37_6c08, 0x2748_774c, 0x34b0_bcb5, 0x391c_0cb3, 0x4ed8_aa4a, 0x5b9c_ca4f, 0x682e_6ff3,
    0x748f_82ee, 0x78a5_636f, 0x84c8_7814, 0x8cc7_0208, 0x90be_fffa, 0xa450_6ceb, 0xbef9_a3f7,
    0xc671_78f2,
];

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09_e667, 0xbb67_ae85, 0x3c6e_f372, 0xa54f_f53a, 0x510e_527f, 0x9b05_688c,
                0x1f83_d9ab, 0x5be0_cd19,
            ],
            buffer: [0u8; 64],
            buffer_len: 0,
            total_len: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total_len += data.len() as u64;
        if self.buffer_len != 0 {
            let take = (64 - self.buffer_len).min(data.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&data[..take]);
            self.buffer_len += take;
            data = &data[take..];
            if self.buffer_len == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffer_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffer_len = data.len();
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buffer_len != 56 {
            self.update(&[0x00]);
        }
        self.update(&bit_len.to_be_bytes());
        let mut digest = [0u8; 32];
        for (index, word) in self.state.iter().enumerate() {
            digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        digest
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (index, word) in w.iter_mut().enumerate().take(16) {
            *word = u32::from_be_bytes(block[index * 4..index * 4 + 4].try_into().unwrap());
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7) ^ w[index - 15].rotate_right(18) ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17) ^ w[index - 2].rotate_right(19) ^ (w[index - 2] >> 10);
            w[index] = w[index - 16].wrapping_add(s0).wrapping_add(w[index - 7]).wrapping_add(s1);
        }
        let mut a = self.state[0];
        let mut b = self.state[1];
        let mut c = self.state[2];
        let mut d = self.state[3];
        let mut e = self.state[4];
        let mut f = self.state[5];
        let mut g = self.state[6];
        let mut h = self.state[7];
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[index]).wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize()
}

fn sha256_skipping(data: &[u8], skip_start: usize, skip_len: usize) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(&data[..skip_start.min(data.len())]);
    for _ in 0..skip_len {
        hasher.update(&[0x00]);
    }
    let after = skip_start.saturating_add(skip_len).min(data.len());
    hasher.update(&data[after..]);
    hasher.finalize()
}

// ---- ELF signature notes (shared with charlotte-launch/src/signature_note.rs) ----

const NOTE_NAME: &[u8] = b"charlotte";
const NOTE_SECTION_NAME: &[u8] = b".note.charlotte-sig";
const NOTE_TYPE_SIGNATURE: u32 = 0x434c_5331; // "CLS1": cluster signature, revision 1
const SIGNATURE_LEN: usize = 64;
const SHT_NOTE: u32 = 7;
const NOTE_TOTAL: usize = 12 + 12 + SIGNATURE_LEN; // header + name + desc

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn build_note_bytes() -> Vec<u8> {
    let mut note = Vec::with_capacity(NOTE_TOTAL);
    note.extend_from_slice(&((NOTE_NAME.len() + 1) as u32).to_le_bytes()); // namesz
    note.extend_from_slice(&(SIGNATURE_LEN as u32).to_le_bytes()); // descsz
    note.extend_from_slice(&NOTE_TYPE_SIGNATURE.to_le_bytes()); // type
    note.extend_from_slice(NOTE_NAME);
    note.push(0);
    while note.len() % 4 != 0 {
        note.push(0);
    }
    note.extend_from_slice(&[0u8; SIGNATURE_LEN]);
    note
}

/// The `(shoff, shentsize, shnum, shstrndx)` of an ELF64 image.
fn elf_section_table(image: &[u8]) -> Option<(usize, usize, usize, usize)> {
    if image.len() < 64 || &image[0..4] != b"\x7fELF" || image[4] != 2 || image[5] != 1 {
        return None;
    }
    let shoff = u64::from_le_bytes(image[0x28..0x30].try_into().ok()?) as usize;
    let shentsize = u16::from_le_bytes(image[0x3a..0x3c].try_into().ok()?) as usize;
    let shnum = u16::from_le_bytes(image[0x3c..0x3e].try_into().ok()?) as usize;
    let shstrndx = u16::from_le_bytes(image[0x3e..0x40].try_into().ok()?) as usize;
    if shentsize < 40 || shoff + shentsize * shnum > image.len() {
        return None;
    }
    Some((shoff, shentsize, shnum, shstrndx))
}

/// Locate the signature note's descriptor: `(file_offset, desc_len)`.
fn find_signature_desc(image: &[u8]) -> Option<(usize, usize)> {
    let table = elf_section_table(image)?;
    let (shoff, shentsize, shnum, _) = table;
    for index in 0..shnum {
        let header = shoff + index * shentsize;
        let sh_type = u32::from_le_bytes(image[header + 0x04..header + 0x08].try_into().ok()?);
        if sh_type != SHT_NOTE {
            continue;
        }
        let sh_offset = u64::from_le_bytes(image[header + 0x18..header + 0x20].try_into().ok()?) as usize;
        let sh_size = u64::from_le_bytes(image[header + 0x20..header + 0x28].try_into().ok()?) as usize;
        let note_end = sh_offset.checked_add(sh_size)?;
        if note_end > image.len() {
            continue;
        }
        let note = &image[sh_offset..note_end];
        if note.len() < 12 {
            continue;
        }
        let namesz = u32::from_le_bytes(note[0..4].try_into().ok()?) as usize;
        let descsz = u32::from_le_bytes(note[4..8].try_into().ok()?) as usize;
        let note_type = u32::from_le_bytes(note[8..12].try_into().ok()?);
        let desc_start = align4(12 + namesz);
        if note_type == NOTE_TYPE_SIGNATURE
            && namesz == NOTE_NAME.len() + 1
            && note[12..12 + namesz] == *b"charlotte\0"
            && descsz >= SIGNATURE_LEN
            && desc_start + SIGNATURE_LEN <= note.len()
        {
            return Some((sh_offset + desc_start, descsz));
        }
    }
    None
}

fn hex_decode(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex"))
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
    out.push_str("]");
    out
}

fn elf_sign(path: &str, private_key_hex: &str) {
    let mut image = fs::read(path).expect("read ELF");
    let secret = SecretKey::from_slice(&hex_decode(private_key_hex)).expect("private key");
    let public = secret.public_key();

    // Ensure the note section exists.
    if find_signature_desc(&image).is_none() {
        let (shoff, shentsize, shnum, shstrndx) =
            elf_section_table(&image).expect("ELF64 section table");
        let str_header = shoff + shstrndx * shentsize;
        let str_off = u64::from_le_bytes(image[str_header + 0x18..str_header + 0x20].try_into().unwrap()) as usize;
        let str_size = u64::from_le_bytes(image[str_header + 0x20..str_header + 0x28].try_into().unwrap()) as usize;
        let old_strtab = image[str_off..str_off + str_size].to_vec();

        let note = build_note_bytes();
        let note_offset = align4(image.len());
        let new_strtab_len = old_strtab.len() + NOTE_SECTION_NAME.len() + 1;
        let new_table_offset = note_offset + note.len() + new_strtab_len;
        let old_table = image[shoff..shoff + shentsize * shnum].to_vec();

        let mut new_image = image.clone();
        new_image.resize(note_offset, 0); // zero-fill the alignment pad
        new_image.extend_from_slice(&note);
        new_image.extend_from_slice(&old_strtab);
        new_image.extend_from_slice(NOTE_SECTION_NAME);
        new_image.push(0);
        // The old section header table moves to the end; a new entry follows.
        new_image.extend_from_slice(&old_table);
        let mut new_header = [0u8; 64];
        new_header[0x00..0x04].copy_from_slice(&(old_strtab.len() as u32).to_le_bytes()); // sh_name
        new_header[0x04..0x08].copy_from_slice(&SHT_NOTE.to_le_bytes()); // sh_type
        new_header[0x08..0x10].copy_from_slice(&0u64.to_le_bytes()); // sh_flags
        new_header[0x10..0x18].copy_from_slice(&0u64.to_le_bytes()); // sh_addr
        new_header[0x18..0x20].copy_from_slice(&(note_offset as u64).to_le_bytes()); // sh_offset
        new_header[0x20..0x28].copy_from_slice(&(note.len() as u64).to_le_bytes()); // sh_size
        new_header[0x28..0x2c].copy_from_slice(&0u32.to_le_bytes()); // sh_link
        new_header[0x2c..0x30].copy_from_slice(&0u32.to_le_bytes()); // sh_info
        new_header[0x30..0x38].copy_from_slice(&4u64.to_le_bytes()); // sh_addralign
        new_header[0x38..0x40].copy_from_slice(&0u64.to_le_bytes()); // sh_entsize
        new_image.extend_from_slice(&new_header);

        // Update the header: e_shoff (0x28), e_shnum (0x3c), and the old
        // shstrtab section's sh_size (0x20 within its header).
        new_image[0x28..0x30].copy_from_slice(&(new_table_offset as u64).to_le_bytes());
        new_image[0x3c..0x3e].copy_from_slice(&((shnum + 1) as u16).to_le_bytes());
        let strtab_header = new_table_offset + shstrndx * shentsize;
        let new_str_size = new_strtab_len as u64;
        new_image[strtab_header + 0x18..strtab_header + 0x20]
            .copy_from_slice(&((note_offset + note.len()) as u64).to_le_bytes());
        new_image[strtab_header + 0x20..strtab_header + 0x28]
            .copy_from_slice(&new_str_size.to_le_bytes());
        image = new_image;
    }

    // Sign: SHA-256 of the file with the note's descriptor bytes zeroed.
    let (desc_offset, desc_len) = find_signature_desc(&image).expect("note present");
    assert!(desc_len >= SIGNATURE_LEN, "signature note descriptor too small");
    let digest = sha256_skipping(&image, desc_offset, SIGNATURE_LEN);
    let signature: Signature = secret.sign(&digest, None);
    image[desc_offset..desc_offset + SIGNATURE_LEN].copy_from_slice(&signature.as_ref());
    fs::write(path, &image).expect("write ELF");

    println!("signed {path} (public key {})", hex_encode(&public.as_ref()));
    println!("artifact sha256: {}", hex_encode(&sha256(&image)));
    println!(
        "embed as GREET_ARTIFACT_SHA256: {}",
        rust_array(&sha256(&image)).replace('\n', " ")
    );
}

fn elf_verify(path: &str, public_key_hex: &str) {
    let image = fs::read(path).expect("read ELF");
    let public_key = PublicKey::from_slice(&hex_decode(public_key_hex)).expect("public key");
    match find_signature_desc(&image) {
        None => {
            println!("UNSIGNED: no cluster signature note");
            std::process::exit(2);
        }
        Some((desc_offset, desc_len)) => {
            if desc_len < SIGNATURE_LEN {
                println!("INVALID: signature note descriptor too small");
                std::process::exit(1);
            }
            let digest = sha256_skipping(&image, desc_offset, SIGNATURE_LEN);
            let signature = Signature::from_slice(&image[desc_offset..desc_offset + SIGNATURE_LEN])
                .expect("signature bytes");
            match public_key.verify(&digest, &signature) {
                Ok(()) => println!("VERIFY OK: {} carries a valid cluster signature", path),
                Err(_) => {
                    println!("INVALID: {} fails cluster signature verification", path);
                    std::process::exit(1);
                }
            }
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("generate") => {
            let key_pair = KeyPair::generate();
            let public = key_pair.pk;
            let secret = key_pair.sk;
            println!("public key (hex):  {}", hex_encode(&public.as_ref()));
            println!("private key (hex): {}", hex_encode(&secret.as_ref()));
            println!();
            println!("// Embed in charlotte-launch:");
            println!("pub const CLUSTER_PUBLIC_KEY: [u8; 32] = {}", rust_array(&public.as_ref()));
            println!();
            println!("// Keep privately at the IT department:");
            println!("pub const CLUSTER_PRIVATE_KEY: [u8; 32] = {}", rust_array(&secret.as_ref()));
        }
        Some("elf-sign") => {
            let path = args.get(2).expect("usage: cluster-sign elf-sign <elf> <private-key-hex>");
            let key = args.get(3).expect("usage: cluster-sign elf-sign <elf> <private-key-hex>");
            elf_sign(path, key);
        }
        Some("elf-verify") => {
            let path = args.get(2).expect("usage: cluster-sign elf-verify <elf> <public-key-hex>");
            let key = args.get(3).expect("usage: cluster-sign elf-verify <elf> <public-key-hex>");
            elf_verify(path, key);
        }
        Some("sha256") => {
            let path = args.get(2).expect("usage: cluster-sign sha256 <file>");
            let data = fs::read(path).expect("read file");
            println!("{}", hex_encode(&sha256(&data)));
            println!("rust array: {}", rust_array(&sha256(&data)).replace('\n', " "));
        }
        Some("selftest") => {
            let abc = sha256(b"abc");
            let empty = sha256(b"");
            assert_eq!(
                hex_encode(&abc),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
                "sha256(\"abc\")"
            );
            assert_eq!(
                hex_encode(&empty),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                "sha256(\"\")"
            );
            let one_region = sha256_skipping(b"0123456789abcdef0123456789abcdef", 0, 0);
            assert_eq!(one_region, sha256(b"0123456789abcdef0123456789abcdef"));
            println!("sha256 self-tests pass");
        }
        _ => {
            eprintln!(
                "usage: cluster-sign generate | elf-sign <elf> <privkey-hex> | \
                 elf-verify <elf> <pubkey-hex> | sha256 <file> | selftest"
            );
            std::process::exit(1);
        }
    }
}
