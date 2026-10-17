#!/usr/bin/env python3
"""Probe EOF and idle-client recovery against an isolated running guest.

Example: --port 18080 --ready-log /private/tmp/charlotte-security-serial.log
run-aarch64.sh --http-test invokes this before terminating its guest. It can
also be used against a guest deliberately left running, using its forwarded port.
"""

import argparse
import json
from pathlib import Path
import socket
import struct
import time


def read_metrics(port, require_dns_catalog=False, require_ns_status=False):
    with socket.create_connection(("127.0.0.1", port), timeout=12) as client:
        client.settimeout(12)
        client.sendall(b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        response = bytearray()
        body_start = None
        content_length = None
        while True:
            chunk = client.recv(65536)
            if not chunk:
                raise EOFError("guest closed before sending the complete response")
            response.extend(chunk)
            if body_start is None and b"\r\n\r\n" in response:
                header, _ = bytes(response).split(b"\r\n\r\n", 1)
                body_start = len(header) + 4
                content_length = int(next(
                    line.split(b":", 1)[1].strip()
                    for line in header.split(b"\r\n")[1:]
                    if line.lower().startswith(b"content-length:")
                ))
                assert 0 < content_length <= 1024 * 1024
            if body_start is not None and len(response) >= body_start + content_length:
                break
        header, body = bytes(response).split(b"\r\n\r\n", 1)
        assert header.startswith(b"HTTP/1.1 200 "), header
        document = json.loads(body[:content_length])
        assert "http" in document
        if require_dns_catalog:
            catalog = document["dns"]["catalog"]
            assert catalog["schema"] == "charlotte.dns-catalog.v1"
            rows = catalog["entries"]
            assert isinstance(rows, list) and catalog["count"] == len(rows)
            assert all(isinstance(row["name"], str) and row["name"]
                       and isinstance(row["node"], str) and row["node"]
                       and isinstance(row["generation"], int) and row["generation"] > 0
                       for row in rows)
            assert any(row["name"] == "alpha" for row in rows)
        if require_ns_status:
            status = document["ns"]
            names = status["services"]
            assert status["registered"] == len(names) <= 64
            assert isinstance(status["pending"], int) and status["pending"] >= 0
            prefix = "audit.ns.status."
            expected = {prefix + "x" * (255 - len(prefix)) + chr(ord("a") + i)
                        for i in range(16)}
            assert {name for name in names if name.startswith(prefix)} == expected
            assert all(len(name) == 256 for name in expected)


def metrics(port, require_dns_catalog=False, require_ns_status=False):
    # The serial listener can reset connections during listener handoff. A
    # liveness check requires recovery within a bounded number of requests,
    # not perfect transport delivery. Do not retry malformed HTTP/JSON.
    for attempt in range(3):
        try:
            read_metrics(port, require_dns_catalog, require_ns_status)
            return
        except (OSError, EOFError) as error:
            if attempt == 2:
                raise
            print(f"HTTP transport {type(error).__name__}; retry {attempt + 1}/2", flush=True)
            # Three attempts span six seconds, beyond the guest's five-second
            # idle budget. Immediate retries would only sample the same gap.
            time.sleep(3)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--ready-log", type=Path, required=True)
    parser.add_argument("--wait-seconds", type=int, default=120)
    parser.add_argument("--require-dns-catalog", action="store_true",
                        help="require the complete current catalog and the HTTP fixture's alpha row")
    parser.add_argument("--require-ns-status", action="store_true",
                        help="require the complete current NS status and 16 full-length fixture names")
    args = parser.parse_args()
    deadline = time.monotonic() + args.wait_seconds
    while not args.ready_log.exists() or "httpd is listening" not in args.ready_log.read_text():
        if time.monotonic() >= deadline:
            raise TimeoutError("guest did not start its keyhole")
        time.sleep(0.1)

    metrics(args.port, args.require_dns_catalog, args.require_ns_status)
    print("PASS: initial metrics request", flush=True)
    # EOF before a request must close this socket while leaving the server running.
    with socket.create_connection(("127.0.0.1", args.port), timeout=12) as client:
        client.shutdown(socket.SHUT_WR)
    metrics(args.port, args.require_dns_catalog, args.require_ns_status)
    print("PASS: EOF followed by a successful metrics request", flush=True)

    # Abortive closes exercise peers disappearing during accept/read, rather
    # than only sending a graceful FIN after the server starts its receive.
    for _ in range(4):
        with socket.create_connection(("127.0.0.1", args.port), timeout=12) as client:
            client.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    metrics(args.port, args.require_dns_catalog, args.require_ns_status)
    print("PASS: abortive peer closes followed by a successful metrics request", flush=True)

    # Hold this connection open while a healthy client queues behind it. The
    # healthy request must finish without the idle peer voluntarily closing.
    with socket.create_connection(("127.0.0.1", args.port), timeout=12):
        metrics(args.port, args.require_dns_catalog, args.require_ns_status)
    print("PASS: idle client timed out without preventing subsequent metrics", flush=True)
    metrics(args.port, args.require_dns_catalog, args.require_ns_status)


if __name__ == "__main__":
    main()
