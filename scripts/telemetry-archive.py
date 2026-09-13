#!/usr/bin/env python3
"""Decode the observe service's durable resource archive from a disk image.

Usage:
    python3 scripts/telemetry-archive.py <image> [--json]

The observe service rewrites a ring of sixteen 8 KiB object-store chunks under
reserved IDs 0xfffc_0000_0000_0001..16 (see
docs/architecture/adaptive-resource-policy.md). This tool reassembles the
records, orders them by archive session and sequence, and prints a table of
system resource samples: monotonic ticks, thread and domain counts, owned
frames, CPU load counters, heap pressure, reserved stack pages, and
touched/high-water maxima.

Records overwritten after the ring wraps are simply absent; sequence gaps in
the same session reveal where that happened. `--json` emits one object per
line for pipe-based analysis.
"""

import importlib.util
import json
import os
import struct
import sys

_spec = importlib.util.spec_from_file_location(
    "fs_inspect", os.path.join(os.path.dirname(os.path.abspath(__file__)), "fs-inspect.py")
)
_fs_inspect = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_fs_inspect)
ObjectStore = _fs_inspect.ObjectStore

ARCHIVE_BASE_ID = 0xFFFC_0000_0000_0001
ARCHIVE_CHUNKS = 16
ARCHIVE_MAGIC = 0x3130_4843_5241_4343  # "CCARCH01"
ARCHIVE_VERSION = 2
HEADER_WORDS = 7
RECORD_WORDS = 14


class ArchiveError(Exception):
    pass


def decode_chunk(data):
    if len(data) < HEADER_WORDS * 8:
        raise ArchiveError("chunk shorter than its header")
    header = struct.unpack_from("<7Q", data)
    if header[0] != ARCHIVE_MAGIC:
        raise ArchiveError(f"bad chunk magic {header[0]:#x}")
    if header[1] != ARCHIVE_VERSION:
        raise ArchiveError(f"unsupported archive version {header[1]}")
    _, _, chunk_index, session_ticks, _, record_count, frequency_hz = header
    available = (len(data) - HEADER_WORDS * 8) // (RECORD_WORDS * 8)
    count = min(record_count, available)
    records = []
    for index in range(count):
        base = HEADER_WORDS * 8 + index * RECORD_WORDS * 8
        words = struct.unpack_from("<14Q", data, base)
        records.append(
            {
                "session_ticks": session_ticks,
                "chunk": chunk_index,
                "frequency_hz": frequency_hz,
                "sequence": words[0],
                "ticks": words[1],
                "threads": words[2],
                "domains": words[3],
                "owned_frames": words[4],
                "stack_pages": words[5],
                "stack_used_high_water": words[6],
                "threads_high_water": words[7],
                "free_frames": words[8],
                "usable_frames": words[9],
                "logical_processors": words[10],
                "cpu_busy_ticks": words[11],
                "heap_allocated_bytes": words[12],
                "heap_peak_bytes": words[13],
            }
        )
    return records


def read_archive(path):
    store = ObjectStore(path)
    if not store.is_formatted:
        raise ArchiveError("image is not a valid v3 object store")
    records = []
    for index in range(ARCHIVE_CHUNKS):
        obj_id = ARCHIVE_BASE_ID + index
        if obj_id not in store.objects:
            continue
        data = store.read_object(obj_id)
        if data is None:
            raise ArchiveError(f"object {obj_id:#x} disappeared while reading")
        try:
            records.extend(decode_chunk(data))
        except ArchiveError as error:
            print(f"warning: chunk {index}: {error}", file=sys.stderr)
    records.sort(key=lambda record: (record["session_ticks"], record["sequence"]))
    return records


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 1
    path = sys.argv[1]
    if not os.path.exists(path):
        print(f"File not found: {path}", file=sys.stderr)
        return 1
    as_json = "--json" in sys.argv[2:]

    try:
        records = read_archive(path)
    except (ArchiveError, OSError, struct.error) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    if not records:
        print("no telemetry archive found", file=sys.stderr)
        return 1

    if as_json:
        for record in records:
            print(json.dumps(record))
        return 0

    print(
        f"{'session':>10} {'seq':>6} {'time_s':>8} {'threads':>7} {'domains':>7} "
        f"{'frames':>8} {'free':>8} {'cpu%':>6} {'heap':>10} "
        f"{'stack':>6} {'used':>4} {'threads_hw':>10}"
    )
    previous = None
    for record in records:
        frequency = record["frequency_hz"] or 1
        seconds = record["ticks"] / frequency
        cpu_percent = 0.0
        if previous is not None and previous["session_ticks"] == record["session_ticks"]:
            elapsed = record["ticks"] - previous["ticks"]
            busy = record["cpu_busy_ticks"] - previous["cpu_busy_ticks"]
            processors = record["logical_processors"] or 1
            if elapsed > 0 and busy >= 0:
                cpu_percent = min(100.0, busy * 100.0 / (elapsed * processors))
        print(
            f"{record['session_ticks']:>10} {record['sequence']:>6} {seconds:>8.1f} "
            f"{record['threads']:>7} {record['domains']:>7} {record['owned_frames']:>8} "
            f"{record['free_frames']:>8} {cpu_percent:>6.1f} "
            f"{record['heap_allocated_bytes']:>10} "
            f"{record['stack_pages']:>6} {record['stack_used_high_water']:>4} "
            f"{record['threads_high_water']:>10}"
        )
        previous = record
    return 0


if __name__ == "__main__":
    sys.exit(main())
