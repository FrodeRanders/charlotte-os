#!/usr/bin/env python3
"""Vote diagnostics expose bounded scalar metadata, not application bodies."""
import importlib.util
from pathlib import Path
import sys
import unittest

spec = importlib.util.spec_from_file_location("qemu_hub", Path(__file__).resolve().parents[1] / "qemu-stream-l2-hub.py")
hub = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = hub
spec.loader.exec_module(hub)


def frame(tag, body):
    return bytes.fromhex("52540012340152540012340288b7") + bytes([tag]) + len(body).to_bytes(2, "big") + body


class VoteTrace(unittest.TestCase):
    def test_padded_vote_request_reports_only_scalar_metadata(self):
        value = hub.vote_summary(frame(1, b"\x08\x05\x12\x04node\x18\x0a\x20\x04") + bytes(20))
        self.assertEqual(value, {"source": "52:54:00:12:34:02", "destination": "52:54:00:12:34:01", "tag": 1, "term": 5, "granted": None})

    def test_vote_response_honors_both_term_fields_and_default_false(self):
        self.assertEqual(hub.vote_summary(frame(4, b"\x0a\x04node\x10\x05\x18\x01\x20\x06"))["term"], 6)
        self.assertTrue(hub.vote_summary(frame(4, b"\x10\x05\x18\x01"))["granted"])
        self.assertFalse(hub.vote_summary(frame(4, b"\x10\x05"))["granted"])

    def test_other_protocols_truncation_and_oversized_varints_do_not_trace(self):
        for value in (b"", frame(2, b"secret application data"), frame(1, b"\x08\x05")[:-1],
                      frame(1, b"\x08" + b"\x80" * 11), frame(1, b"\x08" + b"\xff" * 9 + b"\x02"),
                      frame(1, b"\x12\x10short")):
            self.assertIsNone(hub.vote_summary(value))


if __name__ == "__main__":
    unittest.main()
