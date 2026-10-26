#!/usr/bin/env python3
"""Compile the real RNG owner with two isolated unsafe regression mutations.

The standalone host harness supplies a public one-page protocol bound. Cargo
and native controls separately compile the same owner with the actual protocol.
No mutated source is installed in a service or retained as a fallback path.
"""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]
RESULTS = []


def digest(data):
    return hashlib.sha256(data).hexdigest()


class RngDeviceFence(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="charlotte-rng-fence-")
        cls.directory = Path(cls.temporary.name)
        cls.source = (ROOT / "crates/catten-services/src/rng_device.rs").read_text()
        cls.contract = (ROOT / "crates/catten-services/src/rng_device/contract.rs").read_text()
        cls.toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def control(self, name, mutation=None, test_filter=None):
        source = self.source
        if mutation:
            before, after = mutation
            self.assertEqual(source.count(before), 1, "mutation must target exactly one production guard")
            source = source.replace(before, after)
        folder = self.directory / name
        folder.mkdir()
        (folder / "rng_device.rs").write_text(source)
        (folder / "contract.rs").write_text(self.contract)
        harness = '''mod entropy { pub const MAX_REQUEST: usize = 4096; }
#[path="rng_device.rs"] mod rng_device;
#[path="contract.rs"] mod rng_device_contract;
'''
        (folder / "harness.rs").write_text(harness)
        executable = folder / "control"
        command = ["rustc", "+" + self.toolchain, "--edition=2024", "--test", "-O",
                   str(folder / "harness.rs"), "-o", str(executable)]
        compiled = subprocess.run(command, text=True, capture_output=True, check=False)
        self.assertEqual(compiled.returncode, 0, compiled.stdout + compiled.stderr)
        executed = subprocess.run([str(executable)] + ([test_filter] if test_filter else []),
                                  text=True, capture_output=True, check=False)
        diagnostic = executed.stdout + executed.stderr
        RESULTS.append({
            "name": name,
            "original_source_sha256": digest(self.source.encode()),
            "compiled_source_sha256": digest(source.encode()),
            "contract_sha256": digest(self.contract.encode()),
            "harness": harness,
            "mutation": {"from": mutation[0], "to": mutation[1]} if mutation else None,
            "compiler_command": [s.replace(str(self.directory), "<isolated-control>") for s in command],
            "compiler_exit_code": compiled.returncode,
            "test_filter": test_filter,
            "executable_sha256": digest(executable.read_bytes()),
            "exit_code": executed.returncode,
            "diagnostics": diagnostic.replace(str(self.directory), "<isolated-control>").splitlines(),
        })
        return executed.returncode, diagnostic

    def test_unchanged_owner_passes(self):
        status, diagnostic = self.control("unchanged")
        self.assertEqual(status, 0, diagnostic)
        self.assertIn("8 passed; 0 failed", diagnostic)

    def test_replay_after_retention_is_rejected(self):
        status, diagnostic = self.control("replay-mutant", (
            "State::Retained(reason) => return Err(Error::Retained(reason)),",
            "State::Retained(_) => 0,"),
            "rng_device_contract::timeout_and_late_completion_never_restart")
        self.assertEqual(status, 101, diagnostic)
        self.assertIn("0 passed; 1 failed", diagnostic)

    def test_retained_backend_destruction_is_rejected(self):
        status, diagnostic = self.control("destructor-mutant", (
            "if matches!(self.state, State::Idle(_)) {", "if true {"),
            "rng_device_contract::backend_failures_retain_every_field")
        self.assertEqual(status, 101, diagnostic)
        self.assertIn("0 passed; 1 failed", diagnostic)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json-output", type=Path)
    options, remaining = parser.parse_known_args()
    program = unittest.main(argv=[__file__] + remaining, exit=False)
    if options.json_output:
        options.json_output.write_text(json.dumps(RESULTS, indent=2) + "\n")
    raise SystemExit(not program.result.wasSuccessful())
