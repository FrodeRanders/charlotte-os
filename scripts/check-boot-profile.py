#!/usr/bin/env python3
"""Reject a reused kernel from the wrong boot mode and native fixtures in minimal ELFs."""

import argparse
import hashlib
import os
from pathlib import Path
import re
import struct

MODES = {"minimal": b"CHARLOTTE_BOOT_MODE=minimal", "test": b"CHARLOTTE_BOOT_MODE=tests"}


def otlp_profile(env):
    """Canonical public launch bytes, matching charlotte_launch::otlp::Profile."""
    def number(name, default):
        text = env.get(name, str(default))
        if not text or any(c < "0" or c > "9" for c in text):
            raise ValueError("OTLP budgets must be ASCII decimal")
        return int(text)
    if "CATTEN_OTLP_ENDPOINT" not in env:
        if any(name in env for name in ("CATTEN_OTLP_INTERVAL_MS", "CATTEN_OTLP_TIMEOUT_MS")):
            raise ValueError("OTLP interval/timeout require CATTEN_OTLP_ENDPOINT")
        return b""
    endpoint = env["CATTEN_OTLP_ENDPOINT"]
    if not endpoint.startswith("http://"):
        raise ValueError("CATTEN_OTLP_ENDPOINT requires explicit HTTP/IPv4")
    authority, slash, path = endpoint[7:].partition("/")
    path = ("/" + path) if slash else "/v1/metrics"
    host, colon, port = authority.partition(":")
    octets = host.split(".")
    if len(octets) != 4 or any(not p or any(c < "0" or c > "9" for c in p) for p in octets):
        raise ValueError("CATTEN_OTLP_ENDPOINT requires IPv4")
    values = [int(p) for p in octets]
    if any(v > 255 for v in values) or not 0 < values[0] < 224:
        raise ValueError("OTLP destination must be unicast IPv4")
    if colon and (not port or any(c < "0" or c > "9" for c in port)):
        raise ValueError("OTLP port must be decimal")
    port = int(port) if colon else 4318
    interval = number("CATTEN_OTLP_INTERVAL_MS", 30000)
    timeout = number("CATTEN_OTLP_TIMEOUT_MS", 3000)
    if (not 0 < port <= 65535 or not 1000 <= interval <= 86400000
            or not 100 <= timeout <= 60000 or timeout > interval
            or not path.startswith("/") or len(path) > 256
            or any(not 0x21 <= ord(c) <= 0x7e or c in "?#" for c in path)):
        raise ValueError("invalid OTLP destination or budgets")
    raw = path.encode("ascii")
    return struct.pack("<8sHHI4sHHQQ", b"COTLP001", 1, 40, 40 + len(raw), bytes(values),
                       port, len(raw), interval, timeout) + raw


def symbols(data):
    if data[:6] != b"\x7fELF\x02\x01":
        raise ValueError("expected a little-endian ELF64 kernel")
    offset = struct.unpack_from("<Q", data, 40)[0]
    size, count = struct.unpack_from("<HH", data, 58)
    sections = [struct.unpack_from("<IIQQQQIIQQ", data, offset + index * size)
                for index in range(count)]
    for section in sections:
        if section[1] != 2:  # SHT_SYMTAB
            continue
        strings = sections[section[6]]
        names = data[strings[4]:strings[4] + strings[5]]
        for cursor in range(section[4], section[4] + section[5], section[9]):
            start = struct.unpack_from("<I", data, cursor)[0]
            yield bytes(names[start:names.index(b"\0", start)])


def validate(data, mode, archive_interval_ms=None, otlp_sha256=None):
    if MODES[mode] not in data or any(marker in data for key, marker in MODES.items() if key != mode):
        raise ValueError(f"kernel does not match requested {mode} boot mode; rebuild it")
    if archive_interval_ms is not None:
        selected = re.findall(rb"CHARLOTTE_TELEMETRY_ARCHIVE_INTERVAL_MS=([0-9]+);", data)
        if not selected or any(int(value) != archive_interval_ms for value in selected):
            raise ValueError("kernel telemetry archive policy does not match requested interval; rebuild it")
    if otlp_sha256 is not None:
        selected = re.findall(rb"CHARLOTTE_OTLP_PROFILE_SHA256=([0-9a-f]{64});", data)
        if not selected or any(value.decode() != otlp_sha256 for value in selected):
            raise ValueError("kernel OTLP profile does not match requested collector policy; rebuild it")
    names = list(symbols(data))
    if not names:
        raise ValueError("kernel symbol table required for boot-profile inspection")
    if mode == "minimal":
        fixtures = [name for name in names if b"6catten" in name
                    and any(part in name for part in (b"9self_test", b"_tests", b"_for_test", b"test_"))]
        fixtures.extend(name for name in names if name in {
            b"probe_device_topology", b"test_fn", b"async_syscall_coordinator",
        })
        if fixtures or b"SELFTEST COMPLETE: passed=" in data or b"[IPC scalar submission]" in data:
            raise ValueError("native regression fixture found in minimal kernel")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kernel", type=Path, required=True)
    parser.add_argument("--mode", choices=MODES, required=True)
    args = parser.parse_args()
    try:
        selected = os.environ.get("CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS", "0")
        if not selected or any(char < "0" or char > "9" for char in selected):
            raise ValueError("CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS must be 0 or 1000..=86400000")
        interval = int(selected)
        if interval != 0 and not 1000 <= interval <= 86400000:
            raise ValueError("CATTEN_TELEMETRY_ARCHIVE_INTERVAL_MS must be 0 or 1000..=86400000")
        validate(args.kernel.read_bytes(), args.mode, interval,
                 hashlib.sha256(otlp_profile(os.environ)).hexdigest())
    except (ValueError, OSError, struct.error, IndexError) as error:
        parser.exit(1, f"error: {error}\n")
    print(f">>> Kernel boot profile verified: {args.mode}.")


if __name__ == "__main__":
    main()
