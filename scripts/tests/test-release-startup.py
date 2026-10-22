#!/usr/bin/env python3
"""Exercise release startup admission and uncertain outcomes through the real CLI."""
import argparse
from http.server import BaseHTTPRequestHandler, HTTPServer
import importlib.util
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import threading
import time
import unittest

spec = importlib.util.spec_from_file_location(
    "release_generation", Path(__file__).with_name("test-release-generation.py"))
release_fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release_fixture)
fixture = release_fixture.fixture
RECORDS = []


class Startup(unittest.TestCase):
    prepare = fixture.Generation.prepare
    setUp = release_fixture.ReleaseGeneration.setUp

    def peer(self, mode="ready", rejection=(409, '{"error":-23}'), delayed=False, timeout=2, runner=None):
        events = []
        stop = threading.Event()
        active = threading.Event()
        refused = threading.Event()
        envelope = self.release.read_bytes()

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def reply(self, code, body):
                data = body.encode()
                try:
                    self.send_response(code)
                    self.send_header("Content-Length", str(len(data)))
                    self.send_header("Connection", "close")
                    self.end_headers()
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass

            def do_POST(self):
                self.connection.settimeout(3)
                data = self.rfile.read(int(self.headers["Content-Length"]))
                events.append(["POST", self.path, data == envelope])
                posts = sum(row[0] == "POST" for row in events)
                if (mode in ("reject-once", "reject-then-pending") and posts == 1) or mode == "reject":
                    self.reply(*rejection)
                elif mode == "drop":
                    self.close_connection = True
                elif mode == "stall":
                    stop.wait(4)
                elif mode == "truncated":
                    self.wfile.write(b'HTTP/1.1 202 Accepted\r\nContent-Length: 20\r\n\r\n{}')
                    self.wfile.flush()
                    self.close_connection = True
                elif mode == "malformed":
                    self.reply(202, '{"generation":1}')
                else:
                    self.reply(202, release_fixture.admission(data))

            def do_GET(self):
                events.append(["GET", self.path])
                generation = 8 if self.path.endswith("app2") else 2
                if mode in ("pending", "reject-then-pending"):
                    generation -= 1
                if mode == "superseded":
                    generation += 1
                self.reply(200, "{}" if mode == "bad-status" else fixture.status(generation))

            def log_message(self, *_):
                pass

        # Reserve an ephemeral port, then leave it closed until the client
        # reports refusal. A bound, non-listening socket may time out on macOS.
        with HTTPServer(("127.0.0.1", 0), Handler, bind_and_activate=False) as server:
            server.server_bind()
            if delayed:
                server.server_address = ("127.0.0.1", server.server_port)
                server.socket.close()
                server.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)

            def serve():
                if delayed and not refused.wait(timeout + 2):
                    return
                if delayed:
                    server.server_bind()
                server.server_activate()
                active.set()
                server.serve_forever(poll_interval=0.01)

            thread = threading.Thread(target=serve)
            thread.start()
            if not delayed:
                self.assertTrue(active.wait(2))
            endpoint = f"127.0.0.1:{server.server_port}"
            start = time.monotonic()
            try:
                command = [str(fixture.SIGNER), "release-apply", str(self.release), endpoint, str(timeout)]
                environment = None
                if runner:
                    # Execute the real setup and launch blocks from each boot
                    # runner against this peer, without booting another guest.
                    source = (fixture.ROOT / "scripts" / runner).read_text()
                    setup_start = source.index('        DEPLOYMENT_RESULT_FILE="${DEPLOYMENT_TEST_DIR}/result"')
                    setup_end = source.index('\n    fi', setup_start)
                    launch_start = source.index('        # QEMU host forwarding can accept TCP')
                    launch_end = source.index('        if [ "$DEPLOYMENT_INGRESS_TEST" = "1" ] && [ -f "$DEPLOYMENT_RESULT_FILE" ]', launch_start)
                    launch = source[launch_start:launch_end]
                    directory = self.root / runner
                    directory.mkdir(exist_ok=True)
                    log = directory / "serial.log"
                    log.write_text("")
                    harness = directory / "worker.sh"
                    harness.write_text('set -eu\nDEPLOYMENT_WORKER_PID=""\n' +
                                       source[setup_start:setup_end] + '\n' + launch +
                                       '\ntest -z "$DEPLOYMENT_WORKER_PID"\n' +
                                       'printf "%s\\n" "[deployd] listening port=7444" >"$LOG"\n' +
                                       launch + '\ntest -z "$DEPLOYMENT_WORKER_PID"\n' +
                                       'printf "%s\\n" "[dns] RAFT STATE state=Leader" >>"$LOG"\n' +
                                       launch + '\nwait "$DEPLOYMENT_WORKER_PID"\n' +
                                       'test -f "$DEPLOYMENT_RESULT_FILE"\n')
                    environment = dict(os.environ, DEPLOYMENT_INGRESS_TEST="1", TIMEOUT=str(timeout + 5),
                                       DEPLOYMENT_TEST_DIR=str(directory), LOG=str(log),
                                       CLUSTER_SIGN_BIN=str(fixture.SIGNER), DEPLOYMENT_RELEASE=str(self.release),
                                       DEPLOY_HOST_PORT=str(server.server_port))
                    command = ["bash", str(harness)]
                if delayed:
                    with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0) as process:
                        try:
                            self.assertTrue(select.select([process.stderr], [], [], timeout + 2)[0])
                            first = process.stderr.readline()
                            self.assertIn(b"release startup retry: connect", first)
                            refused.set()
                            stdout, stderr = process.communicate(timeout=timeout + 4)
                            result = subprocess.CompletedProcess(command, process.returncode, stdout, first + stderr)
                        finally:
                            refused.set()
                            if process.poll() is None:
                                process.kill()
                                process.communicate()
                else:
                    result = subprocess.run(command, env=environment, capture_output=True, timeout=timeout + 4)
                elapsed = time.monotonic() - start
            finally:
                stop.set()
                refused.set()
                if active.wait(1):
                    server.shutdown()
                thread.join(timeout=2)
            self.assertFalse(thread.is_alive())
        self.assertTrue(all(row[2] for row in events if row[0] == "POST"),
                        "every admitted attempt must retain the original exact envelope")
        RECORDS.append({"case": self.id().split(".")[-1], "mode": mode,
                        "rejection": rejection, "delayed_listen": delayed, "runner": runner,
                        "timeout_seconds": timeout, "elapsed_seconds": elapsed,
                        "exit_status": result.returncode, "events": events,
                        "stdout": result.stdout.decode(), "stderr": result.stderr.decode()})
        return result, elapsed, sum(row[0] == "POST" for row in events)

    def test_connection_refusal_then_single_submission(self):
        result, elapsed, posts = self.peer(delayed=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(posts, 1)
        self.assertGreaterEqual(elapsed, 0.9)
        self.assertIn(b"release startup retry: connect", result.stderr)

    def test_checked_target_rejection_then_new_admission(self):
        result, _, posts = self.peer("reject-once")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(posts, 2)

    def test_repeated_target_rejection_uses_original_budget(self):
        result, elapsed, posts = self.peer("reject")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(posts, 2)
        self.assertLess(elapsed, 2.6)
        self.assertIn(b"release startup ended", result.stderr)

    def test_startup_and_accepted_observation_share_budget(self):
        result, elapsed, posts = self.peer("reject-then-pending")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(posts, 2)
        self.assertLess(elapsed, 2.6)
        self.assertIn(b"did not become ready", result.stderr)

    def test_other_or_malformed_rejections_never_retry(self):
        for code, body in [(503, '{"error":-23}'), (409, '{"error":-22}'),
                           (409, '{"error":-2}'), (403, '{"error":-23}'),
                           (409, '{"error":-23,"error":-23}'),
                           (409, '{"error":-23,"generation":1}'),
                           (409, '{"error":"-23"}'), (409, '{"error":-23} trailing')]:
            with self.subTest(code=code, body=body):
                result, _, posts = self.peer("reject", (code, body))
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(posts, 1)
                self.assertNotIn(b"release startup retry", result.stderr)

    def test_uncertain_submission_and_accepted_failures_never_retry(self):
        for mode in ["drop", "truncated", "stall", "malformed", "pending", "superseded", "bad-status"]:
            with self.subTest(mode=mode):
                result, elapsed, posts = self.peer(mode, timeout=1)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(posts, 1)
                self.assertLess(elapsed, 1.6)
                self.assertNotIn(b"release startup retry", result.stderr)

    def test_boot_workers_gate_once_and_stop_on_uncertainty(self):
        for runner in ["run-aarch64.sh", "run-x86_64.sh"]:
            with self.subTest(runner=runner):
                result, _, posts = self.peer("drop", runner=runner)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(posts, 1)
                directory = self.root / runner
                self.assertTrue((directory / "failed").exists())
                self.assertFalse((directory / "result").exists())
                self.assertIn("submitted work may be committed", (directory / "worker.log").read_text())



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
