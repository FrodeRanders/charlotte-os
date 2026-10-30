#!/usr/bin/env python3
"""Submit one signed node shutdown beside held clients; verify cooperative drain."""
import argparse
from contextlib import ExitStack
from pathlib import Path
import re
import select
import socket
import subprocess
import time

from management_tls import context, exchange


def verify(text):
    if "Kernel panic:" in text or "SELFTEST FAILED:" in text:
        raise ValueError("guest failed before verified management drain")
    for service, phase in (("deployd", "DeploymentIngress"), ("http", "HttpIngress")):
        pattern = (rf"\[{service}\] management drain (begin|complete) inputs=(\d+) "
                   + (r"controls=(\d+) " if service == "deployd" else "")
                   + r"outputs=(\d+) listeners=(\d+) closes=(\d+)(?:\r?\n|$)")
        records = list(re.finditer(pattern, text))
        if len(records) != 2 or [r[1] for r in records] != ["begin", "complete"]:
            raise ValueError(f"{service}: exact begin/complete drain observations missing")
        if int(records[0][2]) < 1:
            raise ValueError(f"{service}: no active input at shutdown")
        if any(int(value) for value in records[1].groups()[1:]):
            raise ValueError(f"{service}: reported completion with live client positions")
        outcomes = list(re.finditer(
            rf"\[shutdown\] {phase}: acknowledged=(\d+) unacknowledged=(\d+) forced=(\d+) lifecycle_status=(\d+)(?:\r?\n|$)",
            text[records[0].start():]))
        if len(outcomes) != 1 or outcomes[0].groups() != ("1", "0", "0", "2"):
            raise ValueError(f"{service}: exact cooperative retirement was not confirmed")
        if records[0].start() + outcomes[0].start() < records[1].end():
            raise ValueError(f"{service}: retirement preceded client drain")
        if text.find("[shutdown] POWER-OFF REQUESTED via PSCI") < records[0].start() + outcomes[0].end():
            raise ValueError(f"{service}: power-off preceded cooperative retirement")
    if "[shutdown] POWER-OFF REQUESTED via PSCI" not in text:
        raise ValueError("verified platform power-off is missing")


def submit(args):
    end = time.monotonic() + 45
    def remaining():
        left = end - time.monotonic()
        if left <= 0:
            raise TimeoutError("management drain exhausted its original host budget")
        return left
    # The kernel's self-test verdict can precede management listener setup.
    # Observe startup hints before the first network attempt; TLS still requires
    # a fresh time reply, and mutation is never retried after submission.
    while True:
        text = args.serial_log.read_text()
        if ("[http] listening port=443; bounded client reactor" in text
                and "[deployd] listening port=7444" in text
                and re.search(r"\[time\] synchronized unix_s=[1-9][0-9]*(?:\s|$)", text)
                and re.search(r"\[dns\] RAFT STATE state=Leader\b", text)):
            break
        remaining()
        time.sleep(0.05)  # Bounded host-log observation has no readiness callback.
    policy = context("reader")
    # Healthy observations precede held peers and mutation admission. Transport
    # admission retry lives inside exchange; accepted HTTP is never replayed.
    exchange(policy, args.status_port,
             b"GET /metrics HTTP/1.1\r\nHost: localhost\r\n\r\n", 200, remaining())
    exchange(policy, args.deployment_port,
             b"GET /v1/deployments/audit_missing HTTP/1.1\r\nHost: localhost\r\n\r\n",
             404, remaining())
    with ExitStack() as held:
        peers = [held.enter_context(socket.create_connection(("127.0.0.1", port), timeout=remaining()))
                 for port in (args.status_port, args.deployment_port) for _ in range(2)]
        time.sleep(0.2)  # Deliver held TCP peers before the sole mutation request.
        print("management drain: holding two idle TLS peers on each ingress", flush=True)
        result = subprocess.run([str(args.client_tool), "shutdown-notify", str(args.intent),
                                 f"127.0.0.1:{args.deployment_port}"],
                                capture_output=True, text=True, timeout=remaining())
        print(result.stdout, end="", flush=True)
        if result.returncode:
            raise RuntimeError(f"single shutdown submission failed; no replay: {result.stderr}")
        while peers:
            ready, _, _ = select.select(peers, [], [], remaining())
            for peer in ready:
                if peer.recv(4096):
                    raise ValueError("idle TLS peer received unexpected wire output")
                peers.remove(peer)
        print("PASS: signed shutdown submitted once; all four held peers closed", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest="mode", required=True)
    send = modes.add_parser("submit")
    send.add_argument("--intent", type=Path, required=True)
    send.add_argument("--client-tool", type=Path, required=True)
    send.add_argument("--status-port", type=int, required=True)
    send.add_argument("--deployment-port", type=int, required=True)
    send.add_argument("--serial-log", type=Path, required=True)
    check = modes.add_parser("verify")
    check.add_argument("serial_log", type=Path)
    args = parser.parse_args()
    if args.mode == "submit":
        submit(args)
    else:
        verify(args.serial_log.read_text())
        print("PASS: active status/deployment clients drained before cooperative retirement and PSCI", flush=True)


if __name__ == "__main__":
    main()
