#!/usr/bin/env python3
"""Run the actual secret owner and byte admission with a tracing host allocator."""
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]


def run():
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    cargo = subprocess.check_output(["rustup", "which", "--toolchain", channel, "cargo"], text=True).strip()
    rustc = subprocess.check_output(["rustup", "which", "--toolchain", channel, "rustc"], text=True).strip()
    env = dict(os.environ, RUSTC=rustc, CATTEN_RESOURCE_POLICY_FILE=str(ROOT / "crates/catten/src/resource_policy/defaults.rs"))
    with tempfile.TemporaryDirectory(prefix="charlotte-sensitive-buffer-") as temporary:
        directory = Path(temporary)
        (directory / "src").mkdir()
        (directory / "Cargo.toml").write_text(
            '[package]\nname="sensitive-buffer-controls"\nversion="0.0.0"\nedition="2024"\n'
            '[workspace]\n[dependencies]\nzeroize={version="=1.9.0",default-features=false}\n')
        (directory / "Cargo.lock").write_bytes((ROOT / "Cargo.lock").read_bytes())
        source = r'''
#![feature(allocator_api)]
#![allow(dead_code, unused_attributes, unexpected_cfgs)]
extern crate alloc;
extern crate self as charlotte_lifecycle;
extern crate self as catten_syscall;
#[path = "@ROOT@/crates/catten-syscall/src/lib.rs"] mod syscall_format;
pub use syscall_format::*;
#[path = "@ROOT@/crates/charlotte-lifecycle/src/resources.rs"] pub mod resources;
#[path = "@ROOT@/crates/catten/src/resource_policy.rs"] mod resource_policy;
#[macro_export] macro_rules! logln { ($message:literal) => {}; }
mod cpu { pub mod multiprocessor { pub mod spin { pub mod mutex {
    pub struct Mutex<T>(std::sync::Mutex<T>);
    impl<T> Mutex<T> {
        pub const fn new(value:T)->Self { Self(std::sync::Mutex::new(value)) }
        pub fn lock(&self)->std::sync::MutexGuard<'_,T> { self.0.lock().unwrap() }
        pub fn try_lock(&self)->Option<std::sync::MutexGuard<'_,T>> { self.0.try_lock().ok() }
    }
}}}}
mod klib {
    #[path = "@ROOT@/crates/catten/src/klib/admission.rs"] pub mod admission;
    #[path = "@OWNER@"] mod sensitive_buffer;
}
'''
        (directory / "src/lib.rs").write_text(source.replace("@ROOT@", str(ROOT)).replace("@OWNER@", str(ROOT / "crates/catten/src/klib/sensitive_buffer.rs")))
        # Fetch with an existing source target before the locked offline test;
        # an uncached CI runner must not be mistaken for allocator rejection.
        subprocess.run([cargo, "fetch"], cwd=directory, env=env, check=True)
        subprocess.run([cargo, "test", "--release", "--lib", "--locked", "--offline", "--target-dir",
                        str(ROOT / "target/sensitive-buffer-controls"), "--", "--test-threads=1"],
                       cwd=directory, env=env, check=True)

        control = (ROOT / "crates/catten/src/klib/sensitive_buffer.rs").read_text()
        control = control.replace("self.0.as_mut_slice().zeroize();", "").replace("self.0.spare_capacity_mut().zeroize();", "")
        control = control.replace('#[path = "sensitive_buffer/tests.rs"]',
                                  f'#[path = "{ROOT}/crates/catten/src/klib/sensitive_buffer/tests.rs"]')
        (directory / "unwiped.rs").write_text(control)
        (directory / "src/lib.rs").write_text(source.replace("@ROOT@", str(ROOT)).replace("@OWNER@", str(directory / "unwiped.rs")))
        rejected = subprocess.run([cargo, "test", "--release", "--lib", "--locked", "--offline", "--target-dir",
                                   str(ROOT / "target/sensitive-buffer-controls"), "--", "--test-threads=1"],
                                  cwd=directory, env=env, capture_output=True, text=True)
        output = rejected.stdout + rejected.stderr
        if rejected.returncode == 0 or "plaintext reached the backend allocator" not in output:
            raise AssertionError(output)
        print("Sensitive-buffer unwiped control rejected: tracer detected plaintext before backend free")


if __name__ == "__main__":
    run()
