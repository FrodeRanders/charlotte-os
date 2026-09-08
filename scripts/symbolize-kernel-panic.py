#!/usr/bin/env python3
"""Symbolize CharlotteOS raw kernel panic backtraces offline."""

from __future__ import annotations

import argparse
import bisect
import hashlib
import pathlib
import re
import shutil
import subprocess
import sys
from dataclasses import dataclass


FRAME_RE = re.compile(r"^\s*#(?P<depth>\d+)\s+(?P<address>0x[0-9a-fA-F]+)\s*$")
ADDRESS_RE = re.compile(r"^0x[0-9a-fA-F]+$")
SHA_RE = re.compile(r"^# kernel-sha256:\s*(?P<sha>[0-9a-fA-F]{64})\s*$")


@dataclass(frozen=True)
class Symbol:
    address: int
    size: int
    kind: str
    name: str


def sha256(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_symbols(path: pathlib.Path) -> tuple[str | None, list[Symbol]]:
    expected_sha = None
    symbols: list[Symbol] = []
    with path.open(encoding="utf-8") as source:
        for raw_line in source:
            line = raw_line.rstrip("\n")
            if match := SHA_RE.match(line):
                expected_sha = match.group("sha").lower()
                continue
            if not line or line.startswith("#"):
                continue
            fields = line.split(maxsplit=3)
            if len(fields) != 4:
                continue
            address_text, size_text, kind, name = fields
            if kind.lower() not in {"t", "w"}:
                continue
            try:
                symbols.append(
                    Symbol(int(address_text, 16), int(size_text, 16), kind, name)
                )
            except ValueError:
                continue
    symbols.sort(key=lambda symbol: symbol.address)
    if not symbols:
        raise ValueError(f"no text symbols found in {path}")
    return expected_sha, symbols


def load_addresses(inputs: list[str]) -> list[tuple[str, int]]:
    frames: list[tuple[str, int]] = []
    for value in inputs:
        candidate = pathlib.Path(value)
        if candidate.is_file():
            with candidate.open(encoding="utf-8", errors="replace") as source:
                for line in source:
                    if match := FRAME_RE.match(line):
                        frames.append(
                            (
                                f"#{int(match.group('depth')):02}",
                                int(match.group("address"), 16),
                            )
                        )
        elif ADDRESS_RE.match(value):
            frames.append((f"#{len(frames):02}", int(value, 16)))
        else:
            raise ValueError(f"input is neither a log file nor a hexadecimal address: {value}")
    if not frames:
        raise ValueError("no raw panic backtrace addresses found")
    return frames


def lookup(symbols: list[Symbol], starts: list[int], address: int) -> str:
    index = bisect.bisect_right(starts, address) - 1
    if index < 0:
        return "<before first kernel text symbol>"
    symbol = symbols[index]
    offset = address - symbol.address
    suffix = f"+0x{offset:x}" if offset else ""
    if symbol.size and offset >= symbol.size:
        suffix += " (nearest preceding symbol)"
    return f"{symbol.name}{suffix}"


def print_source_locations(kernel: pathlib.Path, frames: list[tuple[str, int]]) -> None:
    lldb = shutil.which("lldb")
    if lldb is None:
        print("warning: --source requested but lldb is unavailable", file=sys.stderr)
        return
    command = [lldb, "--batch"]
    for _, address in frames:
        command.extend(["-o", f"image lookup -a {address:#x}"])
    command.append(str(kernel))
    result = subprocess.run(command, check=False, text=True, capture_output=True)
    summaries = [
        line.strip().removeprefix("Summary: ")
        for line in result.stdout.splitlines()
        if line.strip().startswith("Summary:")
    ]
    if summaries:
        print("\nDWARF source locations:")
        for (label, address), summary in zip(frames, summaries, strict=False):
            print(f"  {label} {address:#018x}  {summary}")
    elif result.stderr:
        print(result.stderr.rstrip(), file=sys.stderr)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Map CharlotteOS panic return addresses to kernel functions."
    )
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--kernel", type=pathlib.Path, help="matching unstripped catten ELF")
    source.add_argument("--symbols", type=pathlib.Path, help="generated .symbols sidecar")
    parser.add_argument(
        "--source",
        action="store_true",
        help="also ask lldb for DWARF file/line locations (requires --kernel)",
    )
    parser.add_argument("inputs", nargs="+", help="panic log files or raw 0x... addresses")
    args = parser.parse_args()

    if args.source and args.kernel is None:
        parser.error("--source requires --kernel")

    kernel = args.kernel.resolve() if args.kernel else None
    if args.symbols:
        symbol_map = args.symbols.resolve()
    elif kernel:
        symbol_map = pathlib.Path(f"{kernel}.symbols")
    elif len(args.inputs) == 1 and pathlib.Path(args.inputs[0]).is_file():
        symbol_map = pathlib.Path(f"{pathlib.Path(args.inputs[0]).resolve()}.symbols")
    else:
        parser.error("provide --kernel or --symbols when input is not one serial log")
    if not symbol_map.is_file():
        raise ValueError(f"symbol map does not exist: {symbol_map}")

    expected_sha, symbols = load_symbols(symbol_map)
    if kernel is not None:
        if not kernel.is_file():
            raise ValueError(f"kernel ELF does not exist: {kernel}")
        actual_sha = sha256(kernel)
        if expected_sha and actual_sha != expected_sha:
            raise ValueError(
                f"kernel/symbol-map SHA-256 mismatch: kernel={actual_sha}, map={expected_sha}"
            )

    frames = load_addresses(args.inputs)
    starts = [symbol.address for symbol in symbols]
    build_suffix = f", kernel SHA-256 {expected_sha}" if expected_sha else ""
    print(f"Kernel panic symbols ({symbol_map}{build_suffix}):")
    for label, address in frames:
        print(f"  {label} {address:#018x}  {lookup(symbols, starts, address)}")

    if args.source and kernel is not None:
        print_source_locations(kernel, frames)
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        raise SystemExit(1)
