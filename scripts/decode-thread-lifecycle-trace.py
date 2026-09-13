#!/usr/bin/env python3
"""Decode the atomic THREAD_LIFECYCLE_TRACE image captured from QEMU."""

import struct
import sys

CAPACITY = 8_192
SLOT_WORDS = 13
PHASES = {
    1: "STAGE",
    2: "REAP_DEFER",
    3: "REAP_RECLAIM",
    4: "STACK_DEALLOCATE",
}
NONE = 0xFFFF_FFFF_FFFF_FFFF


def optional(value: int) -> str:
    return "none" if value == NONE else str(value)


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} TRACE.bin", file=sys.stderr)
        return 2
    with open(sys.argv[1], "rb") as trace_file:
        data = trace_file.read()
    expected_size = 8 + CAPACITY * SLOT_WORDS * 8
    if len(data) < expected_size:
        print(
            f"short lifecycle trace image: {len(data)} bytes, expected {expected_size}",
            file=sys.stderr,
        )
        return 1
    total = struct.unpack_from("<Q", data)[0]
    retained = min(total, CAPACITY)
    print(f"[THREAD_LIFECYCLE] total={total} retained={retained}")
    for logical in range(total - retained, total):
        offset = 8 + (logical % CAPACITY) * SLOT_WORDS * 8
        (
            sequence,
            tick,
            phase,
            lp,
            queue_lp,
            tid,
            generation,
            asid,
            stack_base,
            stack_end,
            current_sp,
            on_cpu,
            abort_owner,
        ) = struct.unpack_from("<13Q", data, offset)
        if sequence != ((logical * 2 + 2) & NONE):
            continue
        print(
            f"[THREAD_LIFECYCLE] sequence={logical} tick={tick} lp={lp} "
            f"phase={PHASES.get(phase, '?')} queue_lp={optional(queue_lp)} "
            f"tid={optional(tid)} generation={generation} asid={asid} "
            f"stack={stack_base:#x}..{stack_end:#x} current_sp={current_sp:#x} "
            f"on_cpu={on_cpu} abort_owner={optional(abort_owner)}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
