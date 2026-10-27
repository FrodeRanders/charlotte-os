#!/usr/bin/env python3
"""Verify native management mTLS, exact enrollment and role boundaries.

Use disposable credentials from prepare-management-tls-fixture.py. Every check
uses a fresh connection; rejection must precede any HTTP response. A healthy
reader request after each rejection verifies that the listener remains usable.
"""
import argparse
import os
from pathlib import Path
import socket
import ssl
import subprocess
import time

from management_tls import connect, context


OBSERVE = b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
STATUS = b"GET /v1/deployments/audit_missing HTTP/1.1\r\nHost: localhost\r\n\r\n"
MUTATE = b"POST /v1/deployments HTTP/1.1\r\nHost: localhost\r\nContent-Length: 3584\r\n\r\nx"


def response(client, payload, budget=12):
    end = time.monotonic() + budget
    client.sendall(payload)
    data = bytearray()
    while True:
        client.settimeout(max(0.001, end - time.monotonic()))
        chunk = client.recv(4096)
        if not chunk:
            raise EOFError("incomplete management response")
        data.extend(chunk)
        assert len(data) <= 1024 * 1024, "oversized management response"
        if b"\r\n\r\n" in data:
            header, body = bytes(data).split(b"\r\n\r\n", 1)
            size = int(next(line.split(b":", 1)[1] for line in header.split(b"\r\n")
                            if line.lower().startswith(b"content-length:")))
            if len(body) >= size:
                return int(header.split(b" ", 2)[1]), body[:size]


def healthy(port, payload, code, role="reader"):
    # A serial keyhole recreates its listener after logical close. Retry only
    # pre-HTTP transport admission, never a submitted request or TLS verification.
    for attempt in range(3):
        try:
            client = connect(port, role=role)
            break
        except ssl.SSLCertVerificationError:
            raise
        except ConnectionError:
            if attempt == 2:
                raise
            time.sleep(0.25)
    with client:
        assert client.version() == "TLSv1.3", client.version()
        assert client.cipher()[0] == "TLS_AES_128_GCM_SHA256", client.cipher()
        assert client.selected_alpn_protocol() == "http/1.1"
        actual, body = response(client, payload)
        assert actual == code, (actual, body)
    return body


