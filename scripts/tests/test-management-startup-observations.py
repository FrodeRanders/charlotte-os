#!/usr/bin/env python3
"""Negative controls for native startup evidence and closed-port observations."""
import importlib.util
from pathlib import Path
import unittest
import subprocess
import sys
from unittest.mock import MagicMock, patch

spec = importlib.util.spec_from_file_location("management_startup", Path(__file__).with_name("test-management-startup.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class StartupObservations(unittest.TestCase):
    def test_optimized_python_cannot_disable_security_assertions(self):
        result = subprocess.run([sys.executable, "-O", str(Path(__file__).with_name("test-management-startup.py")), "--help"],
                                capture_output=True, text=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("require enabled runtime checks", result.stderr)

    def test_disabled_needs_both_services_and_ordered_single_publication(self):
        text = probe.PUBLISHED + "1\n" + probe.STEADY + "\nDHCP assigned 10.0.2.15\n" + "\n".join(probe.DISABLED)
        probe.validate_snapshot(text, "disabled")
        for changed in (text.replace(probe.DISABLED[0], ""),
                        text.replace(probe.PUBLISHED, "unrelated marker"),
                        text + "\n" + probe.PUBLISHED + "1\n",
                        text.replace(probe.PUBLISHED + "1\n", "") + probe.PUBLISHED + "1\n",
                        text + "\nKernel panic: unrelated\n",
                        text + "\n[deployd] listening port=7444\n"):
            with self.subTest(text=changed), self.assertRaises(AssertionError):
                probe.validate_snapshot(changed, "disabled")

    def test_rejection_needs_exact_cause_and_no_late_publication(self):
        for cause in ("WrongPolicy", "Signature", "Authentication"):
            text = probe.REJECTED + cause + "\nKernel panic: boot admission trust rejected\n"
            probe.validate_snapshot(text, cause)
            for changed in (text + probe.PUBLISHED + "1\n", text + probe.STEADY,
                            text.replace(cause, "OtherCause"), text.replace(cause, cause + "Suffix")):
                with self.subTest(cause=cause), self.assertRaises(AssertionError):
                    probe.validate_snapshot(changed, cause)

    def test_ready_needs_real_time_and_rejects_disabled_or_panicked_startup(self):
        text = probe.PUBLISHED + "1\n" + probe.STEADY + "\n[deployd] listening port=7444\n[time] synchronized unix_s=1\n"
        probe.validate_snapshot(text, "ready")
        for changed in (text.replace("[time] synchronized ", "no time "),
                        text + probe.DISABLED[1], text + "Kernel panic: unrelated"):
            with self.assertRaises(AssertionError):
                probe.validate_snapshot(changed, "ready")

    def test_timeout_is_not_rejection_evidence(self):
        with patch.object(probe.socket, "create_connection", side_effect=TimeoutError), self.assertRaises(TimeoutError):
            probe.closed(1, b"request", False, 1)

    def test_any_application_data_rejects_disabled_claim(self):
        raw = MagicMock()
        raw.__enter__.return_value = raw
        raw.recv.return_value = b"HTTP/1.1 200 OK\r\n"
        with patch.object(probe.socket, "create_connection", return_value=raw), self.assertRaises(AssertionError):
            probe.closed(1, b"request", False, 1)

    def test_refused_transport_is_a_terminal_observation(self):
        with patch.object(probe.socket, "create_connection", side_effect=ConnectionRefusedError):
            probe.closed(1, b"request", False, 1)


if __name__ == "__main__":
    unittest.main()
