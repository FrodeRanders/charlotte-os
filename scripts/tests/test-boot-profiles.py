#!/usr/bin/env python3
"""Minimal boot is distinct from regression success, including cached artifacts."""

import importlib.util
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


boot = module("minimal_boot", "scripts/validate-minimal-boot.py")
profile = module("boot_profile", "scripts/check-boot-profile.py")
MEMORY = b"[boot validation] memory passed: usable=490 MiB, free=120000 frames, heap reserve=16 MiB\n"
SEED = b"[store] bootstrap seed complete: retained=0 written=25\n"
MINIMAL = b"CHARLOTTE_BOOT_MODE=minimal\n" + MEMORY + SEED + boot.COMPLETE + b"\n"


def elf(mode, symbol=b"_RNvCs123_6catten8bsp_main"):
    header = bytearray(64)
    header[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<Q", header, 40, 64)
    struct.pack_into("<HH", header, 58, 64, 3)
    marker = profile.MODES[mode] + b"\0"
    strings = b"\0" + symbol + b"\0"
    string_start = 64 + 3 * 64 + len(marker)
    symbols_start = string_start + len(strings)
    section = lambda kind, offset, size, link=0, entry=0: struct.pack(
        "<IIQQQQIIQQ", 0, kind, 0, 0, offset, size, link, 0, 1, entry)
    return (header + bytes(64) + section(3, string_start, len(strings))
            + section(2, symbols_start, 24, 1, 24) + marker + strings
            + struct.pack("<IBBHQQ", 1, 2, 0, 0, 0, 0))


class BootProfiles(unittest.TestCase):
    def test_minimal_requires_memory_and_a_complete_operational_boot_line(self):
        prefix = MINIMAL[:MINIMAL.index(boot.COMPLETE)]
        for length in range(len(boot.COMPLETE) + 1):
            self.assertFalse(boot.validate(prefix + boot.COMPLETE[:length]))
        self.assertTrue(boot.validate(MINIMAL))
        self.assertTrue(boot.validate(MINIMAL.replace(b"\n", b"\r\n")))
        self.assertFalse(boot.validate(MINIMAL.replace(MEMORY, b"")))
        self.assertFalse(boot.validate(MINIMAL.replace(SEED, b"")))
        self.assertFalse(boot.validate(MINIMAL.replace(b"free=120000", b"free=0")))
        self.assertFalse(boot.validate(MINIMAL.replace(b"=minimal", b"=tests")))

    def test_regression_verdict_and_panic_cannot_qualify_minimal_boot(self):
        self.assertFalse(boot.validate(MINIMAL + b"SELFTEST COMPLETE: passed=20\n"))
        self.assertFalse(boot.validate(MINIMAL + b"Kernel panic:\n"))
        self.assertFalse(boot.validate(b"SELFTEST COMPLETE: passed=20 failed=0 pending=0\n"))

    def test_kernel_mode_cannot_be_silently_reused(self):
        for mode in profile.MODES:
            profile.validate(elf(mode), mode)
            other = "test" if mode == "minimal" else "minimal"
            with self.assertRaisesRegex(ValueError, "does not match"):
                profile.validate(elf(mode), other)
        with self.assertRaises(ValueError):
            profile.validate(b"not an ELF " + profile.MODES["minimal"], "minimal")

    def test_native_fixture_symbols_and_markers_reject_minimal_artifacts(self):
        for name in (b"_RNvNtCs123_6catten9self_test3run", b"_RNv6catten15admission_tests3run"):
            with self.assertRaisesRegex(ValueError, "fixture"):
                profile.validate(elf("minimal", name), "minimal")
        with self.assertRaisesRegex(ValueError, "fixture"):
            profile.validate(elf("minimal") + b"SELFTEST COMPLETE: passed=", "minimal")

    def bundle(self, arch, mode, root, validate=False):
        code = ('source "$1/scripts/lib/service-bundle.sh"; '
                'catten_service_bundle "$2" "$3" "$4"; '
                + ('catten_validate_service_bundle "$3" "$2"' if validate
                   else 'printf "%s\\n" "$BUNDLE" "${SERVICES[@]}"'))
        return subprocess.run(["bash", "-c", code, "profile-test", str(ROOT), arch, mode, str(root)],
                              capture_output=True, text=True)

    def test_minimal_bundle_rejects_missing_services_and_stale_probes(self):
        for arch in ("aarch64", "x86_64"):
            with self.subTest(arch=arch), tempfile.TemporaryDirectory() as directory:
                result = self.bundle(arch, "minimal", directory)
                self.assertEqual(result.returncode, 0, result.stderr)
                bundle, *services = result.stdout.splitlines()
                self.assertNotIn("security_probe", services)
                self.assertNotIn("echo", services)
                self.assertNotIn("raft", services)
                self.assertNotIn("servicemgr", services)
                self.assertIn("ns", services)
                self.assertIn("objstore", services)
                self.assertIn("httpd", services)
                self.assertNotEqual(self.bundle(arch, "minimal", directory, True).returncode, 0)
                Path(bundle).mkdir(parents=True)
                for name in services:
                    Path(bundle, name + ".elf").touch()
                self.assertEqual(self.bundle(arch, "minimal", directory, True).returncode, 0)
                Path(bundle, "security_probe.elf").touch()
                self.assertNotEqual(self.bundle(arch, "minimal", directory, True).returncode, 0)
                test = self.bundle(arch, "test", directory).stdout.splitlines()
                self.assertNotEqual(bundle, test[0])
                self.assertIn("security_probe", test[1:])

    def test_minimal_rejects_guest_test_options_before_building(self):
        for runner in ("run-aarch64.sh", "run-x86_64.sh"):
            for flag in ("--security-test", "--http-test", "--reuse-storage"):
                result = subprocess.run([str(ROOT / "scripts" / runner), "--minimal", flag],
                                        capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("--minimal", result.stderr)


if __name__ == "__main__":
    unittest.main()