def rejected(port, policy, payload, server_name="127.0.0.1", verification=False):
    time.sleep(0.25)
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=12) as raw:
            with policy.wrap_socket(raw, server_hostname=server_name) as client:
                client.sendall(payload)
                assert client.recv(4096) == b"", "unauthorized peer received application bytes"
                assert not verification, "server identity rejection was not verified"
    except ssl.SSLCertVerificationError:
        return
    except (ssl.SSLError, ConnectionError):
        if verification:
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--status-port", type=int, required=True)
    parser.add_argument("--deployment-port", type=int, required=True)
    parser.add_argument("--client-tool", type=Path,
                        default=Path(__file__).resolve().parents[2] / "target/debug/cluster-sign")
    parser.add_argument("--vip")
    parser.add_argument("--monitor", type=Path)
    parser.add_argument("--netdev", default="charlotte-net")
    args = parser.parse_args()
    for port, payload, code in [(args.status_port, OBSERVE, 200),
                                (args.deployment_port, STATUS, 404)]:
        for role in ("reader", None):
            healthy(port, payload, code, role)
        print(f"PASS: port {port}: verified TLS 1.3 reader and administrator observations", flush=True)
        no_certificate = ssl.create_default_context(cafile=os.environ["CATTEN_MANAGEMENT_CA_FILE"])
        no_certificate.minimum_version = ssl.TLSVersion.TLSv1_3
        no_certificate.set_alpn_protocols(["http/1.1"])
        rejected(port, no_certificate, payload)
        healthy(port, payload, code)
        rejected(port, context("unknown"), payload)
        healthy(port, payload, code)
        rejected(port, context(), payload, "unrelated.invalid", verification=True)
        healthy(port, payload, code)
        wrong_root = ssl.create_default_context()
        wrong_root.load_cert_chain(os.environ["CATTEN_MANAGEMENT_CERT_FILE"],
                                   os.environ["CATTEN_MANAGEMENT_KEY_FILE"])
        rejected(port, wrong_root, payload, verification=True)
        healthy(port, payload, code)
        with socket.create_connection(("127.0.0.1", port), timeout=12) as raw:
            raw.sendall(payload)
            try:
                assert raw.recv(4096) == b"", "plaintext HTTP accepted"
            except ConnectionError:
                pass
        healthy(port, payload, code)
        print(f"PASS: port {port}: missing/unregistered certificate, wrong name/root and plaintext rejected; reader recovered", flush=True)
    with connect(args.deployment_port, role="reader") as client:
        code, body = response(client, MUTATE)
        assert code == 403 and body == b'{"error":"forbidden"}\n', (code, body)
    healthy(args.deployment_port, STATUS, 404)
    print("PASS: reader mutation rejected before incomplete signed body/controller dispatch", flush=True)
    environment = dict(os.environ)
    environment["CATTEN_MANAGEMENT_CERT_FILE"] = environment["CATTEN_MANAGEMENT_READER_CERT_FILE"]
    environment["CATTEN_MANAGEMENT_KEY_FILE"] = environment["CATTEN_MANAGEMENT_READER_KEY_FILE"]
    command = [str(args.client_tool), "ingress-policy-status", f"127.0.0.1:{args.deployment_port}"]
    result = subprocess.run(command, env=environment, capture_output=True, text=True, timeout=15)
    assert result.returncode == 0 or (
        "HTTP/1.1 404 Not Found" in result.stderr
        and '{"error":"ingress policy not found"}' in result.stderr
    ), (result.stdout, result.stderr)
    environment["CATTEN_MANAGEMENT_SERVER_NAME"] = "unrelated.invalid"
    result = subprocess.run(command, env=environment, capture_output=True, text=True, timeout=15)
    assert result.returncode != 0 and "verification rejected before HTTP submission" in result.stderr, result.stderr
    print("PASS: cluster-sign verified reader observation; wrong server name rejected before HTTP submission", flush=True)
    if args.vip:
        # The isolated runner has already qualified the direct deployment port.
        # Remap its existing forward to the configured VIP for this final phase.
        assert args.monitor, "VIP deployment qualification requires the isolated QEMU monitor"
        with socket.socket(socket.AF_UNIX) as monitor:
            monitor.settimeout(3)
            monitor.connect(str(args.monitor))
            def prompt():
                data = b""
                end = time.monotonic() + 3
                while not data.endswith(b"(qemu) "):
                    if time.monotonic() >= end:
                        raise TimeoutError("QEMU monitor prompt deadline expired")
                    monitor.settimeout(end - time.monotonic())
                    chunk = monitor.recv(4096)
                    if not chunk:
                        raise EOFError("QEMU monitor closed before prompt")
                    data += chunk
                    assert len(data) <= 16384
                return data
            prompt()
            for command in [f"hostfwd_remove {args.netdev} tcp::{args.deployment_port}",
                            f"hostfwd_add {args.netdev} tcp:127.0.0.1:{args.deployment_port}-{args.vip}:7444"]:
                monitor.sendall(command.encode() + b"\n")
                result = prompt()
                assert b"error" not in result.lower() and b"could not" not in result.lower(), result
        os.environ["CATTEN_MANAGEMENT_SERVER_NAME"] = args.vip
        healthy(args.status_port, OBSERVE, 200)
        healthy(args.deployment_port, STATUS, 404)
        healthy(args.deployment_port, MUTATE, 403)
        print(f"PASS: status/deployment VIP {args.vip}: verified IP identity, reader observation and mutation rejection", flush=True)


if __name__ == "__main__":
    main()
