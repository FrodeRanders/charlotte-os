#!/usr/bin/env python3
"""Run the fixture assembler's actual Python block with public byte fixtures."""
import hashlib
from pathlib import Path
import resource
import signal
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "prepare-local-deployment-fixture.sh"
BODY = SCRIPT.read_text().split("<<'PYTHON'\n", 1)[1]
assert BODY.endswith("PYTHON\n")
BODY = BODY.removesuffix("PYTHON\n")
MARKER = b"public previous bundle must survive"


class Publication(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="charlotte-fixture-publication-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.parts = self.root / "parts"
        self.parts.mkdir(mode=0o700)
        self.output = self.root / "fixture.bundle"
        self.staging = self.parts / "bundle.pending"
        self.assertEqual(self.root.stat().st_mode & 0o777, 0o700)
        records = []
        for sequence in [1, 2, 3]:
            for node in ["0x42e278ea", "0x42e27737"]:
                data = f"public descriptor {sequence} {node}".encode()
                (self.parts / f"{sequence}.{node}").write_bytes(data)
                records.append(len(data).to_bytes(4, "little") + data)
        self.expected = b"".join(records)

    def assemble(self, limit=None):
        def restrict_child():
            signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
            resource.setrlimit(resource.RLIMIT_FSIZE, (limit, limit))

        # The test parent is single-threaded; limits apply only to the child.
        return subprocess.run(
            [sys.executable, "-", str(self.output), str(self.parts)],
            input=BODY, text=True, capture_output=True, timeout=30,
            preexec_fn=restrict_child if limit is not None else None,
        )

    def succeeded(self, result):
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.output.read_bytes(), self.expected)
        self.assertFalse(self.staging.exists())

    def failed(self, result):
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "")

    def test_fresh_and_repeated_publication_preserve_bytes(self):
        for _ in range(2):
            self.succeeded(self.assemble())
        print(f"fixture-vector bytes={len(self.expected)} "
              f"sha256={hashlib.sha256(self.expected).hexdigest()}")

    def test_symlink_target_is_preserved(self):
        target = self.root / "target"
        target.write_bytes(MARKER)
        self.output.symlink_to(target)
        result = self.assemble()
        self.assertEqual(target.read_bytes(), MARKER)
        self.assertFalse(self.output.is_symlink())
        self.succeeded(result)

    def test_hardlink_target_is_preserved(self):
        target = self.root / "target"
        target.write_bytes(MARKER)
        self.output.hardlink_to(target)
        result = self.assemble()
        self.assertEqual(target.read_bytes(), MARKER)
        self.assertNotEqual(target.stat().st_ino, self.output.stat().st_ino)
        self.succeeded(result)

    def test_existing_reader_keeps_previous_bundle(self):
        self.output.write_bytes(MARKER)
        with self.output.open("rb") as previous:
            result = self.assemble()
            self.assertEqual(previous.read(), MARKER)
        self.succeeded(result)

    def test_partial_write_preserves_previous_bundle(self):
        self.output.write_bytes(MARKER)
        inode = self.output.stat().st_ino
        self.failed(self.assemble(limit=17))
        self.assertEqual(self.output.read_bytes(), MARKER)
        self.assertEqual(self.output.stat().st_ino, inode)
        self.assertEqual(self.staging.stat().st_size, 17)

    def test_partial_write_leaves_absent_output_absent(self):
        self.failed(self.assemble(limit=17))
        self.assertFalse(self.output.exists())
        self.assertEqual(self.staging.stat().st_size, 17)

    def test_occupied_staging_rejects_without_publication(self):
        self.output.write_bytes(MARKER)
        self.staging.write_bytes(b"public retained staging")
        self.failed(self.assemble())
        self.assertEqual(self.output.read_bytes(), MARKER)
        self.assertEqual(self.staging.read_bytes(), b"public retained staging")

    def test_replace_rejection_retains_complete_staging(self):
        self.output.mkdir()
        child = self.output / "previous"
        child.write_bytes(MARKER)
        self.failed(self.assemble())
        self.assertTrue(self.output.is_dir())
        self.assertEqual(child.read_bytes(), MARKER)
        self.assertEqual(self.staging.read_bytes(), self.expected)


if __name__ == "__main__":
    unittest.main()
