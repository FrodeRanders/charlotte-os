#!/usr/bin/env python3
"""Compile the production authentication-state guard on the deployed x86 target."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]
TLS = ROOT / "crates/catten-services/src/tls_client.rs"
TARGET = ROOT / "crates/catten-services/x86_64-unknown-none.json"


class TlsGcmBackend(unittest.TestCase):
    results = []

    @classmethod
    def setUpClass(cls):
        lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
        for package, version in (("polyval", "0.6.2"), ("ghash", "0.5.1"), ("zeroize", "1.9.0")):
            actual = [entry["version"] for entry in lock["package"] if entry["name"] == package]
            if actual != [version]:
                raise ValueError(f"Requalify the backend controls for {package}: {actual}")
        source = TLS.read_text()
        start = source.index('#[cfg(any(target_arch = "aarch64",')
        end = source.index("\n};", start) + len("\n};")
        cls.guard = "#![no_std]\n" + source[start:end] + "\n"
        cls.temporary = tempfile.TemporaryDirectory(prefix="charlotte-tls-gcm-")
        cls.directory = Path(cls.temporary.name)

    @classmethod
    def tearDownClass(cls):
        if output := os.environ.get("CATTEN_TLS_BACKEND_EVIDENCE"):
            Path(output).write_text(json.dumps({
                "target": str(TARGET.relative_to(ROOT)),
                "guard": cls.guard,
                "guard_sha256": hashlib.sha256(cls.guard.encode()).hexdigest(),
                "controls": cls.results,
            }, indent=2) + "\n")
        cls.temporary.cleanup()

    def compile(self, name, zeroize, override_flags=False):
        directory = type(self).directory / name
        (directory / "src").mkdir(parents=True)
        feature = ',features=["zeroize"]' if zeroize else ""
        manifest = ('[package]\nname="tls-gcm-backend-control"\nversion="0.0.0"\nedition="2021"\n'
                    '[workspace]\n[dependencies]\n'
                    f'polyval={{version="=0.6.2",default-features=false{feature}}}\n'
                    f'ghash={{version="=0.5.1",default-features=false{feature}}}\n'
                    'zeroize={version="=1.9.0",default-features=false}\n')
        (directory / "Cargo.toml").write_text(manifest)
        (directory / "src/lib.rs").write_text(type(self).guard)
        env = dict(os.environ)
        for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_TARGET_X86_64_UNKNOWN_NONE_RUSTFLAGS"):
            env.pop(key, None)
        if override_flags:
            # An explicit override shadows Cargo target configuration. The
            # production guard must reject this missing-backend-policy case.
            env["CARGO_ENCODED_RUSTFLAGS"] = "-D\x1fwarnings"
        command = ["cargo", "check", "--offline", "--lib", "--manifest-path", str(directory / "Cargo.toml"),
                   "--target", str(TARGET), "--target-dir", str(ROOT / "target/tls-gcm-backend-controls")]
        result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
        type(self).results.append({
            "name": name, "zeroize_features": zeroize, "overrides_target_flags": override_flags,
            "command": [part.replace(str(directory), "<isolated-control>") for part in command],
            "manifest": manifest, "exit_status": result.returncode,
            "diagnostics": [line for line in result.stderr.splitlines()
                            if "E0080" in line or "assertion failed" in line or "needs_drop" in line],
        })
        return result

    def test_erasing_software_owner_compiles_with_repository_target_flags(self):
        result = self.compile("software-erasing", True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_non_erasing_dispatcher_is_rejected(self):
        result = self.compile("dispatcher-rejected", True, override_flags=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("E0080", result.stderr)
        self.assertIn("needs_drop", result.stderr)

    def test_software_owner_without_erasure_feature_is_rejected(self):
        result = self.compile("software-erasure-disabled", False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("E0080", result.stderr)
        self.assertIn("needs_drop", result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
