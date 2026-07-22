#!/usr/bin/env python3
"""Decode the atomic DEBUG_TRACE memory image captured from QEMU."""

import struct
import sys

CAPACITY = 16_384
SLOT_WORDS = 7
TAGS = {
    0xC00001: "CQ_WAIT_ENTER",
    0xC00002: "CQ_WAIT_RESUME",
    0xC00003: "CQ_WAIT_FAST",
    0xC00004: "CQ_WAIT_GUARD",
    0xC00010: "COMPLETE",
    0xC00011: "COMPLETE_DETACHED",
    0xC00012: "WAKE",
    0xC00013: "SIGNAL_CQ",
    0xC00020: "WAKER_NOTIFY",
    0xC00030: "SUBMIT_TIMER_OK",
    0xC00031: "TIMER_FIRED",
    0xC00032: "TIMER_ARMED",
    0xC00033: "TIMER_STOPPED",
    0xC00040: "SCHED_DISPATCH",
    0xC00041: "SCHED_ADMIT",
    0xC00050: "STACK_ARENA_WAIT",
    0xC00051: "STACK_ARENA_ACQUIRED",
    0xC00052: "STACK_ARENA_RELEASED",
    0xC00060: "DEVICE_PHASE",
}


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} TRACE.bin", file=sys.stderr)
        return 2
    data = open(sys.argv[1], "rb").read()
    expected_size = 8 + CAPACITY * SLOT_WORDS * 8
    if len(data) < expected_size:
        print(f"short trace image: {len(data)} bytes, expected {expected_size}", file=sys.stderr)
        return 1
    total = struct.unpack_from("<Q", data)[0]
    retained = min(total, CAPACITY)
    print(f"[TRACE] total={total} retained={retained}")
    for logical in range(total - retained, total):
        offset = 8 + (logical % CAPACITY) * SLOT_WORDS * 8
        sequence, tick, tag, lp, a, b, c = struct.unpack_from("<7Q", data, offset)
        if sequence != ((logical * 2 + 2) & 0xFFFFFFFFFFFFFFFF):
            continue
        print(
            f"[TRACE] tick={tick} lp={lp} {TAGS.get(tag, '?')} "
            f"a={a:#x} b={b:#x} c={c:#x}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
