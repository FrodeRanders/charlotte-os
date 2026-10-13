#!/usr/bin/env python3
"""Exercise retired tooling inputs before builds, storage or VM launch."""

import os
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
        cls.build = Path(cls.temp.name) / "kernel-build-script"
        channel = next(line.split('"')[1] for line in (ROOT / "rust-toolchain.toml").read_text().splitlines()
                       if line.startswith("channel ="))
        compiler = subprocess.check_output(["rustup", "which", "--toolchain", channel, "rustc"], text=True).strip()
        subprocess.run([compiler, "--edition=2024", str(ROOT / "crates/catten/build.rs"), "-o", str(cls.build)], check=True)

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def environment(self):
        env = os.environ.copy()
        for name in (*RETIRED, "CLUSTER_SIGN_PRIVATE_KEY", "CARGO_FEATURE_BOOT_TRUST_TEST", "CARGO_FEATURE_DEPLOY_NET_TEST"):
            env.pop(name, None)
        env.update(CATTEN_TRUST_MODE="development", CARGO_CFG_TARGET_ARCH="aarch64")
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


if __name__ == "__main__":
    unittest.main()
