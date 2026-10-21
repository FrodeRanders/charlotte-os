#!/usr/bin/env python3
"""Check release-apply against complete causal receipts from a scripted HTTP peer."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import unittest

spec = importlib.util.spec_from_file_location("deployment_generation", Path(__file__).with_name("test-deployment-generation.py"))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


def admission(envelope, generations=(2,8), release_generation=1):
    data = (b"CRLADM01" + hashlib.sha256(envelope).digest() +
            struct.pack("<QH6x", release_generation, len(generations)) +
            b"".join(struct.pack("<Q", generation) for generation in generations))
    return json.dumps({"admission": data.hex()})


class ReleaseGeneration(unittest.TestCase):
    prepare = fixture.Generation.prepare
    apply = fixture.Generation.apply

    def setUp(self):
        fixture.Generation.setUp(self)
        second = self.prepare("app2")
        self.release = self.root / "release.crelease"
        result = subprocess.run([str(fixture.SIGNER), "release-sign", str(self.release), "batch", "1",
                                 str(self.key), str(self.descriptor), str(second)], capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)

    def run_release(self, ack, first=None, second=None):
        return self.apply(ack, {"/v1/deployments/app": first or [fixture.status(2)],
                               "/v1/deployments/app2": second or [fixture.status(8)]}, release=self.release)

    def test_independent_component_generations_succeed(self):
        result, queries = self.run_release(admission)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(queries,2)

    def test_release_generation_cannot_satisfy_component(self):
        result, queries = self.run_release(admission, first=[fixture.status(1)])
        self.assertNotEqual(result.returncode,0,result.stdout)
        self.assertGreaterEqual(queries,2)

    def test_older_then_exact_component_waits(self):
        result, queries = self.run_release(admission, first=[fixture.status(1),fixture.status(2)])
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertEqual(queries,3)

    def test_superseding_component_stops_without_replay(self):
        result, queries = self.run_release(admission, second=[fixture.status(9)])
        self.assertNotEqual(result.returncode,0,result.stdout)
        self.assertIn(b"superseded",result.stderr)
        self.assertEqual(queries,2)

    def test_scalar_or_malformed_ack_stops_before_observation(self):
        good = json.loads(admission(self.release.read_bytes()))["admission"]
        for ack in ['{"generation":1}', '{"admission":"00"}',
                    json.dumps({"admission":good+"00"}),
                    '{"admission":"'+good+'","admission":"'+good+'"}',
                    json.dumps({"admission":good,"other":1})]:
            with self.subTest(ack=ack):
                result, queries = self.run_release(ack)
                self.assertNotEqual(result.returncode,0,result.stdout)
                self.assertEqual(queries,0)
                self.assertIn(b"uncertain",result.stderr)

    def test_wrong_envelope_or_count_cannot_bind_receipt(self):
        for ack in [admission(b"other"), admission(self.release.read_bytes(),(2,)),
                    admission(self.release.read_bytes(),(2,0))]:
            with self.subTest(ack=ack):
                result,queries=self.run_release(ack)
                self.assertNotEqual(result.returncode,0,result.stdout)
                self.assertEqual(queries,0)

    def test_malformed_component_status_stops(self):
        result, queries=self.run_release(admission, second=[fixture.status(8)+" trailing"])
        self.assertNotEqual(result.returncode,0,result.stdout)
        self.assertEqual(queries,2)


if __name__ == "__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--signer",type=Path,default=fixture.SIGNER)
    options,remaining=parser.parse_known_args()
    fixture.SIGNER=options.signer.resolve()
    unittest.main(argv=[__file__,*remaining])
