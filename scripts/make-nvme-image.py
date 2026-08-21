#!/usr/bin/env python3
"""Produce a CharlotteOS object-store image preseeded with signed userspace
service ELFs.

AArch64 uses a small embedded storage bootstrap and loads the remaining
services from a host-preseeded image. The x86-64 appliance can instead start
with a blank disk and idempotently install its immutable signed bundle during
first boot. This tool remains the QEMU path for constructing a version-3 store
ahead of boot (the same format `objstore` creates and that
`scripts/fs-inspect.py` parses). It stores every staged ELF under its derived
cluster-wide artifact id (`dns::artifact_object_id(name)`).

Usage:
    python3 scripts/make-nvme-image.py <image> <bundle-dir>
"""

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

ARTIFACT_ID_TAG = 0xFFFE_0000_0000_0000

BLOCK_SIZE = 512
IMAGE_BLOCKS = 16 * 1024 * 1024 // BLOCK_SIZE  # the standard 16 MiB boot disk


def u16(data, offset):
    return struct.unpack_from("<H", data, offset)[0]


def u32(data, offset):
    return struct.unpack_from("<I", data, offset)[0]


def u64(data, offset):
    return struct.unpack_from("<Q", data, offset)[0]


def put_u16(data, offset, value):
    struct.pack_into("<H", data, offset, value)


def put_u32(data, offset, value):
    struct.pack_into("<I", data, offset, value)


def put_u64(data, offset, value):
    struct.pack_into("<Q", data, offset, value)


def crc32(data):
    return zlib.crc32(data) & 0xFFFF_FFFF


def fnv1a64(data):
    value = 0xCBF2_9CE4_8422_2325
    for byte in data:
        value ^= byte
        value = (value * 0x100_0000_01B3) & 0xFFFF_FFFF_FFFF_FFFF
    return value


def artifact_object_id(name):
    return ARTIFACT_ID_TAG | (fnv1a64(name.encode()) & 0x0000_FFFF_FFFF_FFFF)


