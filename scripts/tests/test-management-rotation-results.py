#!/usr/bin/env python3
"""Rotation observations require exact installed state and terminal causes."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("rotation", ROOT / "scripts/run-management-rotation-test.py")
rotation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rotation)


def success(revision):
    return (f"[boot trust fixture] verified signed module: revision={revision}\n"
            f"[boot trust] admission policy published: revision={revision}\n"
            "[launch] steady-state service set published.\n[boot trust] signed handoff passed\n"
            "[time] synchronized utc\n[deployd] listening port=7444\n"
            "SELFTEST COMPLETE: passed=3 failed=0 pending=0 passed_bitmap=0x7 failed_bitmap=0x0 pending_bitmap=0x0\n")


class RotationResults(unittest.TestCase):
    def test_exact_installed_revision_and_authoritative_suite(self):
        for revision in (7, 8):
            self.assertTrue(rotation.snapshot(success(revision), revision))
            with self.assertRaises(rotation.secure.TestFailure):
                rotation.snapshot(success(revision), revision + 1)
        for text in (success(8) + "Kernel panic:\n", success(8).replace("passed_bitmap=0x7", "passed_bitmap=0x3"),
                     success(8) + "[boot trust] admission policy published: revision=8\n",
                     success(8).replace("[boot trust] signed handoff passed\n", "")):
            with self.subTest(text=text), self.assertRaises(rotation.secure.TestFailure):
                rotation.snapshot(text, 8)
        self.assertFalse(rotation.snapshot("", 8))
        with self.assertRaises(rotation.secure.TestFailure):
            rotation.snapshot("[net] startup rejected stage=3 error=RequiredFeatures detail=0x101\n", 8)
        with self.assertRaises(rotation.secure.TestFailure):
            rotation.snapshot(success(8).replace("failed=0", "failed=1"), 8)

    def test_rejection_requires_exact_cause_before_publication(self):
        for cause in ("UninstalledRevision", "RevisionRollback", "WrongPolicy"):
            line = ("[launch] management TLS resource rejected before trust publication: WrongPolicy"
                    if cause == "WrongPolicy" else "[boot trust fixture] verification rejected: " + cause)
            text = "Kernel panic:\n" + line + "\n"
            if cause == "WrongPolicy":
                with self.assertRaises(rotation.secure.TestFailure):
                    rotation.snapshot(text, 8, cause)
                text += "boot admission trust rejected\n"
            self.assertTrue(rotation.snapshot(text, 8, cause))
            self.assertFalse(rotation.snapshot(line + "\n", 8, cause))
            for incorrect in (text.replace(cause, cause + "Unexpected"), "Kernel panic:\nunrelated\n",
                              text + "[boot trust] admission policy published: revision=8\n",
                              text + "[launch] steady-state service set published.\n"):
                with self.subTest(cause=cause, incorrect=incorrect), self.assertRaises(rotation.secure.TestFailure):
                    rotation.snapshot(incorrect, 8, cause)
            if cause != "WrongPolicy":
                with self.assertRaises(rotation.secure.TestFailure):
                    rotation.snapshot(text + "[boot trust] artifact authority selected before platform services\n", 8, cause)

    def test_public_fixture_commit_precedes_readback_and_failed_replace_preserves_state(self):
        source = ROOT / "crates/catten/src/service/admission"
        with tempfile.TemporaryDirectory() as temporary:
            state = Path(temporary) / "installed.snapshot"
            rotation.commit_fixture_snapshot(state, source / "test-policy.bin")
            old = state.read_bytes()
            with patch.object(rotation.os, "replace", side_effect=OSError("interrupted before commit")):
                with self.assertRaises(OSError):
                    rotation.commit_fixture_snapshot(state, source / "test-policy-next.bin")
            self.assertEqual(state.read_bytes(), old)
            state.with_suffix(".pending").unlink()
            rotation.commit_fixture_snapshot(state, source / "test-policy-next.bin")
            self.assertEqual(state.read_bytes(), (source / "test-policy-next.bin").read_bytes())
            self.assertEqual(state.stat().st_mode & 0o777, 0o600)


if __name__ == "__main__":
    unittest.main()
