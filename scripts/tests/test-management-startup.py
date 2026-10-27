#!/usr/bin/env python3
"""Qualify a native management startup result without treating silence as proof."""
import argparse
from pathlib import Path
import socket
import ssl
import time

import importlib.util
import sys

# Also support imports by the observation tests outside this script directory.
sys.path.insert(0, str(Path(__file__).resolve().parent))
validity_spec = importlib.util.spec_from_file_location("management_validity_probe", Path(__file__).with_name("test-management-validity.py"))
validity = importlib.util.module_from_spec(validity_spec)
validity_spec.loader.exec_module(validity)

spec = importlib.util.spec_from_file_location("management_identity_probe", Path(__file__).with_name("test-management-tls.py"))
identity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(identity)

PUBLISHED = "[boot trust] admission policy published: revision="
STEADY = "[launch] steady-state service set published."
DISABLED = ("[http] management credentials unavailable; ingress disabled",
            "[deployd] management credentials unavailable; ingress disabled")
REJECTED = "[launch] management TLS resource rejected before trust publication: "
EXPECTATIONS = ("ready", "validity", "disabled", "WrongPolicy", "Signature", "Authentication")


def observed(text, expectation):
    """Return readiness for a specific assertion, not success from a timeout."""
    if expectation in ("ready", "validity"):
        return STEADY in text and "[deployd] listening port=7444" in text and "[time] synchronized " in text
    if expectation == "disabled":
        return STEADY in text and "DHCP assigned " in text and all(marker in text for marker in DISABLED)
    return (REJECTED + expectation + "\r" in text or REJECTED + expectation + "\n" in text) and "boot admission trust rejected" in text


def validate_snapshot(text, expectation):
    assert observed(text, expectation), "specific startup observation is missing"
    if expectation in ("ready", "validity", "disabled"):
        assert text.count(PUBLISHED) == 1, "startup did not publish exactly one admission policy"
        assert "Kernel panic:" not in text, "startup panicked for an unrelated reason"
        assert text.index(PUBLISHED) < text.index(STEADY), "service set preceded trust publication"
        if expectation in ("ready", "validity"):
            assert not any(marker in text for marker in DISABLED), "valid credentials disabled a listener"
    else:
        assert "Kernel panic:" in text, "boot did not terminate at its rejection gate"
        assert PUBLISHED not in text, "rejected resource published admission trust"
        assert STEADY not in text, "rejected resource published steady-state services"
    if expectation not in ("ready", "validity"):
        assert "[deployd] listening port=7444" not in text, "rejected startup opened deployment listener"


def closed(port, payload, tls, timeout):
    end = time.monotonic() + timeout
    def remaining():
        budget = end - time.monotonic()
        if budget <= 0:
            raise TimeoutError("closed-port observation deadline expired")
        return budget
    try:
        raw = socket.create_connection(("127.0.0.1", port), timeout=remaining())
    except (ConnectionRefusedError, ConnectionResetError):
        return
    with raw:
        raw.settimeout(remaining())
        if tls:
            # Default verified client policy: a certificate, alert or malformed
            # TLS response is unexpected. Only reset/EOF proves this observation.
            policy = ssl.create_default_context()
            policy.minimum_version = ssl.TLSVersion.TLSv1_3
            try:
                client = policy.wrap_socket(raw, server_hostname="127.0.0.1")
            except (ssl.SSLEOFError, ConnectionResetError):
                return
            with client:
                raise AssertionError("disabled management endpoint completed a TLS handshake")
        try:
            raw.sendall(payload)
            raw.settimeout(remaining())
            assert raw.recv(4096) == b"", "disabled endpoint returned application data"
        except (ConnectionResetError, BrokenPipeError):
            return


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--expect", choices=EXPECTATIONS, required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--status-port", type=int, required=True)
    parser.add_argument("--deployment-port", type=int, required=True)
    parser.add_argument("--timeout", type=float, default=5, help="original budget per closed-port observation")
    parser.add_argument("--observe-only", action="store_true")
    args = parser.parse_args()
    text = args.log.read_text()
    if args.observe_only:
        return 0 if observed(text, args.expect) else 1
    if args.timeout <= 0:
        parser.error("probe timeout must be positive")
    validate_snapshot(text, args.expect)
    if args.expect in ("ready", "validity"):
        identity.healthy(args.status_port, identity.OBSERVE, 200)
        identity.healthy(args.deployment_port, identity.STATUS, 404)
        identity.healthy(args.deployment_port, identity.MUTATE, 403)
        print("PASS: native startup: verified reader observations and mutation rejection", flush=True)
        if args.expect == "validity":
            validity.probe(args.log, args.status_port, args.deployment_port)
            validate_snapshot(args.log.read_text(), args.expect)
    elif args.expect == "disabled":
        for port, payload in ((args.status_port, identity.OBSERVE),
                              (args.deployment_port, identity.STATUS)):
            for tls in (False, True):
                try:
                    closed(port, payload, tls, args.timeout)
                    print(f"OBSERVED: disabled port={port} tls={tls}: EOF/reset/refusal", flush=True)
                except TimeoutError:
                    # SLIRP's host listener can accept while the guest supplies
                    # no terminal TCP result. Preserve that uncertainty. The
                    # startup assertion comes from both explicit disabled events,
                    # not this timeout or any physical socket-close claim.
                    print(f"OBSERVED: disabled port={port} tls={tls}: timeout; transport closure unqualified", flush=True)
        # Re-read after network observations so a late publication cannot be
        # mistaken for disabled/rejected startup. No timeout is success evidence.
        validate_snapshot(args.log.read_text(), args.expect)
        print("PASS: native startup disabled: both service-disabled entries and single ordered policy publication verified", flush=True)
    else:
        validate_snapshot(args.log.read_text(), args.expect)
        print(f"PASS: native startup {args.expect}: exact boot rejection before trust/service publication; terminal boot gate", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
