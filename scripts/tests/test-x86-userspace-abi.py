#!/usr/bin/env python3
"""Compile normal SysV/SSE2 code on the real userspace and kernel targets."""

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
    def compile(self, target, source=SOURCE, avx=False):
        with tempfile.TemporaryDirectory(prefix="knytos-x86-abi-") as temporary:
            directory = Path(temporary)
            (directory / "src").mkdir()
            (directory / "src/lib.rs").write_text(source)
            (directory / "Cargo.toml").write_text(
                '[package]\nname="x86-abi-control"\nversion="0.0.0"\nedition="2024"\n'
                '[workspace]\n[features]\nkernel_tests=[]\n')
            env = os.environ.copy()
            for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
                        "CARGO_TARGET_X86_64_UNKNOWN_NONE_RUSTFLAGS",
                        "CARGO_TARGET_X86_64_UNKNOWN_NONE_CATTEN_RUSTFLAGS"):
                env.pop(key, None)
            if avx:
                env["CARGO_ENCODED_RUSTFLAGS"] = "-D\x1fwarnings\x1f-C\x1ftarget-feature=+avx"
            return subprocess.run(
                ["cargo", "check", "--offline", "--lib", "--manifest-path",
                 str(directory / "Cargo.toml"), "--target", str(ROOT / target),
                 "--target-dir", str(ROOT / "target/x86-abi-controls")],
                cwd=ROOT, env=env, capture_output=True, text=True)

    def test_userspace_accepts_sse2_and_sysv_float_abi(self):
        result = self.compile("crates/knytos-services/x86_64-unknown-none.json")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("future incompatib", result.stderr)

    def test_kernel_accepts_sse2_and_sysv_float_abi(self):
        result = self.compile("target_specs/x86_64-unknown-none-catten.json")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("future incompatib", result.stderr)

    def test_kernel_rejects_unsaved_avx_codegen(self):
        # Include the production guard, with only the surrounding boot-protocol
        # types stubbed. This is a compiler control; privileged code never runs.
        boot = ROOT / "crates/catten/src/cpu/isa/x86_64/init/boot.rs"
        source = (SOURCE + 'extern crate self as limine;\n'
                  'pub mod mp { pub struct MpInfo; }\n'
                  'pub unsafe extern "C" fn ap_main(_: &mp::MpInfo) -> ! { loop {} }\n'
                  f'#[path = "{boot}"] mod boot;\n')
        result = self.compile("target_specs/x86_64-unknown-none-catten.json", source, avx=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("AVX code generation requires an extended-state ownership contract", result.stderr)


if __name__ == "__main__":
    unittest.main()
