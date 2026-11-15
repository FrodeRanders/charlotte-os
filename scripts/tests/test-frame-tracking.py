#!/usr/bin/env python3
"""Compile the real retaining tracker and byte allocator with host ABI adapters."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]
TRACKER = ROOT / "crates/catten/src/memory/frame_tracking.rs"


def main():
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    env = dict(os.environ, CATTEN_RESOURCE_POLICY_FILE=str(ROOT / "crates/catten/src/resource_policy/defaults.rs"))
    source = (ROOT / "scripts/tests/fixtures/frame-tracking.rs").read_text()
    paths = dict(SDK="crates/catten-syscall/src/lib.rs", RESOURCES="crates/charlotte-lifecycle/src/resources.rs",
                 POLICY="crates/catten/src/resource_policy.rs", ADMISSION="crates/catten/src/klib/admission.rs")
    for name, path in paths.items():
        source = source.replace("= " + name + "]", "= " + json.dumps(str(ROOT / path)) + "]")
    with tempfile.TemporaryDirectory(prefix="charlotte-frame-tracking-") as temporary:
        directory = Path(temporary)
        harness = directory / "harness.rs"
        binary = directory / "tests"

        def compile_tracker(path, expected_error=None):
            harness.write_text(source.replace("= TRACKER]", "= " + json.dumps(str(path)) + "]"))
            result = subprocess.run(["rustc", "+" + channel, "--edition=2024", "--test", str(harness), "-D", "warnings", "-o", str(binary)],
                                    cwd=ROOT, env=env, capture_output=True, text=True)
            output = result.stdout + result.stderr
            if expected_error is None:
                assert result.returncode == 0, output
            else:
                assert result.returncode != 0 and expected_error in output, output

        compile_tracker(TRACKER)
        subprocess.run([str(binary), "--test-threads=1"], check=True)
        broken = directory / "eager-release.rs"
        broken.write_text(TRACKER.read_text().replace('#[path = "frame_tracking/host_tests.rs"]', "#[path = " + json.dumps(str(TRACKER.parent / "frame_tracking/host_tests.rs")) + "]")
                          + "\nimpl Drop for FrameTracker { fn drop(&mut self) { unsafe { ManuallyDrop::drop(&mut self.0) }; } }\n")
        compile_tracker(broken)
        result = subprocess.run([str(binary), "memory::frame_tracking::host_tests::abandonment_keeps_real_allocation_and_both_original_charges", "--exact", "--test-threads=1"],
                                capture_output=True, text=True)
        assert "running 1 test" in result.stdout, result.stdout
        assert result.returncode != 0 and "unconfirmed abandonment freed backing" in result.stdout, result.stdout + result.stderr
        print("Frame tracking controls: eager buffer release rejected.")
        # Initial binding can retarget the private sponsor cell. It must not
        # inherit the stronger shared-allocation clone-equivalence promise.
        mutable = directory / "mutable-clone.rs"
        mutable.write_text(TRACKER.read_text().replace('#[path = "frame_tracking/host_tests.rs"]',
                           "#[path = " + json.dumps(str(TRACKER.parent / "frame_tracking/host_tests.rs")) + "]") + r'''
impl FrameTracker {
    fn require_equivalent_allocator(&self) {
        fn equivalent<A: core::alloc::AllocatorClone>(_: &A) {}
        equivalent(self.0.allocator());
    }
}
''')
        compile_tracker(mutable, "ByteSponsorClone")
        print("Frame tracking controls: mutable sponsor rejected as a clone-equivalent allocator.")


if __name__ == "__main__":
    main()
