#!/usr/bin/env python3
"""Reject native kernel assembly that inherited data-section permissions."""

from pathlib import Path
import struct
import sys


def check(path):
    data = Path(path).read_bytes()
    header = struct.unpack_from("<16sHHIQQQIHHHHHH", data)
    if header[0][:6] != b"\x7fELF\x02\x01":
        raise ValueError("expected a little-endian ELF64 kernel")
    required = {
        62: {"reload_segment_regs", "syscall_entry", "isr_page_fault", "isr_synchronous_ipi"},
        183: {"ivt"},
    }.get(header[2])
    if required is None:
        raise ValueError("unsupported kernel architecture")
    segments = [
        struct.unpack_from("<IIQQQQQQ", data, header[5] + index * header[9])
        for index in range(header[10])
    ]
    sections = [
        struct.unpack_from("<IIQQQQIIQQ", data, header[6] + index * header[11])
        for index in range(header[12])
    ]
    found = set()
    for section in sections:
        if section[1] != 2:  # SHT_SYMTAB, retained in debug and release kernels.
            continue
        strings = sections[section[6]]
        names = data[strings[4]:strings[4] + strings[5]]
        for offset in range(section[4], section[4] + section[5], section[9]):
            name, _, _, index, address, size = struct.unpack_from("<IBBHQQ", data, offset)
            name = names[name:names.index(b"\0", name)].decode()
            if name not in required and not name.startswith(("isr_", "dyn_isr_")):
                continue
            found.add(name)
            if index == 0 or index >= len(sections):
                raise ValueError(f"{name}: missing native assembly definition")
            flags = sections[index][2]
            if not flags & 4 or flags & 1:  # SHF_EXECINSTR, SHF_WRITE.
                raise ValueError(f"{name}: native assembly is outside executable read-only text")
            if not any(
                segment[0] == 1 and segment[1] & 1 and not segment[1] & 2
                and segment[3] <= address
                and address + max(size, 1) <= segment[3] + segment[6]
                for segment in segments
            ):
                raise ValueError(f"{name}: no executable read-only PT_LOAD covers assembly")
    if missing := required - found:
        raise ValueError(f"missing required native assembly: {', '.join(sorted(missing))}")
    print(f">>> Kernel assembly permissions: {len(found)} native entries verified")


if __name__ == "__main__":
    try:
        check(sys.argv[1])
    except (IndexError, ValueError, struct.error, OSError) as error:
        sys.exit(f"error: kernel assembly permission check failed: {error}")
