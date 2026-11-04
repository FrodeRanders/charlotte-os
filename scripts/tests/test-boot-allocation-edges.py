#!/usr/bin/env python3
"""Exercise actual boot factories and the pinned C terminal backend on the host."""
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = Path(__file__).resolve().parent / "fixtures"


def run():
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    rustc = subprocess.check_output(["rustup", "which", "--toolchain", channel, "rustc"], text=True).strip()
    with tempfile.TemporaryDirectory(prefix="charlotte-boot-edges-") as temporary:
        directory = Path(temporary)
        dependency = ROOT / "crates/flanterm-bindings"
        cc = os.environ.get("CC", "cc")
        layout_probe = directory / "layout-probe"
        subprocess.run([cc, "-std=c11", "-DQ3_LAYOUT_PROBE", "-I", str(dependency),
                        str(FIXTURES / "boot-terminal.c"), "-o", str(layout_probe)], check=True)
        malloc_alignment, base_alignment = subprocess.check_output([str(layout_probe)], text=True).split()
        for source, name in [(dependency / "flanterm.c", "terminal"),
                             (dependency / "backends/fb.c", "framebuffer"),
                             (FIXTURES / "boot-terminal.c", "probe")]:
            subprocess.run([cc, "-std=c11", "-ffreestanding", "-fno-builtin", "-I", str(dependency),
                            "-c", str(source), "-o", str(directory / f"{name}.o")], check=True)
        source = ((FIXTURES / "boot-allocation-edges.rs").read_text().replace("@ROOT@", str(ROOT))
                  .replace("@MALLOC_ALIGNMENT@", malloc_alignment))
        harness = directory / "harness.rs"
        harness.write_text(source.replace("@CONSOLE@", str(ROOT / "crates/catten/src/log/flanterm.rs")))

        def compile_to(output):
            subprocess.run([rustc, "--edition=2024", "--test", str(harness), "-o", str(output),
                            *[f"-Clink-arg={directory / (name + '.o')}" for name in ("terminal", "framebuffer", "probe")]],
                           cwd=directory, check=True)

        executable = directory / "controls"
        compile_to(executable)
        subprocess.run([str(executable), "--exact", "log::flanterm::terminal", "--test-threads=1"], check=True)
        for count in (1, 4, 256, 0, 257):
            result = subprocess.run([str(executable), "--exact", "lp_count", "--nocapture"],
                                    env=dict(os.environ, Q3_LP_COUNT=str(count)), capture_output=True, text=True)
            output = result.stdout + result.stderr
            if count in (0, 257):
                assert result.returncode != 0 and "outside the supported range" in output, output
            else:
                assert result.returncode == 0, output
        # A base-context deallocator must be detected even if it uses the right
        # pointer: it releases only part of the containing C operation and uses
        # the wrong layout. Never run C deinit after that rejected control.
        unsafe = directory / "unsafe-console.rs"
        unsafe.write_text((ROOT / "crates/catten/src/log/flanterm.rs").read_text() + r'''
impl Drop for BootContext {
    fn drop(&mut self) { unsafe {
        alloc::alloc::dealloc(self.0.as_ptr().cast(),
            core::alloc::Layout::from_size_align(crate::q3_base_size(), @BASE_ALIGNMENT@).unwrap());
    }}
}
'''.replace("@BASE_ALIGNMENT@", base_alignment))
        harness.write_text(source.replace("@CONSOLE@", str(unsafe)))
        compile_to(executable)
        rejected = subprocess.run([str(executable), "--exact", "log::flanterm::terminal", "--nocapture"],
                                  capture_output=True, text=True)
        output = rejected.stdout + rejected.stderr
        assert rejected.returncode != 0 and "console abandonment released backend storage" in output, output
        print("Boot allocation edges: CPU boundaries, six C failure prefixes, exact cleanup, allocation-free writes and unsafe-owner rejection passed.")


if __name__ == "__main__":
    run()
