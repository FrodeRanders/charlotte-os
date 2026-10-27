#!/usr/bin/env python3
"""Check the real deployment-apply CLI against a local scripted ingress peer."""
import argparse
import management_tls_fixture
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest


ROOT = Path(__file__).resolve().parents[2]
SIGNER = ROOT / "target/debug/cluster-sign"


def status(generation, state="ready"):
    return (f'{{"state":"{state}","deployment_generation":{generation},'
            f'"service_generation":{7 if state == "ready" else 0},'
            f'"node_key":1,"desired_replicas":1,'
            f'"ready_replicas":{1 if state == "ready" else 0}}}')


class Generation(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="charlotte-deployment-generation-")
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        key = root / "fixture.hex"
        key.write_bytes((ROOT / "tools/cluster-sign/dev-key.hex").read_bytes())
        key.chmod(0o600)
        image = bytearray(193)
        image[:6] = b"\x7fELF\x02\x01"
        for offset, value, size in [(18, 183, 2), (40, 64, 8), (58, 64, 2),
                                    (60, 2, 2), (62, 1, 2), (132, 3, 4),
                                    (152, 192, 8), (160, 1, 8)]:
            image[offset:offset+size] = value.to_bytes(size, "little")
        elf = root / "fixture.elf"
        elf.write_bytes(image)
        self.root, self.key, self.elf = root, key, elf
        self.descriptor = self.prepare("app")

    def prepare(self, name):
        descriptor = self.root / f"{name}.cdep"
        result = subprocess.run([str(SIGNER), "deployment-sign", str(descriptor),
                                 name, f"apps/{name}.elf", str(self.elf), "0", "1", "4", "1",
                                 "5000", str(self.key)], capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        return descriptor

    def apply(self, admission, observations, descriptors=None, expected_posts=None, release=None):
        events = []
        descriptors = descriptors or [self.descriptor]
        expected_posts = (1 if release else len(descriptors)) if expected_posts is None else expected_posts

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def reply(self, code, body):
                data = body.encode()
                self.send_response(code)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(data)

            def do_POST(self):
                self.connection.settimeout(2)
                data = self.rfile.read(int(self.headers["Content-Length"]))
                index = sum(event[0] == "POST" for event in events)
                events.append(("POST", self.path, len(data)))
                body = admission[min(index, len(admission)-1)] if isinstance(admission, list) else admission
                self.reply(202, body(data) if callable(body) else body)

            def do_GET(self):
                index = sum(event[0] == "GET" and event[1] == self.path for event in events)
                events.append(("GET", self.path, 0))
                rows = observations[self.path] if isinstance(observations, dict) else observations
                self.reply(200, rows[min(index, len(rows)-1)])

            def log_message(self, *_):
                pass

        with HTTPServer(("127.0.0.1", 0), Handler) as server:
            management_tls_fixture.install(server)
            thread = threading.Thread(target=server.serve_forever,
                                      kwargs={"poll_interval": 0.01}, daemon=True)
            thread.start()
            try:
                endpoint = f"127.0.0.1:{server.server_port}"
                command = (["release-apply", str(release), endpoint, "2"] if release else
                           ["deployment-apply", endpoint, "2", *map(str, descriptors)])
                result = subprocess.run([str(SIGNER), *command], capture_output=True, timeout=10)
            finally:
                server.shutdown()
                thread.join(timeout=2)
            self.assertFalse(thread.is_alive())
        self.assertEqual(sum(event[0] == "POST" for event in events), expected_posts,
                         "accepted descriptor must never be replayed")
        self.assertEqual(events[0][1], "/v1/releases" if release else "/v1/deployments")
        self.assertEqual(events[0][2], (release or self.descriptor).stat().st_size)
        return result, sum(event[0] == "GET" for event in events)

    def test_exact_ready_generation_succeeds(self):
        result, queries = self.apply('{"generation":2}', [status(2)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(queries, 1)

    def test_older_ready_generation_cannot_complete(self):
        result, queries = self.apply('{"generation":2}', [status(1)])
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertGreaterEqual(queries, 1)

    def test_superseding_ready_generation_fails(self):
        result, queries = self.apply('{"generation":2}', [status(3)])
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(b"superseded", result.stderr)
        self.assertEqual(queries, 1)

    def test_older_ready_then_exact_ready_waits(self):
        result, queries = self.apply('{"generation":2}', [status(1), status(2)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(queries, 2)

    def test_exact_pending_then_ready_waits(self):
        result, queries = self.apply('{"generation":2}', [status(2, "committed"), status(2)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(queries, 2)

    def test_malformed_admission_stops_before_observation(self):
        for admission in ['{"other":2}', '{"generation":0}',
                          '{"generation":2,"generation":3}']:
            with self.subTest(admission=admission):
                result, queries = self.apply(admission, [status(2)])
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual(queries, 0)

    def test_malformed_ready_status_is_rejected(self):
        for observation in [status(2)+" trailing", status(2).replace(
                '"deployment_generation":2', '"deployment_generation":2,"deployment_generation":3'),
                status(2).replace('"ready_replicas":1', '"ready_replicas":0'),
                status(2)+'\r\n\r\n{"ignored":true}', '\u00a0'+status(2)+'\u00a0']:
            with self.subTest(observation=observation):
                result, queries = self.apply('{"generation":2}', [observation])
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertEqual(queries, 1)


    def test_each_component_retains_its_own_generation(self):
        second = self.prepare("app2")
        result, queries = self.apply(
            ['{"generation":2}', '{"generation":8}'],
            {"/v1/deployments/app": [status(2)], "/v1/deployments/app2": [status(8)]},
            descriptors=[self.descriptor, second],
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(queries, 2)

    def test_uncertain_second_admission_stops_the_batch(self):
        second, third = self.prepare("app2"), self.prepare("app3")
        result, queries = self.apply(
            ['{"generation":2}', '{"other":8}'], [status(2)],
            descriptors=[self.descriptor, second, third], expected_posts=2,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(queries, 0)
        self.assertIn(b"this and earlier descriptors may already be committed", result.stderr)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--signer", type=Path, default=SIGNER)
    options, remaining = parser.parse_known_args()
    SIGNER = options.signer.resolve()
    unittest.main(argv=[__file__, *remaining])
