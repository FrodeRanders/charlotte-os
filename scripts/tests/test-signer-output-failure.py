#!/usr/bin/env python3
"""Exercise signer failures in disposable private directories; never log key bytes."""
import argparse
import os
from pathlib import Path
import resource
import signal
import subprocess
import tempfile
import unittest


GENERATORS = [
    ("generate", 129),
    ("operations-signing-generate", 129),
    ("operations-recipient-generate", 65),
]
ROOT = Path(__file__).resolve().parents[2]
SIGNER = ROOT / "target/debug/cluster-sign"


def run(*args, limit=None):
    def restrict_child():
        # The single-threaded test parent changes only the child. Ignore the
        # size-limit signal so write_all receives the actual file-size error.
        signal.signal(signal.SIGXFSZ, signal.SIG_IGN)
        resource.setrlimit(resource.RLIMIT_FSIZE, (limit, limit))

    return subprocess.run(
        [str(SIGNER), *map(str, args)], capture_output=True,
        preexec_fn=restrict_child if limit is not None else None,
        timeout=30,
    )


class OutputFailure(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="charlotte-output-failure-")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.assertEqual(self.root.stat().st_mode & 0o777, 0o700)

    def rejected(self, result):
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, b"")

    def retained(self, path, result, size, private=False):
        self.rejected(result)
        self.assertTrue(path.exists(), result.stderr.decode(errors="replace"))
        self.assertEqual(path.stat().st_size, size)
        if private:
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_partial_private_write_retains_output(self):
        for command, _ in GENERATORS:
            for limit in [0, 17]:
                with self.subTest(command=command, limit=limit):
                    private = self.root / f"{command}-{limit}.hex"
                    public = self.root / f"{command}-{limit}.pub"
                    result = run(command, private, public, limit=limit)
                    self.retained(private, result, limit, private=True)
                    self.assertFalse(public.exists())
                    self.assertIn(b"output may be partial", result.stderr)

    def test_partial_public_write_retains_output(self):
        key = self.root / "fixture.hex"
        key.write_bytes((ROOT / "tools/cluster-sign/dev-key.hex").read_bytes())
        key.chmod(0o600)
        for limit in [0, 17]:
            with self.subTest(limit=limit):
                output = self.root / f"shutdown-{limit}"
                result = run("shutdown-sign", output, 1, 1, 1, 301, 60000, 5000,
                             key, limit=limit)
                self.retained(output, result, limit)
                self.assertIn(b"output may be partial", result.stderr)

    def test_public_creation_failure_retains_private(self):
        for command, size in GENERATORS:
            with self.subTest(command=command):
                private = self.root / f"{command}.hex"
                public = self.root / f"{command}.pub"
                public.write_bytes(b"public existing output")
                result = run(command, private, public)
                self.retained(private, result, size, private=True)
                self.assertEqual(public.read_bytes(), b"public existing output")
                self.assertIn(b"key pair incomplete", result.stderr)
                self.assertIn(os.fsencode(private), result.stderr)

    def test_same_output_path_retains_private(self):
        for command, size in GENERATORS:
            with self.subTest(command=command):
                private = self.root / f"{command}.hex"
                result = run(command, private, private)
                self.retained(private, result, size, private=True)
                self.assertIn(b"key pair incomplete", result.stderr)

    def test_private_creation_rejection_preserves_outputs(self):
        for command, _ in GENERATORS:
            with self.subTest(command=command):
                private = self.root / f"{command}.hex"
                public = self.root / f"{command}.pub"
                private.write_bytes(b"public existing input marker")
                result = run(command, private, public)
                self.rejected(result)
                self.assertEqual(private.read_bytes(), b"public existing input marker")
                self.assertFalse(public.exists())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--signer", type=Path, default=SIGNER)
    options, remaining = parser.parse_known_args()
    SIGNER = options.signer.resolve()
    unittest.main(argv=[__file__, *remaining])
