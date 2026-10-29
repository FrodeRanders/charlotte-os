#!/usr/bin/env python3
"""Check production port selection against Python HMAC and reject the old owner."""
import hashlib
import hmac
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "crates/catten-services/src/network_random.rs"


class NetworkRandom(unittest.TestCase):
    results = []

    @classmethod
    def setUpClass(cls):
        packages = tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]
        # Legacy HMAC/SHA versions are intentional negative-control inputs in
        # an isolated manifest; they need not remain in the production graph.
        # Only the current control's zeroize pin must match the workspace lock.
        for name, version in (("zeroize", "1.9.0"),):
            if not any(p["name"] == name and p["version"] == version for p in packages):
                raise ValueError(f"Requalify the network-random control for {name} {version}")
        cls.temporary = tempfile.TemporaryDirectory(prefix="charlotte-network-random-")
        cls.directory = Path(cls.temporary.name)

    @classmethod
    def tearDownClass(cls):
        if output := os.environ.get("CATTEN_NETWORK_RANDOM_EVIDENCE"):
            Path(output).write_text(json.dumps({
                "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
                "controls": cls.results,
            }, indent=2) + "\n")
        cls.temporary.cleanup()

    def execute(self, name, source, legacy=False):
        directory = type(self).directory / name
        (directory / "src").mkdir(parents=True)
        manifest = ('[package]\nname="network-random-control"\nversion="0.0.0"\nedition="2024"\n'
                    '[workspace]\n[dependencies]\n'
                    f'charlotte-launch={{path={json.dumps(str(ROOT / "crates/charlotte-launch"))}}}\n'
                    'zeroize={version="=1.9.0",default-features=false}\n')
        if legacy:
            manifest += ('hmac={version="=0.12.1",default-features=false}\n'
                         'sha2={version="=0.10.9",default-features=false}\n')
        (directory / "Cargo.toml").write_text(manifest)
        (directory / "src/main.rs").write_text(source)
        # The retired HMAC version is deliberately outside the workspace graph.
        # Fetch each isolated graph before evaluating its offline control, even
        # when the production dependencies have already been cached.
        subprocess.run(["cargo", "fetch", "--manifest-path", str(directory / "Cargo.toml")],
                       cwd=directory, check=True)
        command = ["cargo", "check" if legacy else "run", "--locked", "--offline", "--quiet", "--release",
                   "--target-dir", str(ROOT / "target/network-random-controls")]
        result = subprocess.run(command, cwd=directory, capture_output=True, text=True)
        type(self).results.append({
            "name": name, "command": command, "manifest": manifest, "source": source,
            "exit_status": result.returncode,
            "stdout_sha256": hashlib.sha256(result.stdout.encode()).hexdigest(),
            "diagnostics": [line for line in result.stderr.splitlines()
                            if "E0080" in line or "legacy HMAC lacks Drop" in line],
        })
        return result

    def test_production_sequence_matches_independent_python_hmac(self):
        source = f'''#[path={json.dumps(str(SOURCE))}]
mod network_random;
const _: () = assert!(core::mem::needs_drop::<charlotte_launch::sha256::Sha256>());
fn main() {{
    let mut random = network_random::NetworkRandom::initialize(|bytes| {{
        for (index, byte) in bytes.iter_mut().enumerate() {{ *byte = index as u8; }}
        Ok::<_, ()>(())
    }}).unwrap();
    assert_eq!(random.interface_seed(), u64::from_le_bytes([0,1,2,3,4,5,6,7]));
    for _ in 0..1024 {{ println!("{{}}", random.ephemeral_port()); }}
}}
'''
        result = self.execute("production", source)
        self.assertEqual(result.returncode, 0, result.stderr)
        expected = []
        for counter in range(1024):
            digest = hmac.new(bytes(range(8, 40)),
                              b"CharlotteOS TCP/IP ephemeral port v1" + counter.to_bytes(8, "little"),
                              hashlib.sha256).digest()
            expected.append(49152 + (int.from_bytes(digest[:2], "little") & 0x3fff))
        self.assertEqual(list(map(int, result.stdout.splitlines())), expected)
        type(self).results[-1]["python_hmac_vectors_matched"] = len(expected)

    def test_legacy_hmac_type_cannot_meet_destructor_presence_contract(self):
        source = '''const _: () = assert!(core::mem::needs_drop::<hmac::Hmac<sha2::Sha256>>(),
    "legacy HMAC lacks Drop");
fn main() {}
'''
        result = self.execute("legacy-rejected", source, legacy=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("E0080", result.stderr)
        self.assertIn("legacy HMAC lacks Drop", result.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
