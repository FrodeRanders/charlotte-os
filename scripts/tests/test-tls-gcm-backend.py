#!/usr/bin/env python3
"""Compile production authentication-state guards on x86 service/kernel targets."""
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
MANAGEMENT_TLS = ROOT / "crates/charlotte-management-tls/src/crypto.rs"
KERNEL_TARGET = ROOT / "target_specs/x86_64-unknown-none-catten.json"


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
        source = MANAGEMENT_TLS.read_text()
        start = source.index('#[cfg(all(target_os = "none",')
        end = source.index("\n};", start) + len("\n};")
        cls.management_guard = "#![no_std]\n" + source[start:end] + "\n"
        cls.temporary = tempfile.TemporaryDirectory(prefix="charlotte-tls-gcm-")
        cls.directory = Path(cls.temporary.name)

    @classmethod
    def tearDownClass(cls):
        if output := os.environ.get("CATTEN_TLS_BACKEND_EVIDENCE"):
            Path(output).write_text(json.dumps({
                "target": str(TARGET.relative_to(ROOT)),
                "guard": cls.guard,
                "guard_sha256": hashlib.sha256(cls.guard.encode()).hexdigest(),
                "management_target": str(KERNEL_TARGET.relative_to(ROOT)),
                "management_guard": cls.management_guard,
                "management_guard_sha256": hashlib.sha256(cls.management_guard.encode()).hexdigest(),
                "controls": cls.results,
            }, indent=2) + "\n")
        cls.temporary.cleanup()

    def compile(self, name, zeroize, override_flags=False, management=False):
        directory = type(self).directory / name
        (directory / "src").mkdir(parents=True)
        target = KERNEL_TARGET if management else TARGET
        guard = type(self).management_guard if management else type(self).guard
        feature = ',features=["zeroize"]' if zeroize else ""
        manifest = ('[package]\nname="tls-gcm-backend-control"\nversion="0.0.0"\nedition="2021"\n'
                    '[workspace]\n[dependencies]\n'
                    f'polyval={{version="=0.6.2",default-features=false{feature}}}\n'
                    f'ghash={{version="=0.5.1",default-features=false{feature}}}\n'
                    'zeroize={version="=1.9.0",default-features=false}\n'
                    '[lints.rust]\n'
                    'unexpected_cfgs={level="warn",check-cfg=["cfg(polyval_force_soft)"]}\n')
        (directory / "Cargo.toml").write_text(manifest)
        (directory / "src/lib.rs").write_text(guard)
        env = dict(os.environ)
        for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_TARGET_X86_64_UNKNOWN_NONE_RUSTFLAGS",
                    "CARGO_TARGET_X86_64_UNKNOWN_NONE_CATTEN_RUSTFLAGS"):
            env.pop(key, None)
        if override_flags:
            # An explicit override shadows Cargo target configuration. The
            # production guard must reject this missing-backend-policy case.
            env["CARGO_ENCODED_RUSTFLAGS"] = "-D\x1fwarnings"
        # Fresh CI runners have no registry cache. Resolve/fetch this isolated
        # graph before the offline controls; setup failure is never evidence
        # that a production destructor guard rejected the selected backend.
        # Fetch all platforms: build-std can need registry metadata/sources
        # beyond dependencies selected by the freestanding target alone.
        subprocess.run(["cargo", "fetch", "--manifest-path", str(directory / "Cargo.toml")],
                       cwd=ROOT, env=env, check=True)
        command = ["cargo", "check", "--locked", "--offline", "--lib", "--manifest-path", str(directory / "Cargo.toml"),
                   "--target", str(target), "--target-dir", str(ROOT / "target/tls-gcm-backend-controls")]
        result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
        type(self).results.append({
            "name": name, "zeroize_features": zeroize, "overrides_target_flags": override_flags,
            "target": str(target.relative_to(ROOT)),
            "guard_sha256": hashlib.sha256(guard.encode()).hexdigest(),
            "command": [part.replace(str(directory), "<isolated-control>") for part in command],
            "manifest": manifest, "exit_status": result.returncode,
            "diagnostics": [line for line in result.stderr.splitlines()
                            if "E0080" in line or "assertion failed" in line or "needs_drop" in line
                            or "requires the erasing software" in line],
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

    def test_management_kernel_software_owner_compiles(self):
        result = self.compile("management-software-erasing", True, management=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_management_kernel_dispatcher_is_rejected(self):
        result = self.compile("management-dispatcher-rejected", True, override_flags=True, management=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("management TLS requires the erasing software POLYVAL backend", result.stderr)
        self.assertIn("E0080", result.stderr)

    def test_management_kernel_owner_without_erasure_is_rejected(self):
        result = self.compile("management-erasure-disabled", False, management=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("E0080", result.stderr)
        self.assertIn("needs_drop", result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
