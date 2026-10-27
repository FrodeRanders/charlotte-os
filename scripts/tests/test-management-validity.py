#!/usr/bin/env python3
"""Enrolled client date rejection, qualified by native events and healthy peers."""
import argparse
import json
import os
from pathlib import Path
import re
import ssl
import time

from management_tls import ProbeFailure, context, exchange

EVENTS = {"expired": "client certificate expired", "future": "client certificate not yet valid"}


def events(text, category):
    # Every error event comes from a distinct local connection. Do not accept a
    # TLS/HTTP error string or a repeated old event as native validity evidence.
    return {(int(match[1]), int(match[2])) for match in re.finditer(
        r"\[EL0 LOG\] lp=\d+ asid=(\d+) \[management\] connection=(\d+) "
        + re.escape(EVENTS[category]) + r"\r?$", text, re.MULTILINE)}


def probe(log, status_port, deployment_port, budget=12, *, scope="leaf"):
    directory = Path(os.environ["CATTEN_MANAGEMENT_VALIDITY_DIR"])
    if scope not in ("leaf", "issuer"):
        raise ProbeFailure("unknown validity scope")
    records = json.loads((directory / ("validity.json" if scope == "leaf" else "chain-validity.json")).read_text())
    prefix = "" if scope == "leaf" else "issuer-"
    expected = {(prefix + category + "-" + role, category, role) for category in EVENTS for role in ("reader", "administrator")}
    if {(row["name"], row["category"], row["role"]) for row in records} != expected or len(records) != len(expected):
        raise ProbeFailure("validity controls must contain all four distinct enrolled role/date combinations")
    healthy = context("reader")
    observations = []
    for port, request, expected_status in (
        (status_port, b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n", 200),
        (deployment_port, b"GET /v1/deployments/audit_missing HTTP/1.1\r\nHost: localhost\r\n\r\n", 404),
    ):
        exchange(healthy, port, request, expected_status, budget)
        for row in records:
            policy = ssl.create_default_context(cafile=os.environ["CATTEN_MANAGEMENT_CA_FILE"])
            policy.minimum_version = policy.maximum_version = ssl.TLSVersion.TLSv1_3
            policy.set_alpn_protocols(["http/1.1"])
            policy.load_cert_chain(directory / (row["name"] + ".pem"), directory / (row["name"] + ".key"))
            before = events(log.read_text(), row["category"])
            end = time.monotonic() + budget
            try:
                exchange(policy, port, request, None, max(0, end - time.monotonic()))
            except ssl.SSLCertVerificationError:
                raise ProbeFailure("server verification failure cannot qualify client validity rejection") from None
            except (ssl.SSLError, ConnectionError, EOFError):
                # UART-file delivery can lag transport close. Retain the same
                # original probe budget; silence/timeout never proves rejection.
                while not (new := events(log.read_text(), row["category"]) - before):
                    if time.monotonic() >= end:
                        raise ProbeFailure("client validity rejection lacked its exact native event") from None
                    time.sleep(min(0.05, max(0, end - time.monotonic())))
                if len(new) != 1:
                    raise ProbeFailure("ambiguous native validity rejection episode")
            else:
                raise ProbeFailure("invalid-date enrolled client received management data")
            result, _ = exchange(healthy, port, request, expected_status, budget)
            observation = {"name": row["name"], "role": row["role"], "category": row["category"],
                           "certificate_scope": scope,
                           "port": port, "native_event": EVENTS[row["category"]],
                           "source_asid": next(iter(new))[0], "connection": next(iter(new))[1],
                           "healthy_reader_status": result["status"]}
            observations.append(observation)
            print("PASS: management validity " + json.dumps(observation, sort_keys=True), flush=True)
    return observations


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--status-port", type=int, required=True)
    parser.add_argument("--deployment-port", type=int, required=True)
    parser.add_argument("--probe-timeout", type=float, default=12)
    args = parser.parse_args()
    if args.probe_timeout <= 0 or not all(1 <= port <= 65535 for port in (args.status_port, args.deployment_port)) or args.status_port == args.deployment_port:
        parser.error("requires a positive probe budget and two distinct valid ports")
    probe(args.log, args.status_port, args.deployment_port, args.probe_timeout)
