#!/usr/bin/env python3
"""Compile the real userspace SSE2 ABI and reject SIMD on the kernel target."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = '''#![no_std]
#![deny(future_incompatible)]
#[unsafe(no_mangle)]
pub extern "C" fn double(value: f64) -> f64 { value + value }
#[target_feature(enable = "sse2")]
pub unsafe fn vectors() {}
'''


class X86UserspaceAbi(unittest.TestCase):
    def compile(self, target):
        with tempfile.TemporaryDirectory(prefix="charlotte-x86-abi-") as temporary:
            directory = Path(temporary)
            (directory / "src").mkdir()
            (directory / "src/lib.rs").write_text(SOURCE)
            (directory / "Cargo.toml").write_text(
                '[package]\nname="x86-abi-control"\nversion="0.0.0"\nedition="2024"\n'
                '[workspace]\n')
            env = os.environ.copy()
            for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
                        "CARGO_TARGET_X86_64_UNKNOWN_NONE_RUSTFLAGS",
                        "CARGO_TARGET_X86_64_UNKNOWN_NONE_CATTEN_RUSTFLAGS"):
                env.pop(key, None)
            return subprocess.run(
                ["cargo", "check", "--offline", "--lib", "--manifest-path",
                 str(directory / "Cargo.toml"), "--target", str(ROOT / target),
                 "--target-dir", str(ROOT / "target/x86-abi-controls")],
                cwd=ROOT, env=env, capture_output=True, text=True)

    def test_userspace_accepts_sse2_and_sysv_float_abi(self):
        result = self.compile("crates/catten-services/x86_64-unknown-none.json")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("future incompatib", result.stderr)

    def test_kernel_keeps_its_separate_no_simd_abi(self):
        result = self.compile("target_specs/x86_64-unknown-none-catten.json")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("x86_softfloat_sse", result.stderr)


if __name__ == "__main__":
    unittest.main()
