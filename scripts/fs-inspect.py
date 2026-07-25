#!/usr/bin/env python3
"""CharlotteOS filesystem inspector.

Reads a persistent object store disk image (nvme-disk.img), parses the
on-disk layout, and provides a tree view of the filesystem rooted at
object ID 100.  Can also dump individual file or directory contents.

Usage:
    python3 scripts/fs-inspect.py <image>                    # tree
    python3 scripts/fs-inspect.py <image> format             # format image
    python3 scripts/fs-inspect.py <image> tree               # tree
    python3 scripts/fs-inspect.py <image> dump <object-id>   # hexdump
    python3 scripts/fs-inspect.py <image> cat <path>         # cat file
    python3 scripts/fs-inspect.py <image> raw <object-id>    # raw bytes
    python3 scripts/fs-inspect.py <image> objects            # list objects
    python3 scripts/fs-inspect.py <image> info               # store info
"""

import struct
import sys
import os


SB_MAGIC = 0x525453424A4F  # "OBJSTR" LE
SB_VERSION = 1
ROOT_ID = 100
BLOCK_SIZE = 512
DIR_ENTRIES = 512
DIR_BLOCKS = 32
METADATA_BLOCKS = 34  # 1 + 1 + 32


def read_u64(data, offset):
    return struct.unpack_from("<Q", data, offset)[0]


def read_u32(data, offset):
    return struct.unpack_from("<I", data, offset)[0]


class ObjectStore:
    """Parse the on-disk object store format."""

    def __init__(self, path):
        self.path = path
        with open(path, "rb") as f:
            self.data = bytearray(f.read())
        try:
            self._parse_superblock()
            self._parse_directory()
        except ValueError:
            self.objects = {}
            self.block_size = 512
            self.total_blocks = 0
            self.next_id = 0
            self.dir_start = 0

    @property
    def is_formatted(self):
        return len(self.objects) > 0 or self.total_blocks > 0

    def _parse_superblock(self):
        magic = read_u64(self.data, 0)
        version = read_u32(self.data, 8)
        if magic != SB_MAGIC or version != SB_VERSION:
            raise ValueError(
                f"Not a CharlotteOS object store (magic={magic:#x}, version={version})"
            )
        self.block_size = read_u32(self.data, 16)
        self.total_blocks = read_u32(self.data, 20)
        self.next_id = read_u32(self.data, 28)
        self.dir_start = read_u64(self.data, 32)

    def _parse_directory(self):
        self.objects = {}
        dir_base = self.dir_start * self.block_size
        for i in range(DIR_ENTRIES):
            offset = dir_base + i * 32
            obj_id = read_u64(self.data, offset)
            flags = read_u32(self.data, offset + 8)
            size = read_u32(self.data, offset + 12)
            first_lba = read_u64(self.data, offset + 16)
            if obj_id != 0:
                self.objects[obj_id] = (flags, size, first_lba)

    def read_object(self, obj_id):
        if obj_id not in self.objects:
            return None
        _, size, first_lba = self.objects[obj_id]
        if first_lba == 0:
            return b""
        offset = first_lba * self.block_size
        if offset + self.block_size > len(self.data):
            return None
        return bytes(self.data[offset : offset + self.block_size])


class Filesystem:
    """Walk the filesystem tree starting at ROOT_ID."""

    FLAG_DIR = 1 << 0

    def __init__(self, store):
        self.store = store

    def _decode_dir(self, data):
        entries = []
        pos = 0
        while pos + 4 <= len(data):
            name_len = read_u32(data, pos)
            if name_len == 0:
                break
            if pos + 4 + name_len + 20 > len(data):
                break
            name = data[pos + 4 : pos + 4 + name_len].decode("utf-8", errors="replace")
            file_id = read_u64(data, pos + 4 + name_len)
            flags = read_u32(data, pos + 4 + name_len + 8)
            size = read_u64(data, pos + 4 + name_len + 12)
            entries.append((name, file_id, flags, size))
            pos += 4 + name_len + 20
        return entries

    def list_dir(self, obj_id):
        data = self.store.read_object(obj_id)
        if data is None:
            return []
        return self._decode_dir(data)

    def read_file(self, obj_id):
        return self.store.read_object(obj_id)

    def tree(self, obj_id=ROOT_ID, prefix="", name="/"):
        print(f"{prefix}{name}  [{obj_id}]")
        entries = self.list_dir(obj_id)
        for i, (entry_name, entry_id, flags, size) in enumerate(entries):
            is_last = i == len(entries) - 1
            connector = "\u2514\u2500\u2500 " if is_last else "\u251c\u2500\u2500 "
            child_prefix = prefix + ("    " if is_last else "\u2502   ")
            is_dir = bool(flags & self.FLAG_DIR)
            if is_dir:
                print(f"{prefix}{connector}{entry_name}/  [{entry_id}]")
                self._tree_recurse(entry_id, child_prefix)
            else:
                sz = self._fmt_size(size)
                print(f"{prefix}{connector}{entry_name}  [{entry_id}]  {sz}")

    def _tree_recurse(self, obj_id, prefix):
        entries = self.list_dir(obj_id)
        for i, (entry_name, entry_id, flags, size) in enumerate(entries):
            is_last = i == len(entries) - 1
            connector = "\u2514\u2500\u2500 " if is_last else "\u251c\u2500\u2500 "
            child_prefix = prefix + ("    " if is_last else "\u2502   ")
            is_dir = bool(flags & self.FLAG_DIR)
            if is_dir:
                print(f"{prefix}{connector}{entry_name}/  [{entry_id}]")
                self._tree_recurse(entry_id, child_prefix)
            else:
                sz = self._fmt_size(size)
                print(f"{prefix}{connector}{entry_name}  [{entry_id}]  {sz}")

    @staticmethod
    def _fmt_size(size):
        if size == 0:
            return "empty"
        if size < 1024:
            return f"{size}B"
        if size < 1024 * 1024:
            return f"{size / 1024:.1f}K"
        return f"{size / (1024 * 1024):.1f}M"

    def find(self, path):
        parts = [p for p in path.strip("/").split("/") if p]
        current = ROOT_ID
        result = None
        for part in parts:
            found = None
            for name, obj_id, flags, size in self.list_dir(current):
                if name == part:
                    result = (obj_id, flags, size)
                    found = obj_id
                    break
            if found is None:
                return None
            current = found
        return result


