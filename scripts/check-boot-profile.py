#!/usr/bin/env python3
"""Reject a reused kernel from the wrong boot mode and native fixtures in minimal ELFs."""

import argparse
from pathlib import Path
import struct

MODES = {"minimal": b"CHARLOTTE_BOOT_MODE=minimal", "test": b"CHARLOTTE_BOOT_MODE=tests"}


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


def validate(data, mode):
    if MODES[mode] not in data or any(marker in data for key, marker in MODES.items() if key != mode):
        raise ValueError(f"kernel does not match requested {mode} boot mode; rebuild it")
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
        validate(args.kernel.read_bytes(), args.mode)
    except (ValueError, OSError, struct.error, IndexError) as error:
        parser.exit(1, f"error: {error}\n")
    print(f">>> Kernel boot profile verified: {args.mode}.")


if __name__ == "__main__":
    main()
