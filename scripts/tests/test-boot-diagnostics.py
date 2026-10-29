#!/usr/bin/env python3
"""Failure triggers, bounded capture and per-run evidence retention."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("boot_diagnostics", ROOT / "scripts/boot-diagnostics.py")
DIAG = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(DIAG)
SUCCESS = (b"SELFTEST COMPLETE: passed=1 failed=0 pending=0 passed_bitmap=0x1 "
           b"failed_bitmap=0x0 pending_bitmap=0x0\n")


class BootDiagnostics(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="charlotte-diagnostic-test-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.log = self.directory / "input.log"
        self.log.write_bytes(b"")
        self.manifest = {"serial_log": str(self.log), "transport": str(self.directory),
                         "timeout_seconds": 1, "cpus": 8}
        DIAG.write_json(self.directory / "manifest.json", self.manifest)

    def watch(self):
        DIAG.watch(argparse.Namespace(directory=str(self.directory), pid=os.getpid(), qemu=[]))
        return json.loads((self.directory / "result.json").read_text())

    def test_shell_finish_preserves_arguments_under_nounset(self):
        arguments = self.directory / "finish-arguments"
        for snapshot in ("", "0", "1"):
            with self.subTest(snapshot=snapshot):
                result = subprocess.run(
                    ["/bin/bash", "-euo", "pipefail", "-c", r'''
source "$1/scripts/lib/boot-common.sh"
CATTEN_BOOT_ROOT_DIR="$1"
CATTEN_DIAGNOSTICS_DIR="$2"
probe_arguments="$3"
python3() { printf '%s\0' "$@" > "$probe_arguments"; }
true &
CATTEN_DIAGNOSTICS_WATCHER=$!
if [ -n "$4" ]; then
    catten_boot_diagnostics_finish "selftests complete" "$4"
else
    catten_boot_diagnostics_finish "selftests complete"
fi
[ -z "$CATTEN_DIAGNOSTICS_WATCHER" ]
[ "$CATTEN_DIAGNOSTICS_FAILED" = "0" ]
''', "boot-finish-test", str(ROOT), str(self.directory / "diagnostics with spaces"),
                     str(arguments), snapshot], capture_output=True, text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                expected = [str(ROOT / "scripts/boot-diagnostics.py"), "finish", "--directory",
                            str(self.directory / "diagnostics with spaces"), "--watcher"]
                actual = arguments.read_bytes().decode().split("\0")[:-1]
                self.assertEqual(actual[:5], expected)
                self.assertTrue(actual[5].isdigit())
                self.assertEqual(actual[6:], ["--reason", "selftests complete"]
                                 + (["--snapshot"] if snapshot == "1" else []))

    def test_failure_markers_and_terminal_results_never_turn_into_success(self):
        self.assertIsNone(DIAG.failure_reason(SUCCESS))
        self.assertIsNone(DIAG.failure_reason(b"FATAL USER FAULT\nFATAL EL0 DATA/INST ABORT\n"))
        for marker, reason in DIAG.MARKERS:
            self.assertEqual(DIAG.failure_reason(SUCCESS + marker), reason)
        for field in (b"failed=0", b"pending=0", b"failed_bitmap=0x0", b"pending_bitmap=0x0"):
            failed = SUCCESS.replace(field, field[:-1] + b"1")
            self.assertEqual(DIAG.failure_reason(failed), "unsuccessful-selftest-result")
            self.assertIsNone(DIAG.failure_reason(failed[:-1]))

    def test_failure_is_published_before_capture_and_overrides_complete_request(self):
        self.log.write_bytes(SUCCESS + b"Kernel panic:\n")
        DIAG.write_json(self.directory / "request.json", {"reason": "complete", "snapshot": False})

        def capture(directory, manifest, reason):
            failure = json.loads((directory / "failure.json").read_text())
            self.assertEqual(failure["reason"], "kernel-panic")
            self.assertEqual(reason, "kernel-panic")
            return {"reason": reason, "snapshot": "test-captured"}

        with patch.object(DIAG, "capture", side_effect=capture) as captured:
            result = self.watch()
        self.assertEqual(result["reason"], "kernel-panic")
        captured.assert_called_once()
        self.assertEqual((self.directory / "serial.log").read_bytes(), self.log.read_bytes())

    def test_deadline_is_not_restarted_by_unrelated_serial_progress(self):
        with patch.object(DIAG.time, "monotonic", side_effect=[0, 0.4, 0.8, 1.2]), \
             patch.object(DIAG.time, "sleep", side_effect=lambda _: self.log.write_bytes(b"more progress\n")), \
             patch.object(DIAG, "capture", return_value={"reason": "deadline"}) as captured:
            result = self.watch()
        self.assertEqual(result["elapsed_seconds"], 1.2)
        captured.assert_called_once_with(self.directory, unittest.mock.ANY, "deadline")

    def test_success_never_attaches_without_explicit_snapshot(self):
        self.log.write_bytes(SUCCESS)
        DIAG.write_json(self.directory / "request.json", {"reason": "complete", "snapshot": False})
        with patch.object(DIAG, "capture") as captured:
            self.assertEqual(self.watch(), {"reason": "complete", "snapshot": "not-requested"})
        captured.assert_not_called()
        self.assertFalse((self.directory / "failure.json").exists())

    def fake_qmp(self, running):
        events = []

        class FakeQmp:
            def __init__(_self, _path):
                pass

            def command(_self, command, arguments=None):
                events.append(command)
                return {"query-status": {"running": running},
                        "query-cpus-fast": [{"cpu-index": i} for i in range(8)],
                        "human-monitor-command": "all eight LP registers\n"}.get(command, {})

            def close(_self):
                events.append("close")

        return FakeQmp, events

    def test_capture_reads_every_lp_and_full_symbol_size_then_restores_state(self):
        (self.directory / "kernel.symbols").write_text("00000100 00000050 B SCHEDULER_LP_DIAGNOSTICS\n")
        for running in (True, False):
            qmp, events = self.fake_qmp(running)

            def debugger(command, **kwargs):
                self.assertEqual(events[1], "stop")
                self.assertIn("thread backtrace all", command)
                self.assertIn("thread select 8", command)
                self.assertIn("memory read --force --format x --size 8 --count 10 0x100", command)
                self.assertIn("unix-connect://" + str(self.directory / "gdb.sock"), " ".join(command))
                kwargs["stdout"].write("all eight LP backtraces and registers\n")
                return subprocess.CompletedProcess(command, 0)

            with patch.object(DIAG, "Qmp", qmp), patch.object(DIAG.shutil, "which", return_value="lldb"), \
                 patch.object(DIAG.subprocess, "run", side_effect=debugger):
                result = DIAG.capture(self.directory, self.manifest, "test")
            self.assertEqual(result["snapshot"], "captured")
            self.assertEqual(events[-2:], ["cont" if running else "stop", "close"])

    def test_debugger_timeout_keeps_qmp_registers_and_reports_partial(self):
        qmp, events = self.fake_qmp(True)
        with patch.object(DIAG, "Qmp", qmp), patch.object(DIAG.shutil, "which", return_value="lldb"), \
             patch.object(DIAG.subprocess, "run", side_effect=subprocess.TimeoutExpired("lldb", 15)):
            result = DIAG.capture(self.directory, self.manifest, "deadline")
        self.assertEqual(result["snapshot"], "partial")
        self.assertTrue(result["errors"][0].startswith("LLDB:"))
        self.assertEqual(events[-2:], ["cont", "close"])
        self.assertEqual((self.directory / "registers.log").read_text(), "all eight LP registers\n")

    def test_missing_debugger_is_partial_rather_than_a_fabricated_backtrace(self):
        qmp, _events = self.fake_qmp(True)
        with patch.object(DIAG, "Qmp", qmp), patch.object(DIAG.shutil, "which", return_value=None):
            result = DIAG.capture(self.directory, self.manifest, "deadline")
        self.assertEqual(result["lldb"], "unavailable")
        self.assertEqual(result["snapshot"], "partial")
        self.assertFalse((self.directory / "lldb.log").exists())

    def test_debugger_ownership_conflicts_reject_before_build(self):
        for architecture in ("aarch64", "x86_64"):
            for arguments in (["--debug-snapshot"], ["--debug-snapshot", "--timeout", "1", "--gdb"],
                              ["--debug-snapshot", "--timeout", "1", "--no-failure-capture"]):
                result = subprocess.run([str(ROOT / f"scripts/run-{architecture}.sh"), *arguments],
                                        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(b"error: --debug-snapshot", result.stdout)
                self.assertNotIn(b"Building", result.stdout)

    def test_prepare_failure_releases_only_its_unused_private_storage(self):
        root = self.directory / "repository"
        root.mkdir()
        (root / "keep-user-file").write_text("preserve")
        with patch.object(DIAG, "prepare_inputs", side_effect=ValueError("wrong kernel identity")):
            with self.assertRaisesRegex(ValueError, "wrong kernel"):
                DIAG.prepare(argparse.Namespace(root=str(root), arch="aarch64"))
        self.assertEqual(list((root / "target/boot-diagnostics").iterdir()), [])
        self.assertEqual((root / "keep-user-file").read_text(), "preserve")

    def test_qmp_buffered_unrelated_events_cannot_restart_the_deadline(self):
        qmp = DIAG.Qmp.__new__(DIAG.Qmp)
        qmp.buffer = b'{"event":"unrelated"}\n' * 20
        qmp.serial = 0
        qmp.socket = unittest.mock.Mock()
        with patch.object(DIAG.time, "monotonic", side_effect=[0, 1, 2.1]):
            with self.assertRaisesRegex(TimeoutError, "deadline"):
                qmp.command("query-status")

    def test_payload_identity_comes_from_boot_image_and_rejects_mismatched_symbols(self):
        image = self.directory / "boot.img"
        image.write_bytes(b"boot image identity")
        kernel = self.directory / "build-output"
        kernel.write_bytes(b"newer unrelated build")
        payload_hash = DIAG.hashlib.sha256(b"executed payload").hexdigest()
        Path(str(self.log) + ".kernel.sha256").write_text(payload_hash)
        symbols = Path(str(self.log) + ".symbols")
        symbols.write_text("# kernel-sha256: " + payload_hash + "\n")
        args = argparse.Namespace(root=str(ROOT), kernel=str(kernel), boot_image=str(image),
                                  log=str(self.log), arch="x86_64", instance="fixture", cpus=8,
                                  bundle=str(self.directory), disk=str(image), timeout=10)

        def extract(command, **_kwargs):
            self.assertEqual(command[:4], ["mcopy", "-i", str(image), "::/catten"])
            Path(command[4]).write_bytes(b"executed payload")

        with patch.object(DIAG.subprocess, "run", side_effect=extract), \
             patch.object(DIAG.subprocess, "check_output", side_effect=["1109d05a\n", "", b"tracked patch"]):
            manifest = DIAG.prepare_inputs(args, self.directory)
            self.assertEqual(manifest["kernel_sha256"], payload_hash)
            self.assertEqual((self.directory / "kernel.elf").read_bytes(), b"executed payload")
            symbols.write_text("# kernel-sha256: " + "0" * 64 + "\n")
            with self.assertRaisesRegex(ValueError, "symbol map"):
                DIAG.prepare_inputs(args, self.directory)

    def test_dead_watcher_retains_unavailable_result_and_releases_its_transport(self):
        transport = self.directory / "private-transport"
        transport.mkdir()
        for name in ("qmp.sock", "gdb.sock"):
            (transport / name).touch()
        manifest = dict(self.manifest, transport=str(transport))
        DIAG.write_json(self.directory / "manifest.json", manifest)
        with patch.object(DIAG, "alive", return_value=False):
            DIAG.finish(argparse.Namespace(directory=str(self.directory), watcher=123,
                                           reason="complete", snapshot=False))
        result = json.loads((self.directory / "result.json").read_text())
        self.assertEqual(result["snapshot"], "unavailable")
        self.assertTrue(result["errors"])
        self.assertFalse(transport.exists())
        self.assertTrue(self.log.exists())
        artifacts = json.loads((self.directory / "artifacts.json").read_text())
        self.assertEqual(artifacts["serial.log"], DIAG.digest(self.log))

    def test_final_archive_includes_uart_tail_after_debugger_detach(self):
        self.log.write_bytes(b"before detach\n")
        args = argparse.Namespace(directory=str(self.directory))
        DIAG.archive(args)
        self.log.write_bytes(b"before detach\nafter detach, before QEMU exit\n")
        DIAG.archive(args)
        self.assertEqual((self.directory / "serial.log").read_bytes(), self.log.read_bytes())
        artifacts = json.loads((self.directory / "artifacts.json").read_text())
        self.assertEqual(artifacts["serial.log"], DIAG.digest(self.log))


if __name__ == "__main__":
    unittest.main()
