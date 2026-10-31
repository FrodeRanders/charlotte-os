#!/usr/bin/env python3
"""Exercise retired tooling inputs before builds, storage or VM launch."""

import os
import json
import importlib.util
import shutil
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
RETIRED = ("CATTEN_CLUSTER_VIP", "CATTEN_CLUSTER_TCP_PORT", "CATTEN_CLUSTER_SERVICE_NAME")


class ToolingContracts(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="charlotte-tooling-")
        directory = Path(cls.temp.name)
        (directory / "boot-out").mkdir()
        channel = next(line.split('"')[1] for line in (ROOT / "rust-toolchain.toml").read_text().splitlines()
                       if line.startswith("channel ="))
        # The actual build script now uses the shared management grammar. Build
        # it as a host executable with Cargo so dependency and linker policies
        # remain coherent. This private harness never edits the workspace lock.
        manifest = directory / "Cargo.toml"
        manifest.write_text('[package]\nname="kernel-build-script-probe"\nversion="0.0.0"\nedition="2024"\n'
                            '[workspace]\n[dependencies]\ncharlotte-launch={path=' + json.dumps(str(ROOT / "crates/charlotte-launch")) + '}\n'
                            '[[bin]]\nname="kernel-build-script-probe"\npath=' + json.dumps(str(ROOT / "crates/catten/build.rs")) + '\n')
        shutil.copyfile(ROOT / "Cargo.lock", directory / "Cargo.lock")
        artifacts = subprocess.check_output(
            ["cargo", "+" + channel, "build", "--manifest-path", str(manifest),
             "--target-dir", str(directory / "target"), "--message-format=json"], cwd=directory, text=True)
        cls.build = next(Path(artifact["executable"]) for line in artifacts.splitlines()
                       if (artifact := json.loads(line)).get("reason") == "compiler-artifact"
                       and artifact["target"]["name"] == "kernel-build-script-probe")

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def environment(self):
        env = os.environ.copy()
        for name in (*RETIRED, "CLUSTER_SIGN_PRIVATE_KEY", "CARGO_FEATURE_BOOT_TRUST_TEST", "CARGO_FEATURE_DEPLOY_NET_TEST",
                     "CATTEN_MANAGEMENT_TLS_RESOURCE", "CATTEN_RESOURCE_POLICY", "CATTEN_BOOT_TEST_INSTALLED_POLICY",
                     "CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS"):
            env.pop(name, None)
        for name in ("CATTEN_OTLP_ENDPOINT", "CATTEN_OTLP_INTERVAL_MS", "CATTEN_OTLP_TIMEOUT_MS"):
            env.pop(name, None)
        env.update(CATTEN_TRUST_MODE="development", CARGO_CFG_TARGET_ARCH="aarch64",
                   CARGO_MANIFEST_DIR=str(ROOT / "crates/catten"),
                   OUT_DIR=str(Path(self.temp.name) / "boot-out"))
        return env

    def runner_rejects(self, args, diagnostic):
        result = subprocess.run(["bash", str(ROOT / "scripts/run-aarch64.sh"), *args],
                                env=self.environment(), capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(diagnostic, result.stderr)

    def test_retired_paired_flag_rejects(self):
        self.runner_rejects(["--cluster-service", "10.0.2.42:80", "--cluster-service-name", "orders"],
                            "Unknown argument: --cluster-service-name")

    def test_named_and_platform_assignments_reach_network_validation(self):
        self.runner_rejects(["--cluster-service", "10.0.2.42:80", "--cluster-service", "orders=10.0.2.43:8080",
                             "--cluster-ingress-test"], "requires --cluster-service and a shared L2 backend")

    def test_duplicate_endpoint_rejects(self):
        self.runner_rejects(["--cluster-service", "orders=10.0.2.42:80", "--cluster-service", "payments=10.0.2.42:80"],
                            "duplicate cluster service endpoint")

    def test_invalid_assignment_rejects(self):
        for assignment, diagnostic in [("bad name=10.0.2.42:80", "portable name characters"),
                                       ("=10.0.2.42:80", "portable name characters"),
                                       ("orders=256.0.2.42:80", "invalid IPv4 address"),
                                       ("orders=10.0.2.42:0", "must be an integer from 1 through 65535")]:
            with self.subTest(assignment=assignment):
                self.runner_rejects(["--cluster-service", assignment], diagnostic)

    def test_canonical_table_builds_on_both_architectures(self):
        for arch in ("aarch64", "x86_64"):
            env = self.environment()
            env.update(CARGO_CFG_TARGET_ARCH=arch, CATTEN_CLUSTER_SERVICES="orders=10.0.2.42:80;10.0.2.43:8080")
            result = subprocess.run([str(self.build)], env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"linker/{arch}.ld", result.stdout)

    def test_telemetry_archive_defaults_off_and_accepts_bounded_intervals(self):
        for value in (None, "0", "1000", "10000", "60000", "86400000"):
            env = self.environment()
            if value is not None:
                env["CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS"] = value
            result = subprocess.run([str(self.build)], env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn(f"cargo:rustc-env=CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS={value or '0'}", result.stdout)
            self.assertIn("cargo:rerun-if-env-changed=CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS", result.stdout)

    def test_invalid_telemetry_archive_policy_rejects_at_build(self):
        for value in ("", "on", "1", "999", "86400001", "-1000", "+1000", " 1000", "1000 ", "18446744073709551616"):
            env = self.environment()
            env["CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS"] = value
            result = subprocess.run([str(self.build)], env=env, capture_output=True, text=True, timeout=10)
            self.assertNotEqual(result.returncode, 0, value)
            self.assertIn("CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS", result.stderr)

    def test_outbound_otlp_profile_matches_cached_kernel_inspection(self):
        spec=importlib.util.spec_from_file_location("boot_profile",ROOT/"scripts/check-boot-profile.py")
        profile=importlib.util.module_from_spec(spec)
        spec.loader.exec_module(profile)
        for endpoint in (None,"http://10.0.2.2:14318/v1/metrics","http://127.0.0.1"):
            env=self.environment()
            if endpoint is not None:env["CATTEN_OTLP_ENDPOINT"]=endpoint
            result=subprocess.run([str(self.build)],env=env,capture_output=True,text=True,timeout=10)
            self.assertEqual(result.returncode,0,result.stderr)
            actual=(Path(env["OUT_DIR"])/"otlp-profile.bin").read_bytes()
            self.assertEqual(actual,profile.otlp_profile(env))
        for endpoint in ("","https://10.0.2.2:4318/v1/metrics","http://collector:4318/x","http://10.0.2.2/x\r\nInjected: value"):
            env=self.environment();env["CATTEN_OTLP_ENDPOINT"]=endpoint
            result=subprocess.run([str(self.build)],env=env,capture_output=True,text=True,timeout=10)
            self.assertNotEqual(result.returncode,0)

    def test_retired_build_variables_reject_without_values(self):
        for name in RETIRED:
            for value in ("", "retired-input-must-not-be-echoed"):
                with self.subTest(name=name, empty=not value):
                    env = self.environment()
                    env[name] = value
                    result = subprocess.run([str(self.build)], env=env, capture_output=True, text=True, timeout=10)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(f"{name} is retired", result.stderr)
                    self.assertNotIn("retired-input-must-not-be-echoed", result.stdout + result.stderr)
                    self.assertNotIn("cargo:rustc-link-arg", result.stdout)

    def test_installed_snapshot_requires_x86_fixture_and_development(self):
        for changes, diagnostic in (
            ({"CATTEN_BOOT_TEST_INSTALLED_POLICY": ""}, "requires the boot_trust_test fixture"),
            ({"CATTEN_BOOT_TEST_INSTALLED_POLICY": "", "CARGO_FEATURE_BOOT_TRUST_TEST": "1"}, "supports only the x86 QEMU fixture"),
            ({"CARGO_CFG_TARGET_ARCH": "x86_64", "CARGO_FEATURE_BOOT_TRUST_TEST": "1",
              "CATTEN_TRUST_MODE": "production"}, "production images are disabled"),
            ({"CARGO_CFG_TARGET_ARCH": "x86_64", "CARGO_FEATURE_BOOT_TRUST_TEST": "1",
              "CATTEN_BOOT_TEST_INSTALLED_POLICY": "relative.bin"}, "must be an absolute file path"),
        ):
            with self.subTest(changes=changes):
                env = self.environment()
                env.update(changes)
                result = subprocess.run([str(self.build)], env=env, capture_output=True, text=True, timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(diagnostic, result.stderr)
                self.assertNotIn("cargo:rustc-link-arg", result.stdout)

    def test_installed_snapshot_is_complete_bounded_and_staged_independently(self):
        source = ROOT / "crates/catten/src/service/admission"
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            env = self.environment()
            env.update(CARGO_CFG_TARGET_ARCH="x86_64", CARGO_FEATURE_BOOT_TRUST_TEST="1", OUT_DIR=str(directory))
            for name in (None, "test-policy-next.bin"):
                with self.subTest(snapshot=name or "default"):
                    original = source / (name or "test-policy.bin")
                    if name:
                        env["CATTEN_BOOT_TEST_INSTALLED_POLICY"] = str(original)
                    result = subprocess.run([str(self.build)], env=env, capture_output=True, text=True, timeout=10)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual((directory / "boot-test-installed-policy.bin").read_bytes(), original.read_bytes())
                    self.assertIn("CATTEN_BOOT_TEST_INSTALLED_POLICY_FILE=", result.stdout)
            candidate = directory / "malformed.bin"
            for data in (b"", (source / "test-policy.bin").read_bytes()[:-1],
                         (source / "test-policy.bin").read_bytes() + b"x", bytes(10000)):
                candidate.write_bytes(data)
                env["CATTEN_BOOT_TEST_INSTALLED_POLICY"] = str(candidate)
                result = subprocess.run([str(self.build)], env=env, capture_output=True, text=True, timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("malformed installed fixture snapshot", result.stderr)
                self.assertNotIn("CATTEN_BOOT_TEST_INSTALLED_POLICY_FILE=", result.stdout)


if __name__ == "__main__":
    unittest.main()
