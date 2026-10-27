#!/usr/bin/env python3
"""Date rejection requires the exact new native episode, never transport silence."""
import importlib.util
import json
import os
from pathlib import Path
import ssl
import tempfile
import unittest
from unittest.mock import MagicMock, patch

import management_tls

spec = importlib.util.spec_from_file_location("validity", Path(__file__).with_name("test-management-validity.py"))
validity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(validity)


class ValidityObservations(unittest.TestCase):
    def setUp(self):
        self.owner = tempfile.TemporaryDirectory()
        self.addCleanup(self.owner.cleanup)
        self.directory = Path(self.owner.name)
        self.log = self.directory / "serial"
        self.log.write_text("")
        self.rows = [{"name": category + "-" + role, "category": category, "role": role}
                     for category in validity.EVENTS for role in ("reader", "administrator")]
        (self.directory / "validity.json").write_text(json.dumps(self.rows))
        self.patchers = [patch.dict(os.environ, CATTEN_MANAGEMENT_VALIDITY_DIR=str(self.directory), CATTEN_MANAGEMENT_CA_FILE="unused"),
                         patch.object(validity, "context", return_value=MagicMock()),
                         patch.object(validity.ssl, "create_default_context", return_value=MagicMock())]
        for patcher in self.patchers:
            patcher.start()
            self.addCleanup(patcher.stop)

    def event(self, identity, category):
        with self.log.open("a") as out:
            out.write(f"[+ 1.000] [EL0 LOG] lp=0 asid=133 [management] connection={identity} {validity.EVENTS[category]}\n")

    def test_event_parser_rejects_suffixes_and_distinguishes_episode_and_category(self):
        self.event(1, "expired")
        self.event(2, "future")
        text = self.log.read_text()
        self.assertEqual(validity.events(text, "expired"), {(133, 1)})
        self.assertEqual(validity.events(text, "future"), {(133, 2)})
        self.assertEqual(validity.events(text.replace("certificate expired", "certificate expiredSuffix"), "expired"), set())
        self.assertEqual(validity.events(text.replace("\n", "\r\n"), "expired"), {(133, 1)})

    def test_all_eight_controls_need_new_events_and_healthy_recovery(self):
        # Per port: baseline, then one failed date control + one healthy reader
        # for each of the four distinct certificate/role combinations.
        count = 0
        def exchange(_policy, port, _request, _expected, _budget):
            nonlocal count
            local = count % 9
            count += 1
            if local in (1, 3, 5, 7):
                row = self.rows[(local - 1) // 2]
                self.event(count, row["category"])
                raise EOFError()
            return {"status": 200 if port == 1 else 404}, None
        with patch.object(validity, "exchange", side_effect=exchange), patch("builtins.print"):
            observations = validity.probe(self.log, 1, 2)
        self.assertEqual(len(observations), 8)
        self.assertEqual(count, 18)

    def test_transport_failure_without_new_matching_event_fails(self):
        for category in ("stale-expired", "stale-future", "new-future", "new-policy"):
            self.log.write_text("")
            if category.startswith("stale-"):
                self.event(1, category.removeprefix("stale-"))
            calls = 0
            def exchange(*_arguments):
                nonlocal calls
                calls += 1
                if calls == 1:
                    return {}, None
                if category == "new-future":
                    self.event(2, "future")
                elif category == "new-policy":
                    with self.log.open("a") as out:
                        out.write("[EL0 LOG] lp=0 asid=133 [management] connection=2 client policy rejected\n")
                raise EOFError()
            with self.subTest(category=category), patch.object(validity, "exchange", side_effect=exchange), self.assertRaisesRegex(validity.ProbeFailure, "lacked its exact native event"):
                validity.probe(self.log, 1, 2, budget=0.001)

    def test_server_verification_and_timeout_cannot_qualify_client_rejection(self):
        for error in (ssl.SSLCertVerificationError("wrong server"), TimeoutError("silent peer")):
            with self.subTest(error=error), patch.object(validity, "exchange", side_effect=[({}, None), error]):
                with self.assertRaises((validity.ProbeFailure, TimeoutError)):
                    validity.probe(self.log, 1, 2, budget=0.001)

    def test_missing_or_duplicate_role_date_combination_rejects_before_network(self):
        for records in (self.rows[:-1], self.rows + [self.rows[0]]):
            (self.directory / "validity.json").write_text(json.dumps(records))
            with patch.object(validity, "exchange") as exchange, self.assertRaises(validity.ProbeFailure):
                validity.probe(self.log, 1, 2)
            exchange.assert_not_called()

    def test_rejection_disallows_even_an_incomplete_application_response(self):
        raw, client, policy = MagicMock(), MagicMock(), MagicMock()
        client.__enter__.return_value = client
        client.version.return_value = "TLSv1.3"
        client.cipher.return_value = ("TLS_AES_128_GCM_SHA256", "TLSv1.3", 128)
        policy.wrap_socket.return_value = client
        for data in (b"H", b"HTTP/1.1 403 Forbidden\r\n"):
            client.recv.side_effect = [data, b""]
            with self.subTest(data=data), patch.object(management_tls.socket, "create_connection", return_value=raw):
                with self.assertRaisesRegex(validity.ProbeFailure, "received application bytes"):
                    management_tls.exchange(policy, 1, b"request", None, 1)


if __name__ == "__main__":
    unittest.main()
