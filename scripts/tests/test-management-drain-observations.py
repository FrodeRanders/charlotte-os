#!/usr/bin/env python3
"""Reject false drain success and replay of an uncertain signed submission."""
import importlib.util
import io
from pathlib import Path
import subprocess
import types
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("management_drain", Path(__file__).with_name("test-management-drain.py"))
drain = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drain)

GOOD = """[deployd] management drain begin inputs=2 controls=1 outputs=0 listeners=1 closes=0
[deployd] management drain complete inputs=0 controls=0 outputs=0 listeners=0 closes=0
[shutdown] DeploymentIngress: acknowledged=1 unacknowledged=0 forced=0 lifecycle_status=2
[http] management drain begin inputs=2 outputs=0 listeners=2 closes=0
[http] management drain complete inputs=0 outputs=0 listeners=0 closes=0
[shutdown] HttpIngress: acknowledged=1 unacknowledged=0 forced=0 lifecycle_status=2
[shutdown] POWER-OFF REQUESTED via PSCI
"""


class Observations(unittest.TestCase):
    def test_active_clients_cooperative_retirement_and_poweroff(self):
        drain.verify(GOOD)

    def test_live_phase_cannot_be_hidden_by_empty_listeners_and_closes(self):
        for field in ("inputs", "controls", "outputs"):
            with self.subTest(field=field):
                old = "[deployd] management drain complete inputs=0 controls=0 outputs=0"
                new = old.replace(field + "=0", field + "=1")
                with self.assertRaisesRegex(ValueError, "live client positions"):
                    drain.verify(GOOD.replace(old, new))

    def test_missing_duplicate_or_idle_only_observations_reject(self):
        for text in (GOOD.replace("begin inputs=2", "begin inputs=0"),
                     GOOD.replace("[http] management drain complete", "missing"),
                     GOOD + GOOD, GOOD.replace("closes=0\n", "closes=0 extra\n")):
            with self.subTest(text=text), self.assertRaises(ValueError):
                drain.verify(text)

    def test_force_unacknowledged_exit_and_false_ready_reject(self):
        for old, new in (("forced=0", "forced=1"), ("unacknowledged=0", "unacknowledged=1"),
                         ("acknowledged=1", "acknowledged=0"), ("lifecycle_status=2", "lifecycle_status=0")):
            with self.subTest(change=new), self.assertRaises(ValueError):
                drain.verify(GOOD.replace(old, new))

    def test_panic_or_early_poweroff_is_not_success(self):
        for text in (GOOD + "Kernel panic:", GOOD + "SELFTEST FAILED: fixture",
                     GOOD.replace("[shutdown] POWER-OFF REQUESTED via PSCI\n", ""),
                     "[shutdown] POWER-OFF REQUESTED via PSCI\n" + GOOD):
            with self.subTest(text=text), self.assertRaises(ValueError):
                drain.verify(text)


class Submission(unittest.TestCase):
    def exercise(self, result):
        peers = []
        class Peer:
            def __init__(self):
                self.closed = False
                peers.append(self)
            def __enter__(self): return self
            def __exit__(self, *_): self.closed = True
            def recv(self, _): return b""
        args = types.SimpleNamespace(status_port=22681, deployment_port=22445,
                                     client_tool=Path("fixture-signer"), intent=Path("fixture.cshutdown"),
                                     serial_log=types.SimpleNamespace(read_text=lambda: "[http] listening port=443; bounded client reactor\n[deployd] listening port=7444\n[time] synchronized unix_s=1793394483 utc=fixture\n[dns] RAFT STATE state=Leader"))
        with patch.object(drain, "context"), patch.object(drain, "exchange"), \
             patch.object(drain.socket, "create_connection", side_effect=lambda *_a, **_k: Peer()), \
             patch.object(drain.time, "sleep"), \
             patch.object(drain.select, "select", side_effect=lambda p, *_: (list(p), [], [])), \
             patch.object(drain.subprocess, "run", side_effect=result) as command, redirect_stdout(io.StringIO()):
            try:
                drain.submit(args)
            finally:
                self.assertEqual(command.call_count, 1, "an uncertain mutation was replayed")
                self.assertEqual(len(peers), 4)
                self.assertTrue(all(p.closed for p in peers))

    def test_accepted_shutdown_submits_once(self):
        self.exercise([types.SimpleNamespace(returncode=0, stdout="202 Accepted\n", stderr="")])

    def test_failed_or_timed_out_submission_is_not_replayed(self):
        for result in ([types.SimpleNamespace(returncode=1, stdout="", stderr="lost reply")],
                       subprocess.TimeoutExpired("fixture-signer", 1)):
            with self.subTest(result=result), self.assertRaises((RuntimeError, subprocess.TimeoutExpired)):
                self.exercise(result)


if __name__ == "__main__":
    unittest.main(verbosity=2)
