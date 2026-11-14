#!/usr/bin/env python3
"""Exercise SDK input verification and project creation without native syscalls."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('app_sdk', ROOT / 'scripts/app-sdk.py')
sdk_cli = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sdk_cli)


class SdkTests(unittest.TestCase):
    def test_snapshot_tampering_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            sdk = Path(directory)
            (sdk / 'source.rs').write_text('original')
            (sdk / 'sdk.json').write_text(json.dumps({'files': {'source.rs': hashlib.sha256(b'original').hexdigest()}}))
            sdk_cli.checked_sdk(sdk)
            (sdk / 'source.rs').write_text('substitute')
            with self.assertRaisesRegex(ValueError, 'source.rs'):
                sdk_cli.checked_sdk(sdk)

    def test_starter_has_independent_host_logic_and_refuses_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            sdk = root / 'sdk'; sdk.mkdir()
            lock = {'toolchain': 'nightly-2026-07-27', 'files': {}}
            (sdk / 'sdk.json').write_text(json.dumps(lock))
            (sdk / 'Cargo.lock').write_text('version = 4\n')
            project = root / 'orders'
            sdk_cli.new_project(sdk, project, lock)
            config = json.loads((project / 'charlotte.json').read_text())
            self.assertEqual(config['grants'], [])
            self.assertEqual(config['sdk_fingerprint'], hashlib.sha256((sdk / 'sdk.json').read_bytes()).hexdigest())
            self.assertTrue((project / 'logic/src/lib.rs').is_file())
            self.assertTrue((project / 'app/src/main.rs').is_file())
            (sdk / 'crates/charlotte-app').mkdir(parents=True)
            (sdk / 'crates/charlotte-app/Cargo.toml').write_text('[package]\nname="charlotte-app"\n')
            sdk_cli.checked_project_sources(sdk, project)
            manifest = project / 'app/Cargo.toml'
            manifest.write_text(manifest.read_text().replace(str(sdk / 'crates/charlotte-app'), str(root / 'other-sdk/charlotte-app')))
            with self.assertRaisesRegex(ValueError, 'selected SDK'):
                sdk_cli.checked_project_sources(sdk, project)
            (project / 'sentinel').write_text('keep')
            with self.assertRaisesRegex(ValueError, 'overwrite'):
                sdk_cli.new_project(sdk, project, lock)
            self.assertEqual((project / 'sentinel').read_text(), 'keep')


if __name__ == '__main__':
    unittest.main()
