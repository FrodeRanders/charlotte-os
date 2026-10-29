#!/usr/bin/env python3
"""Execute each actual runner's HTTP gate with controlled host observations."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def gate(source):
    begin = source.index('        if [ "$HTTP_TEST" = "1" ] && [ "$HTTP_PROBED" = "0" ]')
    ending = '\n        fi'
    end = source.index(ending, begin) + len(ending)
    return source[begin:end]


class RunnerGates(unittest.TestCase):
    def test_startup_assertions_require_unambiguous_isolated_observation(self):
        base = dict(os.environ, MANAGEMENT_STARTUP_TEST="disabled", TIMEOUT="90", NET_BACKEND="user",
                    NETWORK="1", GDB="", HTTP_TEST="0", DEPLOYMENT_INGRESS_TEST="0",
                    SHUTDOWN_INGRESS_TEST="0", SECURITY_TEST="0", SHUTDOWN_TEST="0",
                    LIVE_UPGRADE_TEST="0", FAILURE_CAPTURE="0")
        for change, accepted in (({}, True), ({"MANAGEMENT_STARTUP_TEST": "Signature"}, True),
                                 ({"MANAGEMENT_STARTUP_TEST": "other"}, False),
                                 ({"TIMEOUT": ""}, False), ({"HTTP_TEST": "1"}, False),
                                 ({"NET_BACKEND": "socket"}, False), ({"NETWORK": "0"}, False),
                                 ({"MANAGEMENT_STARTUP_TEST": "WrongPolicy", "FAILURE_CAPTURE": "1"}, False)):
            result = subprocess.run(["bash", "-c", 'source "$1"; catten_boot_validate_management_startup',
                                     "management-validation", str(ROOT / "scripts/lib/boot-common.sh")],
                                    env=dict(base, **change), capture_output=True, text=True, timeout=5)
            self.assertEqual(result.returncode == 0, accepted, result.stderr)

    def run_gate(self, architecture, identity=0, liveness=0, baseline=False, serial=None,
                 listener_only=False):
        source = (ROOT / "scripts" / f"run-{architecture}.sh").read_text()
        if baseline:
            source = (ROOT / "scripts/tests/fixtures/management-x86-http-gate-before.sh").read_text()
        if listener_only:
            self.assertEqual(source.count('&& catten_boot_http_probe_ready; then'), 1)
            source = source.replace('&& catten_boot_http_probe_ready; then',
                                    '&& grep -Fq "httpd is listening" "$LOG"; then')
        with tempfile.TemporaryDirectory(prefix="charlotte-management-gate-") as directory:
            log = Path(directory) / "serial"
            log.write_text(serial if serial is not None else
                           "httpd is listening\n[time] synchronized unix_s=1793177170 utc=fixture\n")
            environment = dict(os.environ, LOG=str(log), HTTP_TEST="1", HTTP_PROBED="0",
                               HTTP_PROBE_OK="0", ROOT_DIR=str(ROOT), HTTP_HOST_PORT="1",
                               DEPLOY_HOST_PORT="2", IDENTITY_STATUS=str(identity),
                               LIVENESS_STATUS=str(liveness), CATTEN_QEMU_MONITOR="0",
                               CATTEN_MANAGEMENT_CA_FILE="fixture-ca",
                               CATTEN_MANAGEMENT_CERT_FILE="fixture-client",
                               CATTEN_MANAGEMENT_KEY_FILE="fixture-key",
                               CATTEN_MANAGEMENT_READER_CERT_FILE="fixture-reader",
                               CATTEN_MANAGEMENT_UNKNOWN_CERT_FILE="fixture-unregistered")
            harness = r'''
set -eu
source "$ROOT_DIR/scripts/lib/boot-common.sh"
python3() {
    case "$1" in
        *test-management-tls.py) return "$IDENTITY_STATUS" ;;
        *test-http-keyhole-liveness.py|*test-deployment-http-liveness.py) return "$LIVENESS_STATUS" ;;
        *) command python3 "$@" ;;
    esac
}
curl() {
    case "${@: -1}" in
        */cluster/metrics) echo '{"schema":"charlotte.cluster.v2","fresh_committed":true,"raft":{"served_by":"fixture"},"deployments":[]}' ;;
        *) echo '{"http":{"requests":1}}' ;;
    esac
}
sleep() { :; }
'''
            result = subprocess.run(["bash", "-c", harness + gate(source) + '\nprintf "GATE=%s\\n" "$HTTP_PROBE_OK"\n'],
                                    env=environment, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout.splitlines()[-1]

    def test_positive_observations_open_both_gates(self):
        for architecture in ("aarch64", "x86_64"):
            self.assertEqual(self.run_gate(architecture), "GATE=1")

    def test_listener_without_positive_native_time_does_not_start_probes(self):
        for architecture in ("aarch64", "x86_64"):
            for sample in ("", "[time] synchronized unix_s=0 ",
                           "[time] synchronized unix_s=12bad ",
                           "[peer] synchronized unix_s=1793177170 "):
                with self.subTest(architecture=architecture, sample=sample):
                    self.assertEqual(self.run_gate(architecture, serial="httpd is listening\n" + sample),
                                     "GATE=0")

    def test_time_without_listener_does_not_start_probes(self):
        for architecture in ("aarch64", "x86_64"):
            self.assertEqual(self.run_gate(architecture, serial="[time] synchronized unix_s=1793177170\n"),
                             "GATE=0")

    def test_prior_listener_only_gate_starts_without_time(self):
        for architecture in ("aarch64", "x86_64"):
            self.assertEqual(self.run_gate(architecture, serial="httpd is listening\n",
                                           listener_only=True), "GATE=1")

    def test_identity_rejection_cannot_be_masked_by_healthy_metrics(self):
        for architecture in ("aarch64", "x86_64"):
            self.assertEqual(self.run_gate(architecture, identity=1), "GATE=0")

    def test_liveness_rejection_cannot_be_masked_by_healthy_metrics(self):
        for architecture in ("aarch64", "x86_64"):
            self.assertEqual(self.run_gate(architecture, liveness=1), "GATE=0")

    def test_prior_x86_gate_accepts_the_same_failed_identity_control(self):
        self.assertEqual(self.run_gate("x86_64", identity=1, baseline=True), "GATE=1")


if __name__ == "__main__":
    unittest.main()
