#!/usr/bin/env python3
"""Compile and exercise the actual kernel policy and its trusted file selector."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
POLICY = ROOT / "crates/catten/src/resource_policy.rs"
DEFAULT = POLICY.parent / "resource_policy/defaults.rs"
LARGER = POLICY.parent / "resource_policy/larger.rs"


class ResourcePolicy(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="charlotte-resource-policy-")
        cls.directory = Path(cls.temporary.name)
        cls.harness = cls.directory / "harness.rs"
        cls.harness.write_text(f'#[path = {json.dumps(str(POLICY), ensure_ascii=False)}]\nmod resource_policy;\n')
        cls.selector_binary = cls.directory / "select-policy"
        subprocess.run(["rustc", "--edition", "2024", str(ROOT / "crates/catten/build.rs"),
                        "-o", str(cls.selector_binary)], cwd=ROOT, check=True, capture_output=True, text=True)

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def compile(self, profile):
        env = dict(os.environ, CATTEN_RESOURCE_POLICY_FILE=str(profile))
        return subprocess.run(["rustc", "--edition", "2024", "--test", str(self.harness),
                               "-D", "warnings", "-o", str(self.directory / "tests")],
                              cwd=ROOT, env=env, capture_output=True, text=True)

    def test_default_and_larger_profiles_execute_actual_policy_and_counter_tests(self):
        profiles = [DEFAULT, LARGER]
        if selected := os.environ.get("CATTEN_RESOURCE_POLICY"):
            selection = self.selector(selected)
            self.assertEqual(selection.returncode, 0, selection.stderr)
            profiles.append(Path(selected).resolve())
        for profile in profiles:
            with self.subTest(profile=profile.name):
                compiled = self.compile(profile)
                self.assertEqual(compiled.returncode, 0, compiled.stderr)
                result = subprocess.run([str(self.directory / "tests")], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertIn("13 passed; 0 failed", result.stdout)

    def test_invalid_selected_profiles_fail_during_compilation(self):
        cases = {
            "no-reserve": "p.mapping_nodes.ordinary = p.mapping_nodes.total;",
            "domain": "p.capabilities.domain[0] = p.capabilities.pool.total[0];",
            "machine-range": "p.mapping_nodes.total[0] = u64::MAX;",
            "ratio-zero": "p.memory.object_ram.denominator = 0;",
            "ratio-overcommitted": "p.memory.ordinary_share.numerator = 4;",
            "snapshot": "p.snapshot_buffers.ordinary[0] = 1;",
            "principal": "p.memory.principal_thread_metadata[0] = u64::MAX;",
            "queue": "p.local.endpoint_capacity = usize::MAX;",
            "heap-alignment": "p.kernel_heap.initial_bytes = 1;",
            "heap-overflow": "p.kernel_heap.growth_max_bytes = usize::MAX;",
            "unknown-field": "p.unknown_limit = 1;",
        }
        for name, mutation in cases.items():
            with self.subTest(case=name):
                profile = self.directory / f"{name}.rs"
                profile.write_text("use super::{DEFAULT, Policy};\n"
                                   "pub(super) const POLICY: Policy = { let mut p = DEFAULT; "
                                   + mutation + " p };\n")
                compiled = self.compile(profile)
                self.assertNotEqual(compiled.returncode, 0)
                if name != "unknown-field":
                    self.assertIn("invalid selected resource policy", compiled.stderr)
                else:
                    self.assertIn("unknown_limit", compiled.stderr)

    def selector(self, path=None):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith("CARGO_FEATURE_") and k != "CATTEN_RESOURCE_POLICY"}
        env.update(CARGO_MANIFEST_DIR=str(ROOT / "crates/catten"), CARGO_CFG_TARGET_ARCH="x86_64")
        if path is not None:
            env["CATTEN_RESOURCE_POLICY"] = str(path)
        return subprocess.run([str(type(self).selector_binary)], cwd=ROOT, env=env, capture_output=True, text=True)

    def test_selector_defaults_tracks_absolute_override_and_rejects_bad_paths(self):
        for value, expected in [(None, DEFAULT), (LARGER, LARGER)]:
            result = self.selector(value)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"cargo:rerun-if-changed={expected}", result.stdout)
            self.assertIn(f"cargo:rustc-env=CATTEN_RESOURCE_POLICY_FILE={expected}", result.stdout)
            self.assertIn("cargo:rerun-if-env-changed=CATTEN_RESOURCE_POLICY", result.stdout)
        for path in ["", "relative.rs", self.directory, self.directory / "missing.rs"]:
            with self.subTest(path=str(path)):
                self.assertNotEqual(self.selector(path).returncode, 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
