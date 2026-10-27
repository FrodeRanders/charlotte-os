#!/usr/bin/env python3
"""Check complete ingress acknowledgements and policy observations with the real CLI."""
import argparse
import management_tls_fixture
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import threading
import unittest

spec = importlib.util.spec_from_file_location("deployment_generation", Path(__file__).with_name("test-deployment-generation.py"))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
RECORDS = []
POLICY = '{"generation":2,"sequence":3,"assignments":1,"not_before_unix_seconds":1,"expires_unix_seconds":301}'


class Response(unittest.TestCase):
    prepare = fixture.Generation.prepare

    def run_cli(self, *args):
        result = subprocess.run([str(fixture.SIGNER), *map(str, args)], capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result

    def setUp(self):
        fixture.Generation.setUp(self)
        public = self.root / "fresh.pub"
        self.key = self.root / "fresh.key"
        self.run_cli("generate", self.key, public)
        self.descriptor = self.prepare("fresh")
        self.release = self.root / "release"
        self.run_cli("release-sign", self.release, "batch", "1", self.key, self.descriptor)
        self.shutdown = self.root / "shutdown"
        self.run_cli("shutdown-sign", self.shutdown, "1", "1", "1", "301", "5000", "5000", self.key)
        self.policy = self.root / "policy"
        self.run_cli("ingress-policy-sign", self.policy, "1", "1", "301", "11"*32, self.key, "--clear")
        plaintext, envelope, bundle = [self.root / name for name in ["plaintext", "envelope", "bundle"]]
        plaintext.write_bytes(b"public synthetic profile")
        tool = fixture.ROOT / "tools/cluster-sign"
        self.run_cli("operations-seal", envelope, "public-profile", "s3", "11"*32,
                     hashlib.sha256(self.release.read_bytes()).hexdigest(), "1", "1000",
                     tool / "dev-recipient-key.pub", tool / "dev-operations-key.hex", plaintext)
        self.run_cli("operations-bundle-sign", bundle, "1", "11"*32, public.read_text().strip(),
                     tool / "dev-operations-key.hex", tool / "dev-recipient-key.pub", self.release,
                     "fresh", "profiles/fresh.enc", envelope)
        self.inputs = {"deployment": self.descriptor, "shutdown": self.shutdown,
                       "ingress-policy": self.policy, "operations-bundle": bundle, "release": self.release}

    def observe(self, kind, body, code=None):
        events = []
        status = kind == "policy-status"

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def reply(self):
                data = body.encode()
                self.send_response(code if code is not None else 200 if status else 202)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(data)

            def do_GET(self):
                events.append(("GET", self.path, 0))
                self.reply()

            def do_POST(self):
                self.connection.settimeout(3)
                data = self.rfile.read(int(self.headers["Content-Length"]))
                events.append(("POST", self.path, len(data)))
                self.reply()

            def log_message(self, *_):
                pass

        with HTTPServer(("127.0.0.1", 0), Handler) as server:
            management_tls_fixture.install(server)
            thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01})
            thread.start()
            endpoint = f"127.0.0.1:{server.server_port}"
            args = (["ingress-policy-status", endpoint] if status else
                    [kind+"-notify", str(self.inputs[kind]), endpoint])
            try:
                result = subprocess.run([str(fixture.SIGNER), *args], capture_output=True, timeout=6)
            finally:
                server.shutdown()
                thread.join(timeout=2)
            self.assertFalse(thread.is_alive())
        self.assertEqual(len(events), 1, "a response rejection must not replay the request")
        self.assertEqual(events[0][0], "GET" if status else "POST")
        if not status:
            self.assertEqual(events[0][2], self.inputs[kind].stat().st_size)
        RECORDS.append({"case":self.id().split(".")[-1],"kind":kind,"body":body,"events":events,
                        "exit_status":result.returncode,"stdout":result.stdout.decode(),"stderr":result.stderr.decode()})
        return result

    def test_current_scalar_acknowledgements_succeed(self):
        for kind in ["deployment", "shutdown", "ingress-policy", "operations-bundle"]:
            with self.subTest(kind=kind):
                result = self.observe(kind, ' { "generation" : 2 }\n')
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_malformed_scalar_acknowledgements_are_uncertain(self):
        for kind in ["deployment", "shutdown", "ingress-policy", "operations-bundle"]:
            for body in ['{}','{"generation":0}','{"generation":-1}','{"generation":2,"generation":3}',
                         '{"generation":2,"extra":1}','{"generation":2} trailing',
                         '{"generation":2}\r\n\r\n{"extra":1}','\u00a0{"generation":2}\u00a0']:
                with self.subTest(kind=kind,body=body):
                    result=self.observe(kind,body)
                    self.assertNotEqual(result.returncode,0,result.stdout)
                    self.assertEqual(result.stdout,b"")
                    self.assertIn(b"uncertain",result.stderr)

    def test_release_acknowledgement_preserves_non_json_whitespace(self):
        envelope=self.release.read_bytes()
        record=b"CRLADM01"+hashlib.sha256(envelope).digest()+struct.pack("<QH6xQ",1,1,2)
        body=json.dumps({"admission":record.hex()})
        for valid in [True,False]:
            with self.subTest(valid=valid):
                result=self.observe("release",body if valid else '\u00a0'+body+'\u00a0')
                self.assertEqual(result.returncode,0 if valid else 1,result.stderr)
                if not valid:
                    self.assertEqual(result.stdout,b"")
                    self.assertIn(b"uncertain",result.stderr)

    def test_current_policy_observations_succeed(self):
        for body in [POLICY,json.dumps(dict(reversed(list(json.loads(POLICY).items()))),indent=2),POLICY.replace('"assignments":1','"assignments":0')]:
            with self.subTest(body=body):
                result=self.observe("policy-status",body)
                self.assertEqual(result.returncode,0,result.stderr)

    def test_malformed_policy_observations_reject(self):
        for body in ['{}',POLICY.replace('"generation":2','"generation":0'),
                     POLICY.replace('"sequence":3','"sequence":0'),
                     POLICY.replace('"assignments":1','"assignments":17'),
                     POLICY.replace('"expires_unix_seconds":301','"expires_unix_seconds":0'),
                     POLICY.replace('"generation":2','"generation":2,"generation":3'),
                     POLICY+'\r\n\r\n{}','\u00a0'+POLICY+'\u00a0']:
            with self.subTest(body=body):
                result=self.observe("policy-status",body)
                self.assertNotEqual(result.returncode,0,result.stdout)
                self.assertEqual(result.stdout,b"")
                self.assertIn(b"invalid ingress-policy status",result.stderr)

    def test_policy_http_error_is_reported(self):
        result=self.observe("policy-status",'{"error":-2}',code=404)
        self.assertNotEqual(result.returncode,0)


if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--signer",type=Path,default=fixture.SIGNER)
    parser.add_argument("--evidence",type=Path)
    options,remaining=parser.parse_known_args()
    fixture.SIGNER=options.signer.resolve()
    outcome=unittest.main(argv=[__file__,*remaining],exit=False)
    if options.evidence:
        options.evidence.write_text(json.dumps(RECORDS,indent=2)+"\n")
    raise SystemExit(0 if outcome.result.wasSuccessful() else 1)
