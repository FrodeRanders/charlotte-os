#!/usr/bin/env python3
"""The pinned signer is built once; build failure cannot execute a stale signer."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]


class SigningBatch(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="charlotte-sign-batch-")
        self.addCleanup(self.directory.cleanup)
        self.base = Path(self.directory.name)
        self.bundle = self.base / "bundle"
        self.bundle.mkdir()
        binaries = self.base / "bin"
        binaries.mkdir()
        self.key = self.base / "fixture-key.hex"
        self.key.write_text("00" * 32 + "\n")
        self.key.chmod(0o600)
        self.cargo_log = self.base / "cargo.jsonl"
        self.sign_log = self.base / "sign.jsonl"
        signer = binaries / "signer"
        signer.write_text('''#!/usr/bin/env python3
import json,os,sys
with open(os.environ['SIGN_LOG'],'a') as log:
 log.write(json.dumps({'args':sys.argv[1:],'cwd':os.getcwd()})+'\\n')
''')
        signer.chmod(0o755)
        cargo = binaries / "cargo"
        cargo.write_text('''#!/usr/bin/env python3
import json,os,sys
with open(os.environ['CARGO_LOG'],'a') as log:
 log.write(json.dumps({'args':sys.argv[1:],'cwd':os.getcwd()})+'\\n')
print(json.dumps({'reason':'compiler-artifact','target':{'name':'cluster-sign'},
                  'executable':os.environ['SIGNER']}))
raise SystemExit(int(os.environ.get('BUILD_EXIT','0')))
''')
        cargo.chmod(0o755)
        self.environment = dict(os.environ, PATH=str(binaries) + os.pathsep + os.environ['PATH'],
                                CLUSTER_SIGN_KEY_FILE=str(self.key), CATTEN_TRUST_MODE="development",
                                CARGO_LOG=str(self.cargo_log), SIGN_LOG=str(self.sign_log), SIGNER=str(signer))
        self.environment.pop("CLUSTER_SIGN_PRIVATE_KEY", None)

    def run_batch(self, names, build_exit=0):
        for name in names:
            (self.bundle / (name + ".elf")).write_bytes(b"unchanged ELF fixture")
        return subprocess.run([str(ROOT / "scripts/sign-service-elfs.sh"), str(self.bundle), *names],
                              env=dict(self.environment, BUILD_EXIT=str(build_exit)),
                              capture_output=True, text=True, timeout=10)

    def test_one_pinned_build_and_one_signer_process_per_selected_elf(self):
        result = self.run_batch(["ns", "echo"])
        self.assertEqual(result.returncode, 0, result.stderr)
        builds = [json.loads(line) for line in self.cargo_log.read_text().splitlines()]
        self.assertEqual(len(builds), 1)
        channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        self.assertEqual(builds[0]["args"][:2], ["+" + channel, "build"])
        self.assertIn("--locked", builds[0]["args"])
        self.assertEqual(Path(builds[0]["cwd"]), Path("/tmp").resolve())
        invocations = [json.loads(line) for line in self.sign_log.read_text().splitlines()]
        self.assertTrue(all(Path(row["cwd"]) == Path("/tmp").resolve() for row in invocations))
        signs = [row["args"] for row in invocations]
        self.assertEqual([row[:4] for row in signs], [
            ["elf-sign", str(self.bundle / "ns.elf"), "ns", str(self.key)],
            ["elf-sign", str(self.bundle / "echo.elf"), "echo", str(self.key)]])
        self.assertEqual([row[4] for row in signs], ["bootstrap", "service"])
        self.assertNotIn("00" * 32, self.sign_log.read_text())

    def test_failed_build_cannot_sign_even_after_emitting_an_executable(self):
        result = self.run_batch(["ns"], build_exit=9)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.sign_log.exists())
        self.assertEqual((self.bundle / "ns.elf").read_bytes(), b"unchanged ELF fixture")

    def test_empty_bundle_does_not_start_cargo(self):
        result = self.run_batch([])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.cargo_log.exists())
        self.assertFalse(self.sign_log.exists())


if __name__ == "__main__":
    unittest.main()
