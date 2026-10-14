#!/usr/bin/env python3
"""Check that CI's pinned Sitas source matches the current kernel wait ABI."""

from pathlib import Path
import re
import subprocess
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]
SITAS = ROOT.parent.parent / "gautelis/sitas"


def checked_wait_number(source):
    match = re.search(r"const SYSCALL_CQ_WAIT_CHECKED:\s*u16\s*=\s*(\d+);", source)
    if match is None:
        raise ValueError("Sitas lacks the mandatory checked CQ wait")
    retired = re.findall(r"const SYSCALL_CQ_WAIT\w*:\s*u16\s*=\s*(12|42);", source)
    if retired:
        raise ValueError("Sitas still declares retired CQ waits")
    return int(match.group(1))


class CiRuntimeInputs(unittest.TestCase):
    def setUp(self):
        self.workflow = (ROOT / ".github/workflows/ci.yml").read_text()

    def sitas_source(self, revision):
        return subprocess.check_output(
            ["git", "-C", str(SITAS), "show", f"{revision}:crates/sitas-charlotte/src/lib.rs"],
            text=True,
        )

    def test_pinned_dependency_uses_the_current_kernel_wait_number(self):
        pin = re.search(r"^  SITAS_REV: ([0-9a-f]{40})$", self.workflow, re.MULTILINE)
        self.assertIsNotNone(pin, "CI must use a full immutable Sitas revision")
        syscall = (ROOT / "crates/catten-syscall/src/lib.rs").read_text()
        number = re.search(r"\(CqWaitChecked,\s*(\d+)\)", syscall)
        self.assertIsNotNone(number)
        self.assertEqual(checked_wait_number(self.sitas_source(pin.group(1))), int(number.group(1)))

    def test_previous_ci_pin_is_rejected(self):
        source = self.sitas_source("94eca68554e88f06721df4ccf3a69222a963c2c9")
        with self.assertRaisesRegex(ValueError, "lacks the mandatory checked CQ wait"):
            checked_wait_number(source)

    def test_every_dependency_checkout_uses_the_shared_pin(self):
        clones = self.workflow.split('git clone https://github.com/FrodeRanders/sitas.git "$SITAS_DIR"')
        self.assertGreater(len(clones), 1)
        for following in clones[1:]:
            self.assertTrue(following.lstrip().startswith('git -C "$SITAS_DIR" checkout --detach "$SITAS_REV"'))

    def test_ci_warning_flags_preserve_kernel_frame_pointers(self):
        self.assertNotRegex(self.workflow, r"(?m)^\s*RUSTFLAGS:")
        self.assertIn('CARGO_BUILD_RUSTFLAGS: "-D warnings"', self.workflow)
        config = tomllib.loads((ROOT / ".cargo/config.toml").read_text())
        for arch in ("aarch64", "x86_64"):
            flags = config["target"][f"{arch}-unknown-none-catten"]["rustflags"]
            self.assertIn("force-frame-pointers=yes", flags)
            self.assertIn("warnings", flags)
            self.assertIn("-D", flags)


if __name__ == "__main__":
    unittest.main()
