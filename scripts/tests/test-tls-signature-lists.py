#!/usr/bin/env python3
"""Compile actual TLS offer/selected-signature parsers, plus a rejecting old control."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

if not __debug__:
    raise SystemExit("TLS parser controls require enabled runtime checks")

ROOT = Path(__file__).resolve().parents[2]
TLS = ROOT / "crates/embedded-tls/src"
SOURCE = TLS / "extensions/extension_data/signature_algorithms.rs"
TESTS = ROOT / "scripts/tests/tls-signature-lists.rs"
PREAMBLE = r'''
#![allow(dead_code, unexpected_cfgs, unused_imports)]
#[path = "@TLS@/fmt.rs"] mod fmt;
#[derive(Debug)]
pub enum TlsError { InsufficientSpace, EncodeError, InvalidSignature, InvalidSignatureScheme, InvalidCertificateRequest, InvalidExtensionsLength, UnknownExtensionType, DecodeError, AbortHandshake(alert::AlertLevel, alert::AlertDescription), ParseError(parse_buffer::ParseError) }
#[path = "@TLS@/alert.rs"] mod alert;
#[path = "@TLS@/parse_buffer.rs"] mod parse_buffer;
#[path = "@TLS@/buffer.rs"] mod buffer;
mod extensions {
    @EXTENSION_TYPE@
    #[path = "@TLS@/extensions/extension_group_macro.rs"] mod extension_group_macro;
    pub mod extension_data {
    #[path = "@ALGORITHMS@"] pub mod signature_algorithms;
    #[path = "@TLS@/extensions/extension_data/signature_algorithms_cert.rs"] pub mod signature_algorithms_cert;
    #[path = "@TLS@/extensions/extension_data/unimplemented.rs"] pub mod unimplemented;
    }
    pub mod messages {
        use super::{extension_group_macro::extension_group, extension_data::{signature_algorithms::SignatureAlgorithms, unimplemented::Unimplemented}};
        @REQUEST_EXTENSIONS@
    }
}
mod handshake {
    pub use crate::buffer::CryptoBuffer;
    #[path = "@TLS@/handshake/certificate_verify.rs"] pub mod certificate_verify;
    #[path = "@TLS@/handshake/certificate_request.rs"] pub mod certificate_request;
}
'''


def run():
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    cargo = subprocess.check_output(["rustup", "which", "--toolchain", channel, "cargo"], text=True).strip()
    rustc = subprocess.check_output(["rustup", "which", "--toolchain", channel, "rustc"], text=True).strip()
    env = dict(os.environ, RUSTC=rustc)
    results = []
    # Import the real extension enum, macro and request parser, without pulling
    # unrelated crypto/transport modules into this parser-only host crate.
    extension_type = (TLS / "extensions/mod.rs").read_text()
    for declaration in ["mod extension_group_macro;", "pub mod extension_data;", "pub mod messages;"]:
        extension_type = extension_type.replace(declaration, "")
    messages = (TLS / "extensions/messages.rs").read_text()
    start = messages.index("extension_group! {\n    pub enum CertificateRequestExtension")
    end = messages.index("\n}\n", start) + 3
    preamble = PREAMBLE.replace("@EXTENSION_TYPE@", extension_type).replace("@REQUEST_EXTENSIONS@", messages[start:end])
    with tempfile.TemporaryDirectory(prefix="charlotte-tls-signatures-") as temporary:
        directory = Path(temporary)
        (directory / "src").mkdir()
        (directory / "Cargo.toml").write_text(
            '[package]\nname="tls-signature-controls"\nversion="0.0.0"\nedition="2024"\n'
            '[workspace]\n[dependencies]\nheapless={version="=0.9.3",default-features=false}\n')
        (directory / "Cargo.lock").write_bytes((ROOT / "Cargo.lock").read_bytes())
        for control in [False, True]:
            algorithms = SOURCE
            if control:
                source = SOURCE.read_text()
                start = source.index("        // RFC 8446 sections 3.5")
                end = source.index("\n    pub fn encode", start)
                source = source[:start] + '''        Ok(Self {
            supported_signature_algorithms: buf.read_list::<_, N>(data_length, SignatureScheme::parse)?,
        })
    }
''' + source[end:]
                algorithms = directory / "old-signature-algorithms.rs"
                algorithms.write_text(source)
            (directory / "src/lib.rs").write_text(
                preamble.replace("@TLS@", str(TLS)).replace("@ALGORITHMS@", str(algorithms))
                + TESTS.read_text())
            if not control:
                # Resolve/fetch the isolated graph before either offline control.
                # A fresh runner has no registry cache; setup failure is not
                # parser rejection. Cargo needs the source target to exist first.
                subprocess.run([cargo, "fetch", "--manifest-path", str(directory / "Cargo.toml")],
                               cwd=directory, env=env, check=True)
            command = [cargo, "test", "--lib", "--locked", "--offline", "--manifest-path", str(directory / "Cargo.toml"),
                       "--target-dir", str(ROOT / "target/tls-signature-list-controls")]
            result = subprocess.run(command, cwd=directory, env=env, capture_output=True, text=True)
            output = result.stdout + result.stderr
            if control:
                assert result.returncode != 0 and "captured_openssl4_offer_ignores_sm2_and_preserves_p256 ... FAILED" in output, output
            else:
                assert result.returncode == 0, output
            results.append({"control": "old-parser" if control else "production", "exit_status": result.returncode,
                            "receipts": [line for line in output.splitlines() if "test result:" in line or "captured_openssl4" in line]})
    evidence = {"parser_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(), "controls": results}
    if output := os.environ.get("CATTEN_TLS_SIGNATURE_EVIDENCE"):
        Path(output).write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps(evidence, indent=2))


if __name__ == "__main__":
    run()
