#!/usr/bin/env python3
"""Negative boot evidence cannot be inferred from silence or unrelated failure."""

import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("secure_boot", ROOT / "scripts/run-secure-boot-test.py")
secure = importlib.util.module_from_spec(spec)
spec.loader.exec_module(secure)
SUCCESS = (
    b"[boot trust fixture] verified signed module: revision=7\n"
    b"[boot trust] artifact authority selected before platform services\n"
    b"[node] name service started (asid=1, tid=1)\n"
    b"[boot trust] signed handoff passed\n"
    b"SELFTEST COMPLETE: passed=15 failed=0 pending=0 passed_bitmap=0x1bbff "
    b"failed_bitmap=0x0 pending_bitmap=0x0\n"
)


class SecureBootResults(unittest.TestCase):
    def test_success_requires_complete_result_and_signed_handoff(self):
        for length in range(len(SUCCESS)):
            self.assertFalse(secure.observed_result(SUCCESS[:length], "success"))
        self.assertTrue(secure.observed_result(SUCCESS, "success"))
        self.assertTrue(secure.observed_result(SUCCESS.replace(b"\n", b"\r\n"), "success"))
        for missing in (b"[boot trust] signed handoff passed\n", b"[boot trust fixture] verified signed module: revision=7\n",
                        b"[boot trust] artifact authority selected before platform services\n",
                        b"[node] name service started (asid=1, tid=1)\n"):
            with self.assertRaises(secure.TestFailure):
                secure.observed_result(SUCCESS.replace(missing, b""), "success")

    def test_silence_and_unrelated_errors_are_never_rejection_evidence(self):
        for expectation in secure.CASES.values():
            self.assertFalse(secure.observed_result(b"", expectation))
            self.assertFalse(secure.observed_result(b"QEMU failed to start\n", expectation))
        for expectation in ("InvalidSignature", "RevisionRollback", "UninstalledRevision"):
            with self.assertRaises(secure.TestFailure):
                secure.observed_result(b"Kernel panic:\nunrelated allocator failure\n", expectation)

    def test_firmware_and_loader_failures_cannot_reach_kernel(self):
        evidence = {
            "firmware": b'BdsDxe: failed to load Boot0001 "UEFI QEMU USB HARDDRIVE": Security Violation\n',
            "config": b"!!! CHECKSUM MISMATCH FOR CONFIG FILE !!!\n",
            "kernel-hash": b"PANIC: Blake2b hash for URI `boot():/catten` does not match!\n",
            "policy-hash": b"PANIC: Blake2b hash for URI `boot():/boot-policy.bin` does not match!\n",
        }
        for expectation, message in evidence.items():
            self.assertTrue(secure.observed_result(message, expectation))
            self.assertTrue(secure.observed_result(b"\x1b[31m" + message, expectation))
            with self.assertRaises(secure.TestFailure):
                secure.observed_result(message + b"Catten Kernel Version 0.8.1\n", expectation)
        self.assertFalse(secure.observed_result(evidence["policy-hash"], "kernel-hash"))
        self.assertTrue(secure.observed_result(evidence["firmware"].replace(b"Security Violation", b"Access Denied"), "firmware"))
        self.assertFalse(secure.observed_result(b"Security Violation\n", "firmware"))

    def test_inner_policy_rejection_must_precede_publication(self):
        message = b"Kernel panic:\n[boot trust fixture] verification rejected: RevisionRollback\n"
        self.assertTrue(secure.observed_result(message, "RevisionRollback"))
        for published in (b"[launch] steady-state service set published.\n", b"[boot trust fixture] verified signed module\n"):
            with self.assertRaises(secure.TestFailure):
                secure.observed_result(message + published, "RevisionRollback")
        with self.assertRaises(secure.TestFailure):
            secure.observed_result(SUCCESS + b"Kernel panic:\n", "success")

    def test_successor_requires_the_shared_exact_boot_verifier_rejection(self):
        expected = secure.CASES["policy-next"]
        self.assertEqual(expected, "UninstalledRevision")
        message = b"Kernel panic:\n[boot trust fixture] verification rejected: UninstalledRevision\n"
        self.assertTrue(secure.observed_result(message, expected))
        with self.assertRaises(secure.TestFailure):
            secure.observed_result(message.replace(b"UninstalledRevision", b"uninstalled policy revision"), expected)
        for published in (b"[launch] steady-state service set published.\n", b"[boot trust fixture] verified signed module\n"):
            with self.assertRaises(secure.TestFailure):
                secure.observed_result(message + published, expected)

    def test_platform_rejection_requires_the_selected_authority_and_exact_gate(self):
        prefix = (b"[boot trust fixture] verified signed module: revision=7\n"
                  b"[boot trust] artifact authority selected before platform services\n")
        for case, gate in (("platform-ns-root", "name-service"), ("platform-driver-root", "domain")):
            message = prefix + f"Kernel panic:\n[loader] {gate} load failed: SignatureVerificationFailed\n".encode()
            self.assertTrue(secure.observed_result(message, case))
            for invalid in (message.replace(prefix, b""), message.replace(b"SignatureVerificationFailed", b"PageMapping"),
                            message + b"[launch] steady-state service set published.\n"):
                with self.assertRaises(secure.TestFailure):
                    secure.observed_result(invalid, case)
        with self.assertRaises(secure.TestFailure):
            secure.observed_result(message.replace(b"domain", b"name-service")
                                   + b"[node] name service started\n", "platform-ns-root")


if __name__ == "__main__":
    unittest.main()
