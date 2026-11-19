#!/usr/bin/env python3
"""TLC input preflight rejects rename mistakes on every host filesystem."""

import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("tla_inputs", ROOT / "docs/tla/check-inputs.py")
INPUTS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(INPUTS)


class TlaInputs(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="knytos-tla-inputs-")
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        (self.directory / "check.sh").write_text(
            "run_model Ipc Ipc_small.cfg \\\n"
            "    Send Receive\n"
            "run_expected_violation Ipc Ipc_unsafe.cfg Accounted UnsafeRefund\n")
        (self.directory / "Ipc.tla").write_text("---- MODULE Ipc ----\n====\n")
        for name in ("Ipc_small.cfg", "Ipc_unsafe.cfg"):
            (self.directory / name).write_text("SPECIFICATION Spec\n")

    def test_complete_registry_passes(self):
        self.assertEqual(INPUTS.validate_inputs(self.directory), (1, 2))

    def test_wrong_configuration_case_rejects_even_on_macos(self):
        (self.directory / "Ipc_small.cfg").rename(self.directory / "IPC_small.cfg")
        with self.assertRaisesRegex(ValueError, "missing TLC input Ipc_small.cfg.*case differs"):
            INPUTS.validate_inputs(self.directory)

    def test_wrong_model_case_rejects_even_on_macos(self):
        (self.directory / "Ipc.tla").rename(self.directory / "IPC.tla")
        with self.assertRaisesRegex(ValueError, "missing TLC input Ipc.tla.*case differs"):
            INPUTS.validate_inputs(self.directory)

    def test_wrong_module_declaration_rejects(self):
        (self.directory / "Ipc.tla").write_text("---- MODULE IPC ----\n====\n")
        with self.assertRaisesRegex(ValueError, "must declare MODULE Ipc; found IPC"):
            INPUTS.validate_inputs(self.directory)

    def test_missing_negative_control_rejects(self):
        (self.directory / "Ipc_unsafe.cfg").unlink()
        with self.assertRaisesRegex(ValueError, "missing TLC input Ipc_unsafe.cfg"):
            INPUTS.validate_inputs(self.directory)

    def test_focused_run_ignores_unselected_inputs(self):
        with (self.directory / "check.sh").open("a") as registry:
            registry.write("run_model Missing Missing_small.cfg Next\n")
        self.assertEqual(INPUTS.validate_inputs(self.directory, "Ipc"), (1, 2))
        with self.assertRaisesRegex(ValueError, "missing TLC input Missing"):
            INPUTS.validate_inputs(self.directory)

    def test_unknown_module_rejects(self):
        with self.assertRaisesRegex(ValueError, "no configurations registered for Unknown"):
            INPUTS.validate_inputs(self.directory, "Unknown")

    def test_repository_registry_passes(self):
        modules, cases = INPUTS.validate_inputs(ROOT / "docs/tla")
        self.assertGreater(modules, 0)
        self.assertGreater(cases, modules)

    def test_runner_rejects_inputs_before_starting_java(self):
        for name in ("check.sh", "check-inputs.py"):
            shutil.copy2(ROOT / "docs/tla" / name, self.directory / name)
        (self.directory / "Ipc_small.cfg").rename(self.directory / "IPC_small.cfg")
        jar = self.directory / "tools.jar"
        jar.touch()
        marker = self.directory / "java-started"
        java = self.directory / "java"
        java.write_text('#!/bin/sh\ntouch "$TLA_TEST_JAVA_MARKER"\nexit 99\n')
        java.chmod(0o755)
        result = subprocess.run(
            ["bash", str(self.directory / "check.sh"), str(jar), "Ipc"],
            capture_output=True, text=True, timeout=10,
            env={**os.environ, "PATH": str(self.directory) + os.pathsep + os.environ["PATH"],
                 "TLA_TEST_JAVA_MARKER": str(marker)})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing TLC input Ipc_small.cfg", result.stderr)
        self.assertFalse(marker.exists())


if __name__ == "__main__":
    unittest.main()