def cmd_tree(fs):
    fs.tree()


def cmd_dump(fs, obj_id_str):
    obj_id = int(obj_id_str)
    data = fs.read_file(obj_id)
    if data is None:
        print(f"Object {obj_id} not found.", file=sys.stderr)
        sys.exit(1)
    data = data.rstrip(b"\x00")
    if not data:
        print("(empty)")
        return
    for i in range(0, len(data), 16):
        chunk = data[i : i + 16]
        hex_part = " ".join(f"{b:02x}" for b in chunk)
        ascii_part = "".join(chr(b) if 32 <= b < 127 else "." for b in chunk)
        print(f"{i:08x}  {hex_part:<48}  |{ascii_part}|")
    print(f"{len(data)} bytes")


def cmd_cat(fs, path):
    result = fs.find(path)
    if result is None:
        print(f"'{path}' not found.", file=sys.stderr)
        sys.exit(1)
    obj_id, flags, size = result
    if flags & Filesystem.FLAG_DIR:
        print(f"'{path}' is a directory.", file=sys.stderr)
        sys.exit(1)
    data = fs.read_file(obj_id)
    if data is None:
        print(f"Object {obj_id} not found.", file=sys.stderr)
        sys.exit(1)
    data = data.rstrip(b"\x00")
    sys.stdout.buffer.write(data)


def cmd_raw(store, obj_id_str):
    obj_id = int(obj_id_str)
    data = store.read_object(obj_id)
    if data is None:
        print(f"Object {obj_id} not found.", file=sys.stderr)
        sys.exit(1)
    sys.stdout.buffer.write(data)


def cmd_objects(store):
    for obj_id in sorted(store.objects.keys()):
        flags, size, first_lba = store.objects[obj_id]
        print(f"  {obj_id:>6}  flags={flags:#x}  size={size}  lba={first_lba}")


def cmd_info(store):
    if not store.is_formatted:
        print("  Image is unformatted (all zeros).")
        print(f"  image_size: {len(store.data)} bytes "
              f"({len(store.data) / 1024 / 1024:.1f} MiB)")
        print()
        print("  Run 'format' to initialise the object store.")
        return
    print(f"  block_size:    {store.block_size}")
    print(f"  total_blocks:  {store.total_blocks}")
    print(f"  next_id:       {store.next_id}")
    print(f"  dir_start_lba: {store.dir_start}")
    print(f"  object_count:  {len(store.objects)}")
    print(f"  image_size:    {len(store.data)} bytes "
          f"({len(store.data) / 1024 / 1024:.1f} MiB)")


def cmd_format(store):
    data = store.data
    bs = 512
    tb = len(data) // bs

    struct.pack_into("<Q", data, 0, SB_MAGIC)
    struct.pack_into("<I", data, 8, SB_VERSION)
    struct.pack_into("<I", data, 12, 0)
    struct.pack_into("<I", data, 16, bs)
    struct.pack_into("<I", data, 20, tb)
    struct.pack_into("<I", data, 24, 0)
    struct.pack_into("<I", data, 28, 1)
    struct.pack_into("<Q", data, 32, 2)

    bitmap_off = bs
    for i in range(METADATA_BLOCKS):
        byte_idx = bitmap_off + (i // 8)
        bit_idx = i % 8
        data[byte_idx] |= 1 << bit_idx

    for i in range(DIR_BLOCKS):
        off = (2 + i) * bs
        data[off : off + bs] = b"\x00" * bs

    with open(store.path, "wb") as f:
        f.write(data)

    store.__init__(store.path)
    print("Object store formatted.")


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(1)

    path = sys.argv[1]
    if not os.path.exists(path):
        print(f"File not found: {path}", file=sys.stderr)
        sys.exit(1)

    store = ObjectStore(path)
    fs = Filesystem(store)

    cmd = sys.argv[2] if len(sys.argv) > 2 else "tree"

    if cmd == "tree":
        if not store.is_formatted:
            print("Image is unformatted. Run 'format' first.")
        else:
            cmd_tree(fs)
    elif cmd == "format":
        cmd_format(store)
        cmd_info(store)
    elif cmd == "dump":
        if len(sys.argv) < 4:
            print("Usage: fs-inspect.py <image> dump <object-id>", file=sys.stderr)
            sys.exit(1)
        cmd_dump(fs, sys.argv[3])
    elif cmd == "cat":
        if len(sys.argv) < 4:
            print("Usage: fs-inspect.py <image> cat <path>", file=sys.stderr)
            sys.exit(1)
        cmd_cat(fs, sys.argv[3])
    elif cmd == "raw":
        if len(sys.argv) < 4:
            print("Usage: fs-inspect.py <image> raw <object-id>", file=sys.stderr)
            sys.exit(1)
        cmd_raw(store, sys.argv[3])
    elif cmd == "objects":
        cmd_objects(store)
    elif cmd == "info":
        cmd_info(store)
    else:
        print(f"Unknown command: {cmd}", file=sys.stderr)
        print(__doc__)
        sys.exit(1)


if __name__ == "__main__":
    main()
