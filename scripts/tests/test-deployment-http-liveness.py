#!/usr/bin/env python3
"""Probe deployment ingress rejection and serial deadline recovery.

Normal requests get three 12-second attempts, separated by three seconds to
allow serial listener handoff. A trickled body must be rejected within 35
seconds, against the guest's existing 30-second total request budget. This
does not qualify concurrent-client fairness or accepted signed deployments.
"""

import argparse
import json
from pathlib import Path
import socket
import signal
import struct
import time


STATUS_REQUEST = (
    b"GET /v1/deployments/audit_missing HTTP/1.1\r\n"
    b"Host: localhost\r\nConnection: close\r\n\r\n"
)


def read_response(client, deadline):
    response = bytearray()
    body_start = None
    length = None
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("deployment HTTP response deadline expired")
        client.settimeout(remaining)
        chunk = client.recv(4096)
        if not chunk:
            raise EOFError("deployment ingress closed without a complete reply")
        response.extend(chunk)
        assert len(response) <= 65536, "oversized deployment HTTP response"
        if body_start is None and b"\r\n\r\n" in response:
            header, _ = bytes(response).split(b"\r\n\r\n", 1)
            body_start = len(header) + 4
            length = int(next(
                line.split(b":", 1)[1].strip()
                for line in header.split(b"\r\n")[1:]
                if line.lower().startswith(b"content-length:")
            ))
            assert 0 < length < 65536
        if body_start is not None and len(response) >= body_start + length:
            header, body = bytes(response).split(b"\r\n\r\n", 1)
            assert header.startswith(b"HTTP/1.1 "), header
            return int(header.split(b" ", 2)[1]), json.loads(body[:length])


def request(port, payload, expected):
    for attempt in range(3):
        try:
            deadline = time.monotonic() + 12
            with socket.create_connection(("127.0.0.1", port), timeout=12) as client:
                client.sendall(payload)
                status, body = read_response(client, deadline)
            assert status == expected, (status, body)
            assert "error" in body, body
            return
        except (OSError, EOFError) as error:
            if attempt == 2:
                raise
            print(f"Deployment transport {type(error).__name__}; retry {attempt + 1}/2", flush=True)
            time.sleep(3)


def status(port):
    request(port, STATUS_REQUEST, 404)


def trickled_body(port):
    # One connection with an incomplete but length-valid signed-descriptor
    # envelope. Individual receive waits must not restart the total deadline.
    time.sleep(0.5)
    with socket.create_connection(("127.0.0.1", port), timeout=12) as client:
        start = time.monotonic()
        deadline = start + 35
        client.sendall(
            b"POST /v1/deployments HTTP/1.1\r\nHost: localhost\r\n"
            b"Content-Length: 3584\r\n\r\nx"
        )
        client.settimeout(0.1)
        next_byte = start + 1
        first = b""
        while not first:
            now = time.monotonic()
            if now >= deadline:
                raise TimeoutError("trickled deployment body exceeded 35 seconds")
            if now >= next_byte:
                client.sendall(b"x")
                next_byte = now + 1
            try:
                first = client.recv(1, socket.MSG_PEEK)
                if not first:
                    raise EOFError("trickled body closed without deadline rejection")
            except socket.timeout:
                continue
        code, body = read_response(client, deadline)
        elapsed = time.monotonic() - start
        assert code == 400, (code, body)
        # Ensure this sampled the total body deadline rather than immediate
        # malformed-header rejection or the five-second header deadline.
        assert 25 <= elapsed <= 35, elapsed
        status(port)
        print(f"PASS: trickled deployment body rejected in {elapsed:.2f}s; ingress recovered", flush=True)


class ProbeDeadlineExceeded(RuntimeError):
    pass


def probe_expired(_signum, _frame):
    raise ProbeDeadlineExceeded("deployment liveness probe exceeded its total wall-clock budget")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--ready-log", type=Path, required=True)
    parser.add_argument("--wait-seconds", type=int, default=30)
    parser.add_argument("--probe-seconds", type=int, default=120)
    args = parser.parse_args()
    if args.probe_seconds <= 0 or args.wait_seconds <= 0:
        parser.error("probe and readiness deadlines must be positive")
    signal.signal(signal.SIGALRM, probe_expired)
    signal.alarm(args.probe_seconds)
    deadline = time.monotonic() + args.wait_seconds
    while not args.ready_log.exists() or "[deployd] listening port=" not in args.ready_log.read_text():
        if time.monotonic() >= deadline:
            raise TimeoutError("deployment ingress did not start")
        time.sleep(0.1)

    status(args.port)
    print("PASS: deployment status control", flush=True)
    for headers in [
        b"Content-Length: 3584\r\nContent-Length: 3584\r\n",
        b"Content-Length: 3584\r\nTransfer-Encoding: chunked\r\n",
        b"Content-Length: 3584 junk\r\n",
        b"Content-Length: 999999999\r\n",
    ]:
        request(args.port, b"POST /v1/deployments HTTP/1.1\r\n" + headers + b"\r\n", 400)
        status(args.port)
    print("PASS: deployment malformed/oversized framing and healthy controls", flush=True)
    request(args.port, b"POST /v1/deployments HTTP/1.1\r\nContent-Length: 3584\r\n\r\n" + bytes(3584), 403)
    status(args.port)
    print("PASS: complete invalid deployment body rejected; ingress recovered", flush=True)

    with socket.create_connection(("127.0.0.1", args.port), timeout=12) as client:
        client.shutdown(socket.SHUT_WR)
    status(args.port)
    print("PASS: deployment EOF recovery", flush=True)
    with socket.create_connection(("127.0.0.1", args.port), timeout=12) as client:
        client.sendall(STATUS_REQUEST)
        client.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, struct.pack("ii", 1, 0))
    status(args.port)
    print("PASS: deployment abortive reply recovery", flush=True)
    with socket.create_connection(("127.0.0.1", args.port), timeout=12):
        status(args.port)
    print("PASS: deployment idle-peer deadline recovery", flush=True)
    trickled_body(args.port)


if __name__ == "__main__":
    main()
