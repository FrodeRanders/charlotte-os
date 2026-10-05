#!/usr/bin/env python3
"""A result must be fully written before a boot runner may stop its guest."""

from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SUCCESS = (
    b"[+ 1.2] SELFTEST COMPLETE: passed=19 failed=0 pending=0 "
    b"passed_bitmap=0x2003ffff failed_bitmap=0x0 pending_bitmap=0x0\n"
)


class BootResults(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="charlotte-boot-result-")
        self.addCleanup(self.directory.cleanup)
        self.log = Path(self.directory.name) / "serial.log"

    def check(self, data, function="catten_boot_has_selftest_result", success=False):
        self.log.write_bytes(data)
        return subprocess.run(
            ["bash", "-c", 'source "$1/scripts/lib/boot-common.sh"; "$2" "$3" "$4"',
             "boot-test", str(ROOT), function, str(self.log), "1" if success else ""],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
        ).returncode

    def test_every_partial_prefix_is_rejected(self):
        for length in range(len(SUCCESS)):
            with self.subTest(length=length):
                self.assertNotEqual(self.check(SUCCESS[:length]), 0)
        self.assertEqual(self.check(SUCCESS), 0)

    def test_lf_crlf_and_subsequent_partial_line(self):
        for data in (SUCCESS, SUCCESS.replace(b"\n", b"\r\n"), SUCCESS + b"[thread] partial"):
            self.assertEqual(self.check(data), 0)
            self.assertEqual(self.check(data, success=True), 0)

    def test_complete_failure_is_terminal_but_not_success(self):
        failed = SUCCESS.replace(b"failed=0", b"failed=1").replace(b"failed_bitmap=0x0", b"failed_bitmap=0x1")
        pending = SUCCESS.replace(b"pending=0", b"pending=2").replace(b"pending_bitmap=0x0", b"pending_bitmap=0x3")
        for data in (failed, pending):
            self.assertEqual(self.check(data), 0)
            self.assertNotEqual(self.check(data, success=True), 0)

    def test_malformed_and_truncated_results_fail_validation(self):
        for data in (SUCCESS[:-1], SUCCESS.replace(b"pending_bitmap=", b"other_bitmap="),
                     SUCCESS.replace(b"pending_bitmap=0x0", b"pending_bitmap=0x1")):
            self.assertNotEqual(self.check(data, "catten_boot_validate_selftest_log"), 0)

    def test_panic_rejects_an_otherwise_complete_result(self):
        self.assertNotEqual(self.check(SUCCESS + b"Kernel panic:\n", "catten_boot_validate_selftest_log"), 0)
        self.assertEqual(self.check(SUCCESS, "catten_boot_validate_selftest_log"), 0)


if __name__ == "__main__":
    unittest.main()
