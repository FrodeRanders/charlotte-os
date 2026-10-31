#!/usr/bin/env python3
"""Observe a completed minimal boot; never synthesize a self-test verdict."""

import argparse
from pathlib import Path
import re

COMPLETE = b"BOOT COMPLETE: validation passed; local storage ready; services submitted"


def validate(data):
    lines = [line.rstrip(b"\r") for line in data.split(b"\n")[:-1]]
    if b"Kernel panic:" in data or b"SELFTEST COMPLETE:" in data:
        return False
    return (any(line.endswith(b"CHARLOTTE_BOOT_MODE=minimal") for line in lines)
            and any(re.search(rb"\[boot validation\] memory passed: usable=[0-9]+ MiB, "
                              rb"free=[1-9][0-9]* frames, heap reserve=[0-9]+ MiB$", line)
                    for line in lines)
            and any(re.search(rb"\[store\] bootstrap seed complete: retained=[0-9]+ written=[0-9]+$", line)
                    for line in lines)
            and any(line.endswith(COMPLETE) for line in lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("log", type=Path)
    args = parser.parse_args()
    try:
        passed = validate(args.log.read_bytes())
    except OSError:
        passed = False
    raise SystemExit(0 if passed else 1)


if __name__ == "__main__":
    main()
