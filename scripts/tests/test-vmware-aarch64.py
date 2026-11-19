#!/usr/bin/env python3
"""Exercise appliance packaging and persistent-disk replacement safeguards."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
QEMU_IMG = shutil.which("qemu-img")


@unittest.skipUnless(QEMU_IMG, "VMware packaging requires qemu-img")
class VMwareAarch64(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="knytos-vmware-test-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / "scripts").mkdir()
        (self.root / "vmware").mkdir()
        self.builder = self.root / "scripts/build-vmware-aarch64.sh"
        shutil.copy2(ROOT / "scripts/build-vmware-aarch64.sh", self.builder)
        shutil.copy2(ROOT / "vmware/knytos-aarch64.vmx",
                     self.root / "vmware/knytos-aarch64.vmx")
        # Stub compilation only; use real qemu-img for VMDK creation/conversion.
        runner = self.root / "scripts/run-aarch64.sh"
        runner.write_text('''#!/usr/bin/env python3
import pathlib, sys
root = pathlib.Path(__file__).resolve().parents[1]
args = sys.argv[1:]
(root / "build-args").write_text("\\n".join(args))
instance = args[args.index("--instance") + 1]
if "--minimal" in args:
    instance += "-minimal"
images = root / "os-images"
images.mkdir(exist_ok=True)
(images / f"knytos-aarch64-{args[0]}-{instance}.img").write_bytes(
    b"ARM boot payload" + bytes(1024 * 1024))
clients = images / f"knytos-aarch64-{args[0]}-{instance}.img.management"
clients.mkdir(mode=0o700)
(clients / "reader.key").write_bytes(b"matching client key")
(clients / "reader.key").chmod(0o600)
''')
        runner.chmod(0o755)
        self.parent = self.root / "os-images/vmware"
        self.appliance = self.parent / "Knytos-AArch64.vmwarevm"

    def build(self, *args, env=None):
        return subprocess.run([str(self.builder), *args], env=env,
                              capture_output=True, text=True, timeout=30)

    def existing(self):
        self.appliance.mkdir(parents=True)
        disk = self.appliance / "knytos-data.vmdk"
        disk.write_bytes(b"installed persistent state")
        return disk

    def test_minimal_packages_arm_boot_and_blank_persistent_disk(self):
        result = self.build("debug", "--minimal", "--data-size-mib", "64")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        appliance = self.parent / "Knytos-AArch64-Minimal.vmwarevm"
        vmx = (appliance / "Knytos.vmx").read_text()
        self.assertIn('guestOS = "arm-other-64"', vmx)
        clients = appliance / "management"
        self.assertEqual(clients.stat().st_mode & 0o777, 0o700)
        self.assertEqual((clients / "reader.key").stat().st_mode & 0o777, 0o600)
        self.assertEqual((clients / "reader.key").read_bytes(), b"matching client key")
        args = (self.root / "build-args").read_text().splitlines()
        self.assertIn("--minimal", args)
        self.assertIn("--display", args)
        for kind in ("boot", "data"):
            raw = self.root / f"{kind}.raw"
            subprocess.run([QEMU_IMG, "convert", "-f", "vmdk", "-O", "raw",
                            str(appliance / f"knytos-{kind}.vmdk"), str(raw)],
                           check=True, capture_output=True, timeout=10)
            if kind == "data":
                self.assertEqual(raw.stat().st_size, 64 * 1024 * 1024)
                with raw.open("rb") as stream:
                    while chunk := stream.read(1024 * 1024):
                        self.assertFalse(any(chunk))
            else:
                self.assertTrue(raw.read_bytes().startswith(b"ARM boot payload"))

    def test_existing_appliance_rejected_before_build(self):
        disk = self.existing()
        result = self.build()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("already exists", result.stderr)
        self.assertEqual(disk.read_bytes(), b"installed persistent state")
        self.assertFalse((self.root / "build-args").exists())

    def test_locked_appliance_rejects_explicit_replacement(self):
        disk = self.existing()
        (self.appliance / "Knytos.vmx.lck").mkdir()
        result = self.build("--replace")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("appliance is locked", result.stderr)
        self.assertEqual(disk.read_bytes(), b"installed persistent state")

    def test_failed_conversion_preserves_existing_appliance(self):
        disk = self.existing()
        tools = self.root / "tools"
        tools.mkdir()
        converter = tools / "qemu-img"
        converter.write_text("#!/bin/sh\necho 'injected conversion failure' >&2\nexit 43\n")
        converter.chmod(0o755)
        env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"])
        result = self.build("--replace", env=env)
        self.assertEqual(result.returncode, 43, result.stderr)
        self.assertEqual(disk.read_bytes(), b"installed persistent state")
        self.assertFalse(list(self.parent.glob(".aarch64-appliance.*")))

    def test_replace_publishes_complete_appliance(self):
        self.existing()
        result = self.build("--replace", "--data-size-mib", "64")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("--tests", (self.root / "build-args").read_text().splitlines())
        for name in ("Knytos.vmx", "knytos-boot.vmdk", "knytos-data.vmdk"):
            self.assertTrue((self.appliance / name).is_file())
        self.assertFalse(list(self.parent.glob(".aarch64-appliance.*")))


if __name__ == "__main__":
    unittest.main()
