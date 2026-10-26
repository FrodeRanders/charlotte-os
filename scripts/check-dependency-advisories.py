#!/usr/bin/env python3
"""Fail closed on the complete lockfile's advisory report and scanner errors."""
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def validate_report(report):
    """A zero scanner status alone does not reject informational notices."""
    expected = {"target_arch": [], "target_os": [], "severity": None, "ignore": [],
                "informational_warnings": ["unmaintained", "unsound", "notice"]}
    if report["settings"] != expected:
        raise ValueError("advisory scan settings exclude required scope")
    findings = report["vulnerabilities"]
    if findings["found"] is not False or findings["count"] != 0 or findings["list"] != []:
        raise ValueError("advisory vulnerabilities remain")
    warnings = report["warnings"]
    if not isinstance(warnings, dict) or any(not isinstance(v, list) or v for v in warnings.values()):
        raise ValueError("advisory warnings remain or are malformed")
    database = report["database"]
    if database["advisory-count"] <= 0 or not database["last-commit"] or not database["last-updated"]:
        raise ValueError("advisory database identity is missing")


def main():
    if len(sys.argv) != 1:
        raise ValueError("usage: python3 scripts/check-dependency-advisories.py")
    if not (ROOT / ".cargo/audit.toml").is_file():
        raise ValueError("repository advisory policy is missing")
    expected = (ROOT / ".cargo/audit-version").read_text().strip()
    version = subprocess.run(["cargo", "audit", "--version"], cwd=ROOT,
                             capture_output=True, text=True, check=True).stdout.strip()
    if version.split()[-1] != expected:
        raise ValueError(f"expected cargo-audit {expected}, got {version}")
    result = subprocess.run(["cargo", "audit", "--db", str(ROOT / "target/security-audit/advisory-db"),
                             "--url", "https://github.com/RustSec/advisory-db.git",
                             "--file", str(ROOT / "Cargo.lock"), "--json", "--deny", "warnings"],
                            cwd=ROOT, stdout=subprocess.PIPE)
    sys.stdout.buffer.write(result.stdout)
    sys.stdout.buffer.flush()
    if result.returncode:
        return result.returncode if result.returncode > 0 else 128 - result.returncode
    validate_report(json.loads(result.stdout))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, KeyError, TypeError, IndexError, subprocess.CalledProcessError) as error:
        print(f"error: dependency advisory gate: {error}", file=sys.stderr)
        sys.exit(2)
