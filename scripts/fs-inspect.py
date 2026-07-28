#!/usr/bin/env python3
"""Inspect or format a CharlotteOS v3 persistent object-store image.

Usage:
    python3 scripts/fs-inspect.py <image>                    # filesystem tree
    python3 scripts/fs-inspect.py <image> format             # destructive format
    python3 scripts/fs-inspect.py <image> tree
    python3 scripts/fs-inspect.py <image> dump <object-id>
    python3 scripts/fs-inspect.py <image> cat <path>
    python3 scripts/fs-inspect.py <image> raw <object-id>
    python3 scripts/fs-inspect.py <image> objects
    python3 scripts/fs-inspect.py <image> metadata [object-id|path]
    python3 scripts/fs-inspect.py <image> info
"""

from dataclasses import dataclass
import os
import struct
import sys
import zlib


SB_MAGIC = 0x3352_5453_424A_4F43  # "COBJSTR3"
SB_VERSION = 3
SB_SLOTS = 2
SB_LEN = 80
SB_CHECKSUM_OFFSET = 76

DIR_ENTRY_SIZE = 32
DIR_CRC_SALT = 0x3344_4952
MAX_DIRECTORY_BLOCKS = 4096
FLAG_ALLOCATED = 1

HEADER_MAGIC = 0x3244_484A_424F_4343
HEADER_VERSION = 3
HEADER_LEN = 384
HEADER_CHECKSUM_OFFSET = 112
EXTENTS_OFFSET = 128
EXTENT_SIZE = 16
MAX_EXTENTS = 16
HASH_FNV1A64 = 2

ROOT_ID = 100


def u16(data, offset):
    return struct.unpack_from("<H", data, offset)[0]


def u32(data, offset):
    return struct.unpack_from("<I", data, offset)[0]


def u64(data, offset):
    return struct.unpack_from("<Q", data, offset)[0]


def crc32(data):
    return zlib.crc32(data) & 0xFFFF_FFFF


def fnv1a64(data):
    value = 0xCBF2_9CE4_8422_2325
    for byte in data:
        value ^= byte
        value = (value * 0x100_0000_01B3) & 0xFFFF_FFFF_FFFF_FFFF
    return value


