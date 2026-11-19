#!/usr/bin/env python3
"""Check default development HTTPS enrollment and explicit startup controls."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
COMMON = ROOT / "scripts/lib/boot-common.sh"


class DefaultManagement(unittest.TestCase):
    def run_shell(self, source, **values):
        env = {key: value for key, value in os.environ.items()
               if not key.startswith("KNYTOS_MANAGEMENT_")}
        env.update(NETWORK="1", MANAGEMENT_STARTUP_TEST="")
        env.update(values)
        return subprocess.run(["bash", "-eu", "-c", 'source "$1"\n' + source,
                               "default-management", str(COMMON)],
                              env=env, capture_output=True, text=True, timeout=10)

    def test_ordinary_networked_boot_provisions_credentials(self):
        for mode in ("minimal", "test"):
            result = self.run_shell('''
knytos_boot_prepare_management_fixture() { echo provisioned; }
knytos_boot_prepare_management
''', BOOT_MODE=mode)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), "provisioned")

    def test_explicit_resource_needs_no_local_client_keys_to_enable_server(self):
        result = self.run_shell('''
knytos_boot_prepare_management_fixture() { echo replaced; exit 1; }
knytos_boot_prepare_management
test "$KNYTOS_MANAGEMENT_TLS_RESOURCE" = /selected/resource
''', KNYTOS_MANAGEMENT_TLS_RESOURCE="/selected/resource")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_startup_controls_preserve_missing_and_supplied_inputs(self):
        for expectation in ("disabled", "WrongPolicy", "Signature", "Authentication",
                            "ready", "validity", "chain-validity"):
            for resource in (None, "/selected/resource"):
                values = {"MANAGEMENT_STARTUP_TEST": expectation}
                if resource:
                    values["KNYTOS_MANAGEMENT_TLS_RESOURCE"] = resource
                result = self.run_shell('''
knytos_boot_prepare_management_fixture() { echo replaced; exit 1; }
knytos_boot_prepare_management
''', **values)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, "")

    def test_no_network_does_not_generate_an_unusable_identity(self):
        result = self.run_shell('''
knytos_boot_prepare_management_fixture() { exit 1; }
knytos_boot_prepare_management
''', NETWORK="0")
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_cached_kernel_cannot_silently_get_new_credentials(self):
        result = self.run_shell('knytos_boot_prepare_management\n', KNYTOS_SKIP_KERNEL_BUILD="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cannot provision a previously built kernel", result.stderr)

    def test_sidecar_keeps_exact_clients_and_restricts_permissions(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            inputs = directory / "identities"
            inputs.mkdir()
            names = ("ca.pem", "reader.pem", "reader.key", "administrator.pem",
                     "administrator.key", "resource.bin")
            for name in names:
                (inputs / name).write_text("identity:" + name)
            image = directory / "boot.img"
            result = self.run_shell('knytos_boot_export_management_clients "$IMAGE"\n',
                                    IMAGE=str(image), KNYTOS_BOOT_MANAGEMENT_CLIENT_DIR=str(inputs),
                                    HTTP_HOST_PORT="18080")
            self.assertEqual(result.returncode, 0, result.stderr)
            destination = directory / "boot.img.management"
            self.assertEqual(destination.stat().st_mode & 0o777, 0o700)
            for name in names:
                self.assertEqual((destination / name).read_bytes(), (inputs / name).read_bytes())
                self.assertEqual((destination / name).stat().st_mode & 0o777, 0o600)
            result = self.run_shell('knytos_boot_export_management_clients "$IMAGE"\n',
                                    IMAGE=str(image),
                                    KNYTOS_MANAGEMENT_TLS_RESOURCE=str(destination / "resource.bin"))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual((destination / "reader.key").read_text(), "identity:reader.key")
            # A later image without generated enrollment cannot keep stale keys.
            result = self.run_shell('knytos_boot_export_management_clients "$IMAGE"\n',
                                    IMAGE=str(image))
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(destination.exists())


if __name__ == "__main__":
    unittest.main()
