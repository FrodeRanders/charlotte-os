#!/usr/bin/env python3
"""Check real ingress CLI deadlines with a bounded, owned loopback peer."""
import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
from pathlib import Path
import subprocess
import threading
import time
import unittest

spec = importlib.util.spec_from_file_location("deployment_generation", Path(__file__).with_name("test-deployment-generation.py"))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
release_spec = importlib.util.spec_from_file_location("release_generation", Path(__file__).with_name("test-release-generation.py"))
release_fixture = importlib.util.module_from_spec(release_spec)
release_spec.loader.exec_module(release_fixture)


RECORDS = []


class Deadline(unittest.TestCase):
    prepare = fixture.Generation.prepare

    def setUp(self):
        fixture.Generation.setUp(self)
        second = self.prepare("app2")
        self.descriptors = [self.descriptor, second]
        self.release = self.root / "release.crelease"
        result = subprocess.run([str(fixture.SIGNER), "release-sign", str(self.release), "batch", "1",
                                 str(self.key), *map(str, self.descriptors)], capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)

    def peer(self, mode, command="release", timeout="1", hostname=False):
        events = []
        stop = threading.Event()

        class Server(ThreadingHTTPServer):
            daemon_threads = False
            block_on_close = True

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def reply(self, body, delay=0, trickle=False):
                if stop.wait(delay):
                    return
                data = body.encode()
                try:
                    self.send_response(202 if self.command == "POST" else 200)
                    self.send_header("Content-Length", str(len(data)))
                    self.send_header("Connection", "close")
                    self.end_headers()
                    if trickle:
                        for byte in data:
                            self.wfile.write(bytes([byte]))
                            self.wfile.flush()
                            if stop.wait(0.1):
                                break
                    else:
                        self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def do_POST(self):
                self.connection.settimeout(3)
                data = self.rfile.read(int(self.headers["Content-Length"]))
                events.append(("POST", self.path))
                ack = (release_fixture.admission(data, (2, 8)) if self.path == "/v1/releases"
                       else '{"generation":2}')
                self.reply(ack, delay=0.7 if mode in ["shared", "poll"] else 0.6 if mode == "batch" else 0)

            def do_GET(self):
                events.append(("GET", self.path))
                generation = 8 if self.path.endswith("app2") else 2
                if mode == "poll":
                    generation = 1
                self.reply(fixture.status(generation), delay=0.7 if mode == "shared" else 0,
                           trickle=mode == "trickle")

            def log_message(self, *_):
                pass

        with Server(("127.0.0.1", 0), Handler) as server:
            thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01})
            thread.start()
            endpoint = f'{"localhost" if hostname else "127.0.0.1"}:{server.server_port}'
            args = (["release-apply", str(self.release), endpoint, timeout] if command == "release" else
                    ["deployment-apply", endpoint, timeout, *map(str, self.descriptors)] if command == "batch" else
                    ["deployment-status", "app", endpoint, timeout])
            start = time.monotonic()
            try:
                result = subprocess.run([str(fixture.SIGNER), *args], capture_output=True, timeout=13)
                elapsed = time.monotonic() - start
            finally:
                stop.set()
                server.shutdown()
                thread.join(timeout=2)
            self.assertFalse(thread.is_alive())
        RECORDS.append({"case": self.id().split(".")[-1], "command": command,
                        "timeout_seconds": timeout, "hostname": hostname,
                        "exit_status": result.returncode, "elapsed_seconds": elapsed,
                        "events": events, "stdout": result.stdout.decode(), "stderr": result.stderr.decode()})
        return result, elapsed, events

    def test_exact_receipt_and_ready_controls_succeed(self):
        result, _, events = self.peer("immediate")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(sum(kind == "POST" for kind, _ in events), 1)

    def test_admission_and_readiness_share_one_budget(self):
        result, elapsed, events = self.peer("shared")
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertLess(elapsed, 1.6)
        self.assertEqual(sum(kind == "POST" for kind, _ in events), 1)

    def test_trickled_reply_cannot_restart_remaining_budget(self):
        result, elapsed, events = self.peer("trickle")
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertLess(elapsed, 1.6)
        self.assertEqual(sum(kind == "POST" for kind, _ in events), 1)

    def test_batch_notifications_share_one_budget(self):
        result, elapsed, events = self.peer("batch", command="batch")
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertLess(elapsed, 1.6)
        self.assertEqual(sum(kind == "POST" for kind, _ in events), 2)
        self.assertFalse(any(kind == "GET" for kind, _ in events))

    def test_poll_pause_uses_remaining_budget(self):
        result, elapsed, events = self.peer("poll")
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertLess(elapsed, 1.6)
        self.assertEqual(sum(kind == "POST" for kind, _ in events), 1)

    def test_numeric_endpoint_rejects_hostname_before_network(self):
        result, _, events = self.peer("immediate", hostname=True)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(b"numeric", result.stderr)
        self.assertEqual(events, [])

    def test_extreme_timeout_is_rejected_without_panic_or_network(self):
        for command in ["release", "batch", "status"]:
            with self.subTest(command=command):
                result, _, events = self.peer("immediate", command=command, timeout=str(2**64-1))
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertNotIn(b"panicked", result.stderr)
                self.assertIn(b"clock range", result.stderr)
                self.assertEqual(events, [])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--signer", type=Path, default=fixture.SIGNER)
    parser.add_argument("--evidence", type=Path)
    options, remaining = parser.parse_known_args()
    fixture.SIGNER = options.signer.resolve()
    outcome = unittest.main(argv=[__file__, *remaining], exit=False)
    if options.evidence:
        options.evidence.write_text(json.dumps(RECORDS, indent=2)+"\n")
    raise SystemExit(0 if outcome.result.wasSuccessful() else 1)