@dataclass(frozen=True)
class Layout:
    bitmap_lba: int
    bitmap_blocks: int
    directory_lba: int
    directory_blocks: int
    data_lba: int

    @classmethod
    def for_device(cls, block_size, total_blocks):
        bitmap_blocks = ((total_blocks + 7) // 8 + block_size - 1) // block_size
        directory_blocks = min(max(total_blocks // 2048, 1), MAX_DIRECTORY_BLOCKS)
        bitmap_lba = SB_SLOTS
        directory_lba = bitmap_lba + bitmap_blocks
        data_lba = directory_lba + 2 * directory_blocks
        if data_lba + 2 > total_blocks:
            raise ValueError("image is too small for the v3 object-store layout")
        return cls(bitmap_lba, bitmap_blocks, directory_lba, directory_blocks, data_lba)

    def entry_count(self, block_size):
        return self.directory_blocks * block_size // DIR_ENTRY_SIZE


@dataclass(frozen=True)
class Superblock:
    slot: int
    generation: int
    block_size: int
    total_blocks: int
    next_id: int
    layout: Layout


@dataclass(frozen=True)
class DirectoryEntry:
    obj_id: int = 0
    flags: int = 0
    generation: int = 0
    header_lba: int = 0
    header_blocks: int = 0


@dataclass(frozen=True)
class ObjectRecord:
    entry: DirectoryEntry
    data_len: int
    allocated_len: int
    data_hash: int
    extents: tuple


class ObjectStore:
    """Strict, read-only parser for object-store format v3."""

    def __init__(self, path):
        self.path = path
        with open(path, "rb") as image:
            self.data = bytearray(image.read())
        self.errors = []
        self.superblock = None
        self.objects = {}
        self.block_size = 512
        self.total_blocks = len(self.data) // self.block_size
        self.next_id = 0
        self.layout = None
        self._parse()

    @property
    def is_formatted(self):
        return self.superblock is not None

    def _block_slice(self, lba, blocks=1):
        if lba < 0 or blocks < 0:
            raise ValueError("negative disk range")
        start = lba * self.block_size
        end = start + blocks * self.block_size
        if end > len(self.data):
            raise ValueError(f"disk range {lba}+{blocks} lies beyond the image")
        return self.data[start:end]

    def _decode_superblock(self, slot, block_size):
        start = slot * block_size
        if start + SB_LEN > len(self.data):
            return None
        data = self.data[start : start + 512]
        if (
            u64(data, 0) != SB_MAGIC
            or u32(data, 8) != SB_VERSION
            or u32(data, 12) != SB_LEN
            or u32(data, SB_CHECKSUM_OFFSET) != crc32(data[:SB_CHECKSUM_OFFSET])
        ):
            return None
        block_size = u32(data, 24)
        total_blocks = u32(data, 28)
        if block_size < 512 or block_size > 4096:
            return None
        layout = Layout(
            bitmap_lba=u64(data, 40),
            bitmap_blocks=u32(data, 48),
            directory_blocks=u32(data, 52),
            directory_lba=u64(data, 56),
            data_lba=u64(data, 64),
        )
        return Superblock(
            slot=slot,
            generation=u64(data, 16),
            block_size=block_size,
            total_blocks=total_blocks,
            next_id=u64(data, 32),
            layout=layout,
        )

    def _parse(self):
        candidates = []
        for candidate_size in (512, 1024, 2048, 4096):
            for slot in range(SB_SLOTS):
                sb = self._decode_superblock(slot, candidate_size)
                if sb is not None and sb.block_size == candidate_size:
                    candidates.append(sb)
        if not candidates:
            return
        sb = max(candidates, key=lambda item: item.generation)
        if sb.total_blocks * sb.block_size > len(self.data):
            self.errors.append("superblock device size exceeds image size")
            return
        try:
            expected = Layout.for_device(sb.block_size, sb.total_blocks)
        except ValueError as error:
            self.errors.append(str(error))
            return
        if sb.layout != expected:
            self.errors.append(f"stored layout {sb.layout} does not match expected {expected}")
            return
        self.superblock = sb
        self.block_size = sb.block_size
        self.total_blocks = sb.total_blocks
        self.next_id = sb.next_id
        self.layout = sb.layout
        self._parse_directory()

    @staticmethod
    def _decode_directory(data):
        if data == bytes(DIR_ENTRY_SIZE):
            return DirectoryEntry()
        if u32(data, 28) != (crc32(data[:28]) ^ DIR_CRC_SALT):
            return None
        return DirectoryEntry(u64(data, 0), u32(data, 8), u32(data, 12), u64(data, 16), u32(data, 24))

    @staticmethod
    def _newest(first, second):
        if first is None:
            return second or DirectoryEntry()
        if second is None:
            return first
        return first if first.generation >= second.generation else second

    def _parse_directory(self):
        first = self._block_slice(self.layout.directory_lba, self.layout.directory_blocks)
        second = self._block_slice(
            self.layout.directory_lba + self.layout.directory_blocks,
            self.layout.directory_blocks,
        )
        used_blocks = set(range(self.layout.data_lba))
        for index in range(self.layout.entry_count(self.block_size)):
            offset = index * DIR_ENTRY_SIZE
            entry = self._newest(
                self._decode_directory(bytes(first[offset : offset + DIR_ENTRY_SIZE])),
                self._decode_directory(bytes(second[offset : offset + DIR_ENTRY_SIZE])),
            )
            if entry.obj_id == 0:
                continue
            if not entry.flags & FLAG_ALLOCATED:
                self.errors.append(f"directory slot {index}: live ID lacks allocated flag")
                continue
            try:
                record = self._parse_header(entry)
            except ValueError as error:
                self.errors.append(f"object {entry.obj_id}: {error}")
                continue
            if entry.obj_id in self.objects:
                self.errors.append(f"duplicate object ID {entry.obj_id}")
                continue
            ranges = [(entry.header_lba, entry.header_blocks), *record.extents]
            claimed = {
                block
                for lba, blocks in ranges
                for block in range(lba, lba + blocks)
            }
            if len(claimed) != sum(blocks for _, blocks in ranges) or claimed & used_blocks:
                self.errors.append(f"object {entry.obj_id}: overlapping allocation")
                continue
            used_blocks.update(claimed)
            self.objects[entry.obj_id] = record
        bitmap = self._block_slice(self.layout.bitmap_lba, self.layout.bitmap_blocks)
        for block in used_blocks:
            if not bitmap[block // 8] & (1 << (block % 8)):
                self.errors.append(f"allocation bitmap omits reachable block {block}")

    def _parse_header(self, entry):
        if entry.header_lba < self.layout.data_lba or entry.header_blocks == 0:
            raise ValueError("invalid header location")
        data = bytes(self._block_slice(entry.header_lba, entry.header_blocks))
        if len(data) < HEADER_LEN:
            raise ValueError("short header")
        header_len = u16(data, 10)
        if (
            u64(data, 0) != HEADER_MAGIC
            or u16(data, 8) != HEADER_VERSION
            or header_len < HEADER_LEN
            or header_len > len(data)
            or u16(data, 58) != HASH_FNV1A64
        ):
            raise ValueError("unsupported or malformed header")
        checksum_data = bytearray(data[:header_len])
        stored_checksum = u32(checksum_data, HEADER_CHECKSUM_OFFSET)
        struct.pack_into("<I", checksum_data, HEADER_CHECKSUM_OFFSET, 0)
        if stored_checksum != crc32(checksum_data):
            raise ValueError("header checksum mismatch")
        if u64(data, 16) != entry.obj_id or u64(data, 24) & 0xFFFF_FFFF != entry.generation:
            raise ValueError("directory/header identity or generation mismatch")
        if u32(data, 60) != entry.header_blocks:
            raise ValueError("directory/header block count mismatch")
        extent_count = u16(data, 56)
        if extent_count > MAX_EXTENTS:
            raise ValueError("too many extents")
        extents = []
        for index in range(extent_count):
            offset = EXTENTS_OFFSET + index * EXTENT_SIZE
            lba, blocks = u64(data, offset), u32(data, offset + 8)
            if blocks == 0 or lba < self.layout.data_lba or lba + blocks > self.total_blocks:
                raise ValueError(f"invalid extent {index}")
            extents.append((lba, blocks))
        data_len = u64(data, 32)
        allocated_len = u64(data, 40)
        actual_allocated = sum(blocks for _, blocks in extents) * self.block_size
        if data_len > allocated_len or allocated_len != actual_allocated:
            raise ValueError("inconsistent object lengths")
        return ObjectRecord(entry, data_len, allocated_len, u64(data, 80), tuple(extents))

    def read_object(self, obj_id, verify=True):
        record = self.objects.get(obj_id)
        if record is None:
            return None
        remaining = record.data_len
        chunks = []
        for lba, blocks in record.extents:
            chunk = bytes(self._block_slice(lba, blocks))
            used = min(remaining, len(chunk))
            chunks.append(chunk[:used])
            remaining -= used
        if remaining:
            raise ValueError(f"object {obj_id}: extents are shorter than data length")
        result = b"".join(chunks)
        if verify and fnv1a64(result) != record.data_hash:
            raise ValueError(f"object {obj_id}: FNV-1a content hash mismatch")
        return result


class Filesystem:
    FLAG_DIR = 1

    def __init__(self, store):
        self.store = store

    def _decode_dir(self, data):
        entries, position = [], 0
        while position + 4 <= len(data):
            name_len = u32(data, position)
            if name_len == 0:
                break
            end = position + 4 + name_len + 20
            if end > len(data):
                raise ValueError("truncated filesystem directory entry")
            name = data[position + 4 : position + 4 + name_len].decode("utf-8", errors="replace")
            base = position + 4 + name_len
            entries.append((name, u64(data, base), u32(data, base + 8), u64(data, base + 12)))
            position = end
        return entries

    def list_dir(self, obj_id):
        data = self.store.read_object(obj_id)
        return [] if data is None else self._decode_dir(data)

    def tree(self, obj_id=ROOT_ID, prefix="", name="/", visited=None):
        visited = set() if visited is None else visited
        print(f"{prefix}{name}  [{obj_id}]")
        if obj_id in visited:
            print(f"{prefix}    (cycle)")
            return
        visited.add(obj_id)
        entries = self.list_dir(obj_id)
        for index, (entry_name, entry_id, flags, size) in enumerate(entries):
            last = index == len(entries) - 1
            connector = "└── " if last else "├── "
            child_prefix = prefix + ("    " if last else "│   ")
            if flags & self.FLAG_DIR:
                self.tree(entry_id, child_prefix, f"{entry_name}/", visited)
            else:
                print(f"{prefix}{connector}{entry_name}  [{entry_id}]  {format_size(size)}")

    def find(self, path):
        current, result = ROOT_ID, None
        for part in (part for part in path.strip("/").split("/") if part):
            result = next(
                ((obj_id, flags, size) for name, obj_id, flags, size in self.list_dir(current) if name == part),
                None,
            )
            if result is None:
                return None
            current = result[0]
        return result


def format_size(size):
    if size == 0:
        return "empty"
    if size < 1024:
        return f"{size}B"
    if size < 1024 * 1024:
        return f"{size / 1024:.1f}K"
    return f"{size / (1024 * 1024):.1f}M"


def describe_flags(value, known, zero_name=None):
    names = [name for bit, name in known if value & bit]
    known_mask = 0
    for bit, _ in known:
        known_mask |= bit
    unknown = value & ~known_mask
    if unknown:
        names.append(f"UNKNOWN({unknown:#x})")
    if not names and zero_name is not None:
        names.append(zero_name)
    return "|".join(names) if names else "none"


def encode_superblock(block_size, total_blocks, layout, generation=1, next_id=1):
    block = bytearray(block_size)
    struct.pack_into("<QIIQIIQ", block, 0, SB_MAGIC, SB_VERSION, SB_LEN, generation, block_size, total_blocks, next_id)
    struct.pack_into(
        "<QIIQQI",
        block,
        40,
        layout.bitmap_lba,
        layout.bitmap_blocks,
        layout.directory_blocks,
        layout.directory_lba,
        layout.data_lba,
        0,
    )
    struct.pack_into("<I", block, SB_CHECKSUM_OFFSET, crc32(block[:SB_CHECKSUM_OFFSET]))
    return block


def cmd_format(store):
    block_size = 512
    if len(store.data) % block_size:
        raise ValueError("image size is not a multiple of 512 bytes")
    total_blocks = len(store.data) // block_size
    layout = Layout.for_device(block_size, total_blocks)
    data = bytearray(len(store.data))
    reserved = layout.data_lba
    bitmap_start = layout.bitmap_lba * block_size
    for block in range(reserved):
        data[bitmap_start + block // 8] |= 1 << (block % 8)
    superblock = encode_superblock(block_size, total_blocks, layout)
    for slot in range(SB_SLOTS):
        start = slot * block_size
        data[start : start + block_size] = superblock
    with open(store.path, "wb") as image:
        image.write(data)
    print("Object store formatted as v3.")


def hexdump(data):
    if not data:
        print("(empty)")
        return
    for offset in range(0, len(data), 16):
        chunk = data[offset : offset + 16]
        hexadecimal = " ".join(f"{byte:02x}" for byte in chunk)
        printable = "".join(chr(byte) if 32 <= byte < 127 else "." for byte in chunk)
        print(f"{offset:08x}  {hexadecimal:<48}  |{printable}|")
    print(f"{len(data)} bytes")


def print_info(store):
    if not store.is_formatted:
        print("  Image is not a valid v3 object store.")
        print(f"  image_size: {len(store.data)} bytes ({len(store.data) / 1024 / 1024:.1f} MiB)")
        return
    sb, layout = store.superblock, store.layout
    print(f"  format_version:   {SB_VERSION}")
    print(f"  superblock_slot:  {sb.slot}")
    print(f"  generation:       {sb.generation}")
    print(f"  block_size:       {store.block_size}")
    print(f"  total_blocks:     {store.total_blocks}")
    print(f"  next_id:          {store.next_id}")
    print(f"  bitmap_lba:       {layout.bitmap_lba} ({layout.bitmap_blocks} blocks)")
    print(f"  directory_lba:    {layout.directory_lba}")
    print(f"  directory_banks:  2 × {layout.directory_blocks} blocks")
    print(f"  directory_slots:  {layout.entry_count(store.block_size)}")
    print(f"  data_lba:         {layout.data_lba}")
    print(f"  object_count:     {len(store.objects)}")
    print(f"  image_size:       {len(store.data)} bytes ({len(store.data) / 1024 / 1024:.1f} MiB)")
    for error in store.errors:
        print(f"  warning: {error}", file=sys.stderr)


def print_object_metadata(obj_id, record, block_size, filesystem_metadata=None):
    entry = record.entry
    print(f"object {obj_id}:")
    if filesystem_metadata is not None:
        path, flags, directory_size = filesystem_metadata
        kind = "directory" if flags & Filesystem.FLAG_DIR else "file"
        print(f"  path:             {path}")
        print(f"  filesystem_type:  {kind}")
        print(
            f"  filesystem_flags: {flags:#x} "
            f"({describe_flags(flags, [(Filesystem.FLAG_DIR, 'DIRECTORY')], 'FILE')})"
        )
        print(f"  directory_size:   {directory_size}")
    print(
        f"  flags:            {entry.flags:#x} "
        f"({describe_flags(entry.flags, [(FLAG_ALLOCATED, 'ALLOCATED')])})"
    )
    print(f"  generation:       {entry.generation}")
    print(f"  header_lba:       {entry.header_lba}")
    print(f"  header_blocks:    {entry.header_blocks}")
    print(f"  data_length:      {record.data_len}")
    print(f"  allocated_length: {record.allocated_len}")
    print(f"  hash_algorithm:   FNV-1a-64")
    print(f"  data_hash:        {record.data_hash:#018x}")
    print(f"  extent_count:     {len(record.extents)}")
    for index, (lba, blocks) in enumerate(record.extents):
        print(
            f"  extent[{index}]:       lba={lba} blocks={blocks} "
            f"bytes={blocks * block_size}"
        )


def resolve_metadata_target(store, filesystem, target):
    if target == "/":
        return ROOT_ID, (target, Filesystem.FLAG_DIR, 0)
    try:
        obj_id = int(target, 0)
    except ValueError:
        result = filesystem.find(target)
        if result is None:
            raise ValueError(f"{target!r} not found")
        obj_id, flags, size = result
        return obj_id, (target, flags, size)
    return obj_id, None


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 1
    path = sys.argv[1]
    if not os.path.exists(path):
        print(f"File not found: {path}", file=sys.stderr)
        return 1
    command = sys.argv[2] if len(sys.argv) > 2 else "tree"
    store = ObjectStore(path)
    try:
        if command == "format":
            cmd_format(store)
            print_info(ObjectStore(path))
            return 0
        if not store.is_formatted:
            print("Image is not a valid v3 object store. Run 'format' to replace it.", file=sys.stderr)
            return 1
        filesystem = Filesystem(store)
        if command == "info":
            print_info(store)
        elif command == "objects":
            for obj_id, record in sorted(store.objects.items()):
                extents = ",".join(f"{lba}+{blocks}" for lba, blocks in record.extents) or "-"
                print(
                    f"{obj_id:>8}  gen={record.entry.generation:<5} "
                    f"size={record.data_len:<10} header={record.entry.header_lba} extents={extents}"
                )
        elif command == "metadata":
            if len(sys.argv) < 4:
                for index, (obj_id, record) in enumerate(sorted(store.objects.items())):
                    if index:
                        print()
                    print_object_metadata(obj_id, record, store.block_size)
            else:
                obj_id, filesystem_metadata = resolve_metadata_target(
                    store, filesystem, sys.argv[3]
                )
                record = store.objects.get(obj_id)
                if record is None:
                    raise ValueError(f"object {obj_id} not found")
                print_object_metadata(
                    obj_id, record, store.block_size, filesystem_metadata
                )
        elif command == "tree":
            filesystem.tree()
        elif command in {"dump", "raw"}:
            if len(sys.argv) < 4:
                raise ValueError(f"{command} requires an object ID")
            data = store.read_object(int(sys.argv[3]))
            if data is None:
                raise ValueError(f"object {sys.argv[3]} not found")
            if command == "dump":
                hexdump(data)
            else:
                sys.stdout.buffer.write(data)
        elif command == "cat":
            if len(sys.argv) < 4:
                raise ValueError("cat requires a filesystem path")
            result = filesystem.find(sys.argv[3])
            if result is None:
                raise ValueError(f"{sys.argv[3]!r} not found")
            obj_id, flags, _ = result
            if flags & Filesystem.FLAG_DIR:
                raise ValueError(f"{sys.argv[3]!r} is a directory")
            sys.stdout.buffer.write(store.read_object(obj_id))
        else:
            raise ValueError(f"unknown command: {command}")
    except (OSError, ValueError, struct.error) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
