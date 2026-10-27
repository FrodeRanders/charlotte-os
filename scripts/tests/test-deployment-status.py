#!/usr/bin/env python3
"""Check complete named deployment observations through the real status CLI."""
import argparse
import management_tls_fixture
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
from pathlib import Path
import subprocess
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[2]
SIGNER = ROOT / "target/debug/cluster-sign"
RECORDS = []
READY = ('{"state":"ready","deployment_generation":2,"service_generation":7,'
         '"node_key":1,"desired_replicas":1,"ready_replicas":1}')
COMMITTED = READY.replace('"ready"', '"committed"').replace('"ready_replicas":1', '"ready_replicas":0')
REPLACING = READY.replace('"ready"', '"replacing"').replace('"desired_replicas":1', '"desired_replicas":2')


class Status(unittest.TestCase):
    def observe(self, rows, wait="0", name="app"):
        events = []

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def do_GET(self):
                index = len(events)
                events.append(("GET", self.path))
                code, body = rows[min(index, len(rows)-1)]
                data = body.encode()
                self.send_response(code)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(data)

            def do_POST(self):
                events.append(("POST", self.path))
                self.send_error(405)

            def log_message(self, *_):
                pass

        with HTTPServer(("127.0.0.1", 0), Handler) as server:
            management_tls_fixture.install(server)
            thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01})
            thread.start()
            start = time.monotonic()
            try:
                args = [str(SIGNER), "deployment-status", name, f"127.0.0.1:{server.server_port}"]
                if wait is not None:
                    args.append(wait)
                result = subprocess.run(args, capture_output=True, timeout=6)
                elapsed = time.monotonic()-start
            finally:
                server.shutdown()
                thread.join(timeout=2)
            self.assertFalse(thread.is_alive())
        self.assertTrue(all(kind == "GET" for kind, _ in events), "status must never submit a mutation")
        RECORDS.append({"case": self.id().split(".")[-1], "wait": wait, "name": name,
                        "rows": rows, "events": events, "elapsed_seconds": elapsed,
                        "exit_status": result.returncode, "stdout": result.stdout.decode(),
                        "stderr": result.stderr.decode()})
        return result, elapsed, events

    def test_read_only_accepts_all_current_states(self):
        for body in [READY, COMMITTED, REPLACING]:
            with self.subTest(body=body):
                result, _, events = self.observe([(200, body)])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.decode().strip(), body)
                self.assertEqual(len(events), 1)

    def test_ready_wait_accepts_json_whitespace_and_field_order(self):
        ordered = dict(reversed(list(json.loads(READY).items())))
        for body in [READY, json.dumps(ordered, indent=2)]:
            with self.subTest(body=body):
                result, _, events = self.observe([(200, body)], wait="1")
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(len(events), 1)

    def test_pending_then_ready_observes_current_name(self):
        result, _, events = self.observe([(200, COMMITTED), (200, READY)], wait="2")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(events), 2)

    def test_malformed_200_stops_without_another_observation(self):
        malformed = [
            '{"state":"ready"}',
            READY.replace('"deployment_generation":2', '"deployment_generation":2,"deployment_generation":3'),
            READY.replace('"ready_replicas":1', '"ready_replicas":0'),
            READY.replace('"node_key":1', '"extra":1'),
            READY.replace('"state":"ready"', '"state":{"state":"ready"}'),
            READY.replace('"deployment_generation":2', '"deployment_generation":0'),
            READY+" trailing",
            READY+'\r\n\r\n{"ignored":true}',
            '\u00a0'+READY+'\u00a0',
        ]
        for wait in ["0", "1"]:
            for body in malformed:
                with self.subTest(wait=wait, body=body):
                    result, _, events = self.observe([(200, body), (200, READY)], wait=wait)
                    self.assertNotEqual(result.returncode, 0, result.stdout)
                    self.assertIn(b"invalid rollout status", result.stderr)
                    self.assertEqual(result.stdout, b"")
                    self.assertEqual(len(events), 1)

    def test_missing_wait_is_one_checked_query(self):
        result, _, events = self.observe([(200, COMMITTED)], wait=None)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(events), 1)

    def test_read_only_http_error_is_reported(self):
        result, _, events = self.observe([(404, '{"error":"deployment not found"}')])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(events), 1)

    def test_invalid_name_rejects_before_waiting_or_network(self):
        result, elapsed, events = self.observe([(200, READY)], wait="1", name="bad name")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"invalid deployment artifact name", result.stderr)
        self.assertLess(elapsed, 0.6)
        self.assertEqual(events, [])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--signer", type=Path, default=SIGNER)
    parser.add_argument("--evidence", type=Path)
    options, remaining = parser.parse_known_args()
    SIGNER = options.signer.resolve()
    outcome = unittest.main(argv=[__file__, *remaining], exit=False)
    if options.evidence:
        options.evidence.write_text(json.dumps(RECORDS, indent=2)+"\n")
    raise SystemExit(0 if outcome.result.wasSuccessful() else 1)
