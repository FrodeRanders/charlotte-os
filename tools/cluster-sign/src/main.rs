//! cluster-sign — the off-cluster signing tool for CharlotteOS cluster
//! artifacts.
//!
//! The cluster's *private* key never enters the cluster: it is held here, at
//! the "IT department". The matching public key is injected into the cluster
//! (embedded in the kernel build and/or committed to the replicated state by
//! the key ceremony) and used by cluster nodes to validate artifacts.
//!
//! Usage:
//!   cluster-sign generate
//!       Print a fresh keypair (public and private keys as hex and as Rust
//!       byte-array literals for embedding).
//!
//!   cluster-sign sign <payload> <private-key-hex>
//!       Sign a payload and print the artifact blob (signature followed by
//!       the payload) as a Rust byte-array literal for embedding in the
//!       service binaries or the kernel self-tests.

use std::env;

use ed25519_compact::{
    KeyPair,
    SecretKey,
    Signature,
};

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn rust_array(bytes: &[u8], indent: &str) -> String {
    let mut out = String::from("&[\n");
    for chunk in bytes.chunks(16) {
        out.push_str(indent);
        out.push_str("    ");
        for byte in chunk {
            out.push_str(&format!("0x{byte:02x}, "));
        }
        out.push('\n');
    }
    out.push_str(indent);
    out.push_str("]");
    out
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
            println!("pub const CLUSTER_PUBLIC_KEY: [u8; 32] = [");
            for chunk in public.as_ref().chunks(16) {
                let line = chunk
                    .iter()
                    .map(|byte| format!("0x{byte:02x}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("    {line},");
            }
            println!("];");
            println!();
            println!("// Keep privately at the IT department:");
            println!("pub const CLUSTER_PRIVATE_KEY: [u8; 32] = [");
            for chunk in secret.as_ref().chunks(16) {
                let line = chunk
                    .iter()
                    .map(|byte| format!("0x{byte:02x}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("    {line},");
            }
            println!("];");
        }
        Some("sign") => {
            let payload = args.get(2).expect("usage: cluster-sign sign <payload> <private-key-hex>");
            let secret_hex = args.get(3).expect("usage: cluster-sign sign <payload> <private-key-hex>");
            let secret_bytes: Vec<u8> = (0..secret_hex.len())
                .step_by(2)
                .map(|index| u8::from_str_radix(&secret_hex[index..index + 2], 16).expect("private key hex"))
                .collect();
            let secret = SecretKey::from_slice(&secret_bytes).expect("valid 32-byte private key");
            let key_pair = KeyPair::from(secret);
            let signature: Signature = key_pair.sign(payload.as_bytes());
            println!("payload: {}", payload);
            println!("public key (hex):  {}", hex_encode(&key_pair.pk.as_ref()));
            println!("signature (hex):   {}", hex_encode(&signature.as_ref()));
            println!();
            let mut artifact = Vec::with_capacity(64 + payload.len());
            artifact.extend_from_slice(&signature.as_ref());
            artifact.extend_from_slice(payload.as_bytes());
            println!("// Artifact blob (signature || payload) for embedding:");
            println!("{}", rust_array(&artifact, ""));
        }
        _ => {
            eprintln!("usage: cluster-sign generate | sign <payload> <private-key-hex>");
            std::process::exit(1);
        }
    }
}
