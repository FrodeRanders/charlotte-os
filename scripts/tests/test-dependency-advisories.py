#!/usr/bin/env python3
"""Exercise the production gate and optional real RustSec regression controls."""
import json
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "scripts/check-dependency-advisories.py"
VERSION = (ROOT / ".cargo/audit-version").read_text().strip()


def clean_report():
    return {"settings": {"target_arch": [], "target_os": [], "severity": None, "ignore": [],
                         "informational_warnings": ["unmaintained", "unsound", "notice"]},
            "vulnerabilities": {"found": False, "count": 0, "list": []}, "warnings": {},
            "database": {"advisory-count": 1355, "last-commit": "a" * 40,
                         "last-updated": "2026-10-26T12:14:38+01:00"}}


class Gate(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="charlotte-audit-policy-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        cargo = self.directory / "cargo"
        cargo.write_text('''#!/usr/bin/env python3
import json,os,sys
from pathlib import Path
if sys.argv[1:]==["audit","--version"]:
    print("cargo-audit-audit "+os.environ["FIXTURE_VERSION"])
else:
    Path(os.environ["FIXTURE_ARGS"]).write_text(json.dumps({"args":sys.argv[1:],"cwd":os.getcwd()}))
    print(os.environ["FIXTURE_REPORT"])
    sys.exit(int(os.environ.get("FIXTURE_EXIT","0")))
''')
        cargo.chmod(0o755)
        self.environment = dict(os.environ, PATH=str(self.directory) + os.pathsep + os.environ["PATH"],
                                FIXTURE_VERSION=VERSION, FIXTURE_REPORT=json.dumps(clean_report()), FIXTURE_ARGS=str(self.directory / "args.json"))

    def execute(self, *args, status=0):
        self.environment["FIXTURE_EXIT"] = str(status)
        return subprocess.run(["python3", str(RUNNER), *args], cwd=self.directory,
                              env=self.environment, capture_output=True, text=True)

    def test_complete_lockfile_and_unfiltered_policy_from_repository_context(self):
        result = self.execute()
        self.assertEqual(result.returncode, 0, result.stderr)
        call = json.loads((self.directory / "args.json").read_text())
        self.assertEqual(call["cwd"], str(ROOT))
        self.assertEqual(call["args"], ["audit", "--db", str(ROOT / "target/security-audit/advisory-db"),
                                      "--url", "https://github.com/RustSec/advisory-db.git",
                                      "--file", str(ROOT / "Cargo.lock"), "--json", "--deny", "warnings"])

    def test_scanner_findings_and_tool_failure_are_propagated(self):
        for status in (1, 17):
            with self.subTest(status=status):
                self.assertEqual(self.execute(status=status).returncode, status)

    def test_wrong_tool_version_rejects_before_scanning(self):
        self.environment["FIXTURE_VERSION"] = "99.0.0"
        result = self.execute()
        self.assertEqual(result.returncode, 2)
        self.assertIn("expected cargo-audit", result.stderr)
        self.assertFalse((self.directory / "args.json").exists())

    def test_narrowing_arguments_cannot_bypass_the_gate(self):
        for args in (("--ignore", "RUSTSEC-2026-0387"), ("--no-fetch",), ("--no-yanked",), ("--stale",)):
            with self.subTest(args=args):
                self.assertEqual(self.execute(*args).returncode, 2)
                self.assertFalse((self.directory / "args.json").exists())

    def test_successful_scanner_cannot_hide_notices_or_narrowed_settings(self):
        for field, value in [("warnings", {"notice": [{"advisory": {"id": "fixture"}}]}),
                             ("settings", {"ignore": ["RUSTSEC-2026-0387"]}),
                             ("vulnerabilities", {"found": True, "count": 1, "list": [{}]}),
                             ("database", {})]:
            with self.subTest(field=field):
                report = clean_report()
                report[field] = value
                self.environment["FIXTURE_REPORT"] = json.dumps(report)
                self.assertEqual(self.execute().returncode, 2)

    def test_malformed_success_is_not_a_clean_audit(self):
        for report in ("", "not json", "null", "{}", "[]"):
            with self.subTest(report=report):
                self.environment["FIXTURE_REPORT"] = report
                self.assertEqual(self.execute().returncode, 2)

    def test_local_policy_overrides_global_defaults_without_exclusions(self):
        policy = tomllib.loads((ROOT / ".cargo/audit.toml").read_text())
        self.assertEqual(policy["advisories"], {"ignore": [], "informational_warnings": ["unmaintained", "unsound", "notice"]})
        self.assertEqual(policy["target"], {"arch": [], "os": []})
        self.assertEqual(policy["output"], {"deny": ["warnings"], "format": "json", "quiet": False})
        self.assertEqual(policy["database"], {"url": "https://github.com/RustSec/advisory-db.git", "fetch": True, "stale": False})
        self.assertEqual(policy["yanked"], {"enabled": True, "update_index": True})

    def test_ci_installs_the_shared_tool_pin_with_locked_dependencies(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text()
        job = workflow.split("  dependency-advisories:\n", 1)[1].split("  formatting:\n", 1)[0]
        self.assertIn('cargo install --locked --version "$(cat "$GITHUB_WORKSPACE/.cargo/audit-version")" cargo-audit', job)
        self.assertIn("working-directory: ${{ runner.temp }}", job)
        self.assertIn("run: python3 scripts/check-dependency-advisories.py", job)
        self.assertNotIn("continue-on-error", job)


@unittest.skipUnless(os.environ.get("CATTEN_ADVISORY_TEST_DB"), "real scanner/database controls are opt-in")
class RealScanner(unittest.TestCase):
    results = []

    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="charlotte-rustsec-controls-")
        cls.directory = Path(cls.temporary.name)
        cls.database = Path(os.environ["CATTEN_ADVISORY_TEST_DB"]).resolve()
        result = subprocess.run(["cargo", "audit", "--version"], cwd=ROOT, capture_output=True, text=True)
        if result.returncode or result.stdout.split()[-1] != VERSION:
            raise ValueError("Install the repository-pinned cargo-audit for real controls")

    @classmethod
    def tearDownClass(cls):
        if output := os.environ.get("CATTEN_ADVISORY_TEST_EVIDENCE"):
            Path(output).write_text(json.dumps(cls.results, indent=2) + "\n")
        cls.temporary.cleanup()

    def scan(self, package):
        lock = 'version = 4\n[[package]]\nname = "audit-fixture"\nversion = "0.0.0"\n'
        if package:
            name, version = package
            lock += ('[[package]]\n' + f'name = "{name}"\nversion = "{version}"\n'
                     'source = "registry+https://github.com/rust-lang/crates.io-index"\n')
        path = type(self).directory / "Cargo.lock"
        path.write_text(lock)
        # Fixtures use the already-fetched real DB, without a live registry
        # query for deliberately old package versions. Production keeps both on.
        result = subprocess.run(["cargo", "audit", "--db", str(type(self).database), "--file", str(path),
                                 "--no-fetch", "--no-yanked", "--json", "--deny", "warnings"],
                                cwd=ROOT, capture_output=True, text=True)
        type(self).results.append({"lockfile": lock, "exit_status": result.returncode,
                                  "stdout": result.stdout, "stderr": result.stderr})
        self.assertTrue(result.stdout, result.stderr)
        report = json.loads(result.stdout)
        type(self).results[-1]["report"] = report
        return result, report

    def test_clean_lockfile_passes(self):
        result, report = self.scan(None)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(report["vulnerabilities"]["found"])
        self.assertFalse(report["warnings"])

    def test_known_vulnerability_rejects(self):
        result, report = self.scan(("crypto-bigint", "0.5.5"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("RUSTSEC-2026-0387", [v["advisory"]["id"] for v in report["vulnerabilities"]["list"]])

    def test_informational_warning_rejects_without_a_vulnerability(self):
        result, report = self.scan(("ansi_term", "0.12.1"))
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(report["vulnerabilities"]["found"])
        warnings = report["warnings"]["unmaintained"]
        self.assertIn("RUSTSEC-2021-0139", [w["advisory"]["id"] for w in warnings])

    def test_notice_requires_the_gate_even_when_scanner_returns_success(self):
        result, report = self.scan(("plutonium", "0.9.0"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(report["vulnerabilities"]["found"])
        self.assertIn("RUSTSEC-2020-0011", [w["advisory"]["id"] for w in report["warnings"]["notice"]])
        spec = importlib.util.spec_from_file_location("dependency_gate", RUNNER)
        gate = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(gate)
        with self.assertRaisesRegex(ValueError, "advisory warnings remain"):
            gate.validate_report(report)


if __name__ == "__main__":
    unittest.main(verbosity=2)
