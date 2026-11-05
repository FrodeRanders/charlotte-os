#!/usr/bin/env python3
"""Exercise actual timer boot phases/calibration and IORT parsing/cache."""
from pathlib import Path
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]


def run():
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    rustc = subprocess.check_output(["rustup", "which", "--toolchain", channel, "rustc"], text=True).strip()
    source = (ROOT / "scripts/tests/fixtures/bootstrap-storage.rs").read_text().replace("@ROOT@", str(ROOT))
    actual = ROOT / "crates/catten/src/cpu/isa/interface/timers/bootstrap.rs"
    with tempfile.TemporaryDirectory(prefix="charlotte-bootstrap-storage-") as temporary:
        directory = Path(temporary)
        harness, binary = directory / "controls.rs", directory / "controls"

        def compile_and_run(path, *args, capture=False):
            harness.write_text(source.replace("@BOOT_TIMER@", str(path)))
            subprocess.run([rustc, "--edition=2024", "--test", str(harness), "-o", str(binary)], check=True)
            return subprocess.run([str(binary), *args, "--test-threads=1"], capture_output=capture, text=True)

        assert compile_and_run(actual).returncode == 0
        control = directory / "allocating-get.rs"
        current = actual.read_text()
        needle = "self.state.clone()"
        assert current.count(needle) == 1
        control.write_text(current.replace(needle, "{ let extra = alloc::boxed::Box::new(7u64); core::hint::black_box(&extra); self.state.clone() }"))
        rejected = compile_and_run(control, "--exact", "local_initialization_and_runtime_access_allocate_no_backing", capture=True)
        assert rejected.returncode != 0 and "runtime timer backing allocation" in rejected.stdout + rejected.stderr
        print("Bootstrap storage controls: allocating runtime timer access rejected.")


if __name__ == "__main__":
    run()
