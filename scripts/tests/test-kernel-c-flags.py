#!/usr/bin/env python3
"""Compile the actual C backend with kernel flags against a PIE compiler default."""
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]


def run():
    compiler = shlex.split(os.environ.get("CC") or shutil.which("x86_64-linux-gnu-gcc") or "cc")
    version = subprocess.check_output([*compiler, "--version"], text=True)
    is_clang = "clang" in version.lower()
    if is_clang:
        compiler.append("--target=x86_64-unknown-none-elf")
    else:
        machine = subprocess.check_output([*compiler, "-dumpmachine"], text=True).strip()
        if not machine.startswith("x86_64"):
            raise RuntimeError("use an x86-64 GCC cross-compiler or Clang for kernel C controls")
    config = tomllib.loads((ROOT / ".cargo/config.toml").read_text())
    flags = shlex.split(config["env"]["CFLAGS_x86_64-unknown-none-catten"])
    with tempfile.TemporaryDirectory(prefix="knytos-kernel-c-") as temporary:
        directory = Path(temporary)
        probe = directory / "static.c"
        probe.write_text("""\
#if defined(__PIC__) || defined(__PIE__)
#error kernel C must not use PIC or PIE
#endif
int kernel_data;
int *kernel_address(void) { return &kernel_data; }
""")
        # Model a distro compiler's built-in PIE default explicitly, so this
        # discriminator also runs on development hosts with a different default.
        backend = ROOT / "crates/flanterm-bindings"
        rejected = subprocess.run(
            [*compiler, "-fPIE", "-ffreestanding", "-mcmodel=kernel", "-c", str(probe),
             "-o", str(directory / "rejected.o")], capture_output=True, text=True)
        output = rejected.stdout + rejected.stderr
        assert rejected.returncode != 0 and ("PIC" in output or "PIE" in output), output
        if not is_clang:
            # GCC must reproduce the CI failure in the actual backend too.
            rejected = subprocess.run(
                [*compiler, "-fPIE", "-ffreestanding", "-mcmodel=kernel", "-c",
                 str(backend / "flanterm.c"), "-o", str(directory / "rejected.o")],
                capture_output=True, text=True)
            output = rejected.stdout + rejected.stderr
            assert rejected.returncode != 0 and "PIC" in output and "kernel" in output, output
        for source in (probe, backend / "flanterm.c", backend / "backends/fb.c"):
            subprocess.run(
                [*compiler, "-O3", "-fPIE", *flags, "-fno-stack-protector",
                 "-fno-stack-check", "-mno-red-zone", "-mgeneral-regs-only", "-c", str(source),
                 "-o", str(directory / (source.stem + ".o"))], check=True)
    print("Kernel C flags: rejected PIE control; static probe and both flanterm sources compiled.")


if __name__ == "__main__":
    run()
