#!/usr/bin/env python3
"""A native expiry proof needs the same admitted episode and zero application bytes."""
import importlib.util
from pathlib import Path
import ssl
import sys
import tempfile
import time
import unittest
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
spec = importlib.util.spec_from_file_location("chain_probe", Path(__file__).with_name("test-management-chain-validity.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


class ChainObservations(unittest.TestCase):
    def setUp(self):
        owner = tempfile.TemporaryDirectory(); self.addCleanup(owner.cleanup)
        self.directory = Path(owner.name)
        (self.directory / "session-administrator.der").write_bytes(b"public fixture leaf")
        self.log = self.directory / "serial"; self.log.write_text("")
        self.peer = MagicMock(); self.peer.__enter__.return_value = self.peer
        self.peer.version.return_value = "TLSv1.3"
        self.peer.cipher.return_value = ("TLS_AES_128_GCM_SHA256", "TLSv1.3", 128)
        self.peer.session_reused = False
        self.peer.recv.return_value = b""
        self.policy = MagicMock(); self.policy.wrap_socket.return_value = self.peer
        self.raw = MagicMock(); self.raw.__enter__.return_value = self.raw
        patcher = patch.object(probe.socket, "create_connection", return_value=self.raw)
        patcher.start(); self.addCleanup(patcher.stop)

    def run_upload(self, observation=None):
        with patch.object(probe, "authority_times", return_value={(13,4): int(time.time())}):
            return probe.held_upload(self.policy, self.directory, 7444, b"signed-body", self.log,
                                 time.monotonic()+1, {"not_after_unix_seconds": int(time.time())-2},
                                 lambda _end: observation or {"source_asid":13,"credential_utc":int(time.time())})

    def test_only_exact_admission_episode_and_hash_match(self):
        digest = "a"*64
        line = f"[EL0 LOG] lp=0 asid=13 [management] connection=4 certificate={digest} role=Administrator permission=Mutate allowed=true credential_utc=123\n"
        self.assertEqual(probe.admissions(line, digest), {(13, 4)})
        self.assertEqual(probe.admissions(line.replace("true", "trueSuffix"), digest), set())
        self.assertEqual(probe.admissions(line, "b"*64), set())
        self.assertEqual(probe.admissions(line.replace("Mutate", "Observe"), digest), set())
        self.assertEqual(probe.authority_times(line, digest, "Administrator", "Mutate"), {(13,4):123})
        self.assertEqual(probe.authority_times(line.replace("credential_utc=123", "credential_utc=bad"), digest, "Administrator", "Mutate"), {})

    def test_host_time_or_another_factory_cannot_prove_native_expiry(self):
        for observation in ({"source_asid":14,"credential_utc":int(time.time())},
                            {"source_asid":13,"credential_utc":int(time.time())-3}):
            with self.subTest(observation=observation), patch.object(probe, "wait_event", return_value=(13,4)):
                with self.assertRaisesRegex(probe.ProbeFailure, "native credential clock"):
                    self.run_upload(observation)
        self.assertTrue(all(len(call.args[0]) > 1 for call in self.peer.sendall.call_args_list))

    def test_matching_native_expiry_and_eof_qualify_only_transport_observation(self):
        with patch.object(probe, "wait_event", side_effect=[(13, 4), (13, 4)]):
            value = self.run_upload()
        self.assertEqual(value["application_bytes"], 0)
        self.assertTrue(value["final_body_byte_submitted"])
        self.assertEqual(value["source_asid"], 13)
        self.assertEqual(value["connection"], 4)

    def test_partial_or_complete_application_bytes_never_qualify(self):
        for data in (b"H", b"HTTP/1.1 400 Bad Request\r\n\r\n"):
            self.peer.recv.return_value = data
            with self.subTest(data=data), patch.object(probe, "wait_event", return_value=(13,4)):
                with self.assertRaisesRegex(probe.ProbeFailure, "application bytes"):
                    self.run_upload()

    def test_another_episode_cannot_supply_expiry_proof(self):
        with patch.object(probe, "wait_event", side_effect=[(13, 4), (13, 5)]):
            with self.assertRaisesRegex(probe.ProbeFailure, "did not match"):
                self.run_upload()

    def test_timeout_and_server_verification_failure_never_qualify(self):
        with patch.object(probe, "wait_event", return_value=(13,4)):
            self.peer.recv.side_effect = TimeoutError("silent peer")
            with self.assertRaises(TimeoutError): self.run_upload()
        self.peer.recv.side_effect = None
        self.policy.wrap_socket.side_effect = ssl.SSLCertVerificationError("wrong server")
        with self.assertRaises(ssl.SSLCertVerificationError): self.run_upload()

    def test_missing_ambiguous_or_stale_events_do_not_create_a_new_episode(self):
        read = lambda _text: {(13,4)}
        with self.assertRaisesRegex(probe.ProbeFailure,"lacked"):
            probe.wait_event(self.log, read, {(13,4)}, time.monotonic())
        with self.assertRaisesRegex(probe.ProbeFailure,"ambiguous"):
            probe.wait_event(self.log, lambda _: {(13,4),(13,5)}, set(), time.monotonic()+1)


if __name__ == "__main__":
    unittest.main()
