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
    actual_calibration = ROOT / "crates/catten/src/cpu/isa/x86_64/timers/apic_timer/calibration.rs"
    actual_hw = ROOT / "crates/catten/src/cpu/isa/aarch64/memory/paging/hw_asid.rs"
    with tempfile.TemporaryDirectory(prefix="charlotte-bootstrap-storage-") as temporary:
        directory = Path(temporary)
        harness, binary = directory / "controls.rs", directory / "controls"

        def compile_and_run(path, *args, capture=False, hw_path=actual_hw, calibration_path=actual_calibration):
            harness.write_text(source.replace("@BOOT_TIMER@", str(path)).replace("@HW_ASID@", str(hw_path)).replace("@CALIBRATION@", str(calibration_path)))
            subprocess.run([rustc, "--edition=2024", "--test", str(harness), "-o", str(binary)], check=True)
            return subprocess.run([str(binary), *args, "--test-threads=1"], capture_output=capture, text=True)

        assert compile_and_run(actual).returncode == 0
        calibration_control = directory / "late-calibration-start.rs"
        calibration_source = actual_calibration.read_text()
        ordering = "    let tsc_start = timestamp();\n    arm(SAMPLE_TICKS);"
        assert calibration_source.count(ordering) == 1
        calibration_control.write_text(calibration_source.replace(ordering, "    arm(SAMPLE_TICKS);\n    let tsc_start = timestamp();"))
        rejected = compile_and_run(actual, "--exact", "calibration_tests::elapsed_countdown_before_timestamp_is_still_measured", capture=True, calibration_path=calibration_control)
        assert rejected.returncode != 0 and "complete countdown escaped measured interval" in rejected.stdout + rejected.stderr
        print("Bootstrap storage controls: countdown armed before measurement rejected.")
        control = directory / "allocating-get.rs"
        current = actual.read_text()
        needle = "self.state.clone()"
        assert current.count(needle) == 1
        control.write_text(current.replace(needle, "{ let extra = alloc::boxed::Box::new(7u64); core::hint::black_box(&extra); self.state.clone() }"))
        rejected = compile_and_run(control, "--exact", "local_initialization_and_runtime_access_allocate_no_backing", capture=True)
        assert rejected.returncode != 0 and "runtime timer backing allocation" in rejected.stdout + rejected.stderr
        print("Bootstrap storage controls: allocating runtime timer access rejected.")
        # Restore growing retirement storage in a private source copy. The
        # allocation tracer must reject actual tag returns, not just a counter.
        current_hw = actual_hw.read_text()
        reserve = "        free.try_reserve_exact(limit as usize)?;"
        assert current_hw.count(reserve) == 1
        old = current_hw.replace(reserve, "")
        start = old.index("        assert!(\n            self.free.len()")
        end = old.index("        self.free.push(asid);", start)
        old = old[:start] + old[end:]
        control_hw = directory / "growing-hw-asid.rs"
        control_hw.write_text(old)
        rejected = compile_and_run(actual, "--exact", "hardware_asid_return_and_exhaustion_allocate_no_backing", capture=True, hw_path=control_hw)
        assert rejected.returncode != 0 and "hardware tag retirement allocated backing" in rejected.stdout + rejected.stderr
        print("Bootstrap storage controls: growing hardware-ASID retirement storage rejected.")


if __name__ == "__main__":
    run()