class Layout:
    def __init__(self, block_size, total_blocks):
        bitmap_blocks = ((total_blocks + 7) // 8 + block_size - 1) // block_size
        directory_blocks = min(max(total_blocks // 2048, 1), MAX_DIRECTORY_BLOCKS)
        self.bitmap_lba = SB_SLOTS
        self.bitmap_blocks = bitmap_blocks
        self.directory_lba = self.bitmap_lba + bitmap_blocks
        self.directory_blocks = directory_blocks
        self.data_lba = self.directory_lba + 2 * directory_blocks
        if self.data_lba + 2 > total_blocks:
            raise ValueError("image is too small for the v3 object-store layout")


class ImageWriter:
    def __init__(self, path, block_size=BLOCK_SIZE, total_blocks=IMAGE_BLOCKS):
        self.path = path
        self.block_size = block_size
        self.total_blocks = total_blocks
        self.layout = Layout(block_size, total_blocks)
        self.data = bytearray(block_size * total_blocks)
        self.bitmap = bytearray((total_blocks + 7) // 8)
        # Blocks below the data area are reserved metadata.
        for block in range(self.layout.data_lba):
            self._set_bit(block, True)

    def _set_bit(self, block, used):
        if used:
            self.bitmap[block // 8] |= 1 << (block % 8)
        else:
            self.bitmap[block // 8] &= ~(1 << (block % 8))

    def _alloc(self, blocks):
        start = None
        count = 0
        for block in range(self.layout.data_lba, self.total_blocks):
            if not self.bitmap[block // 8] & (1 << (block % 8)):
                if start is None:
                    start = block
                count += 1
                if count == blocks:
                    for b in range(start, start + blocks):
                        self._set_bit(b, True)
                    return start
            else:
                start = None
                count = 0
        raise ValueError("not enough free blocks for object")

    def _write_blocks(self, lba, data):
        start = lba * self.block_size
        if start + len(data) > len(self.data):
            raise ValueError("write beyond image")
        self.data[start : start + len(data)] = data

    def encode_superblock(self, generation, next_id):
        block = bytearray(self.block_size)
        put_u64(block, 0, SB_MAGIC)
        put_u32(block, 8, SB_VERSION)
        put_u32(block, 12, SB_LEN)
        put_u64(block, 16, generation)
        put_u32(block, 24, self.block_size)
        put_u32(block, 28, self.total_blocks)
        put_u64(block, 32, next_id)
        put_u64(block, 40, self.layout.bitmap_lba)
        put_u32(block, 48, self.layout.bitmap_blocks)
        put_u32(block, 52, self.layout.directory_blocks)
        put_u64(block, 56, self.layout.directory_lba)
        put_u64(block, 64, self.layout.data_lba)
        put_u32(block, 72, 0)
        put_u32(block, SB_CHECKSUM_OFFSET, crc32(bytes(block[:SB_CHECKSUM_OFFSET])))
        return block

    def encode_directory(self, obj_id, generation, header_lba):
        entry = bytearray(DIR_ENTRY_SIZE)
        put_u64(entry, 0, obj_id)
        put_u32(entry, 8, FLAG_ALLOCATED)
        put_u32(entry, 12, generation)
        put_u64(entry, 16, header_lba)
        put_u32(entry, 24, 1)
        put_u32(entry, 28, crc32(bytes(entry[:28])) ^ DIR_CRC_SALT)
        return entry

    def encode_header(self, obj_id, generation, data_len, extents, data_hash):
        header = bytearray(HEADER_LEN)
        put_u64(header, 0, HEADER_MAGIC)
        put_u16(header, 8, HEADER_VERSION)
        put_u16(header, 10, HEADER_LEN)
        put_u32(header, 12, FLAG_ALLOCATED)
        put_u64(header, 16, obj_id)
        put_u64(header, 24, generation)
        put_u64(header, 32, data_len)
        put_u64(header, 40, sum(blocks for _, blocks in extents) * self.block_size)
        put_u64(header, 48, self.block_size)
        put_u16(header, 56, len(extents))
        put_u16(header, 58, HASH_FNV1A64)
        put_u32(header, 60, 1)
        put_u64(header, 80, data_hash)
        for index, (lba, blocks) in enumerate(extents):
            offset = EXTENTS_OFFSET + index * EXTENT_SIZE
            put_u64(header, offset, lba)
            put_u32(header, offset + 8, blocks)
        put_u32(header, HEADER_CHECKSUM_OFFSET, 0)
        put_u32(header, HEADER_CHECKSUM_OFFSET, crc32(bytes(header)))
        return header

    def put_object(self, obj_id, data, generation=1):
        if not data:
            return
        data_blocks = (len(data) + self.block_size - 1) // self.block_size
        header_lba = self._alloc(1)
        extents = [(self._alloc(data_blocks), data_blocks)]
        header = self.encode_header(obj_id, generation, len(data), extents, fnv1a64(data))
        self._write_blocks(header_lba, header)
        self._write_blocks(extents[0][0], data.ljust(extents[0][1] * self.block_size, b"\0"))
        entry = self.encode_directory(obj_id, generation, header_lba)
        # Write the entry into both directory banks (identical generations,
        # so the newest-entry pick is unambiguous on mount).
        for bank in range(2):
            bank_lba = self.layout.directory_lba + bank * self.layout.directory_blocks
            byte_offset = self._directory_index(obj_id) * DIR_ENTRY_SIZE
            lba = bank_lba + byte_offset // self.block_size
            offset = byte_offset % self.block_size
            block = bytearray(self._read_blocks(lba))
            block[offset : offset + DIR_ENTRY_SIZE] = entry
            self._write_blocks(lba, block)

    def _directory_index(self, obj_id):
        # A stable slot per object id (the store's own directory is
        # index-assigned; ids here are sparse, so spread them by hash).
        entries_per_block = self.block_size // DIR_ENTRY_SIZE
        total = self.layout.directory_blocks * entries_per_block
        return (obj_id >> 8) % total

    def _read_blocks(self, lba):
        start = lba * self.block_size
        return bytes(self.data[start : start + self.block_size])

    def write(self, next_id):
        # Metadata: superblocks (both slots), the allocation bitmap, and the
        # (already written) directory banks.
        for slot in range(SB_SLOTS):
            self._write_blocks(slot, self.encode_superblock(1, next_id))
        self._write_blocks(self.layout.bitmap_lba, bytes(self.bitmap))
        with open(self.path, "wb") as image:
            image.write(self.data)


def main():
    if len(sys.argv) != 3:
        print(f"usage: {sys.argv[0]} <image> <bundle-dir>", file=sys.stderr)
        sys.exit(1)
    image_path, bundle = sys.argv[1], sys.argv[2]

    import glob
    import os

    writer = ImageWriter(image_path)
    next_id = 1
    object_names = {}
    for elf_path in sorted(glob.glob(os.path.join(bundle, "*.elf"))):
        name = os.path.basename(elf_path)[: -len(".elf")]
        with open(elf_path, "rb") as elf:
            data = elf.read()
        obj_id = artifact_object_id(name)
        previous = object_names.get(obj_id)
        if previous is not None:
            raise SystemExit(
                f"artifact id collision: {previous!r} and {name!r} both map to {obj_id:#018x}"
            )
        object_names[obj_id] = name
        writer.put_object(obj_id, data)
        next_id += 1
        print(f"staged {name}.elf ({len(data)} bytes) at object {obj_id:#018x}")
    writer.write(next_id)
    print(f"wrote {image_path} ({len(writer.data)} bytes, {next_id - 1} objects)")


if __name__ == "__main__":
    main()
