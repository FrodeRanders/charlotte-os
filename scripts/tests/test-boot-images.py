#!/usr/bin/env python3
"""Package payloads above the retired 64 MiB FAT boundary using real mtools."""

import hashlib
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
COMMON = ROOT / "scripts/lib/boot-common.sh"
MIB = 1024 * 1024


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


@unittest.skipUnless(all(shutil.which(tool) for tool in ("mformat", "mmd", "mcopy")),
                     "boot-image integration requires mtools")
class BootImages(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="charlotte-boot-image-")
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.kernel = self.directory / "kernel-payload"
        # Payload size alone exceeds the old image; sparse host preparation
        # still produces full bytes when mcopy packages/extracts the file.
        with self.kernel.open("wb") as stream:
            stream.write(b"begin kernel payload")
            stream.truncate(64 * MIB + 4096)
            stream.seek(-4, 2)
            stream.write(b"end!")
        self.image = self.directory / "boot.img"

    def create(self, size):
        return subprocess.run([
            "bash", "-c",
            'source "$1"; catten_boot_create_uefi_image "$2" '
            + size + ' "$3" "$4" "$5"',
            "boot-image-test", str(COMMON), str(self.image),
            str(ROOT / "limine-binary/BOOTX64.EFI"), str(self.kernel),
            str(ROOT / "limine.conf"),
        ], capture_output=True, text=True, timeout=30)

    def test_default_packages_large_payload_without_truncation(self):
        result = self.create('"$CATTEN_BOOT_DEFAULT_IMAGE_SIZE_MIB"')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for source, member in [(self.kernel, "::/catten"),
                               (ROOT / "limine-binary/BOOTX64.EFI", "::/EFI/BOOT/BOOTX64.EFI"),
                               (ROOT / "limine.conf", "::/limine.conf")]:
            extracted = self.directory / (source.name + ".extracted")
            subprocess.run(["mcopy", "-i", str(self.image), member, str(extracted)],
                           check=True, capture_output=True, timeout=30)
            self.assertEqual(digest(extracted), digest(source))
        self.assertFalse(list(self.directory.glob("boot.img.tmp.*")))

    def test_explicit_small_image_rejects_without_replacing_prior_output(self):
        self.image.write_bytes(b"previous image")
        previous = digest(self.image)
        result = self.create("64")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Disk full", result.stderr)
        self.assertIn("failed to create boot image", result.stderr)
        self.assertEqual(digest(self.image), previous)
        self.assertFalse(list(self.directory.glob("boot.img.tmp.*")))


if __name__ == "__main__":
    unittest.main()
