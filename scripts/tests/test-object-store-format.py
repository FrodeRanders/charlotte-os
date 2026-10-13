#!/usr/bin/env python3
"""Cross-check the image producer and read-only inspector on the current grammar."""

import importlib.util
from pathlib import Path
import struct
import sys
import tempfile
import unittest
import zlib

ROOT = Path(__file__).resolve().parents[2]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


writer = load("object_image_writer", "scripts/make-nvme-image.py")
inspector = load("object_image_inspector", "scripts/fs-inspect.py")


class StoreFormat(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="charlotte-store-format-")
        self.path = Path(self.temp.name) / "store.img"
        image = writer.ImageWriter(self.path, total_blocks=128)
        self.object_id = writer.artifact_object_id("fixture")
        image.put_object(self.object_id, b"signed-fixture-placeholder")
        image.write(1)
        self.valid = self.path.read_bytes()

    def tearDown(self):
        self.temp.cleanup()

    def inspect(self, data):
        self.path.write_bytes(data)
        parsed = inspector.ObjectStore(self.path)
        self.assertEqual(self.path.read_bytes(), bytes(data), "inspection changed the image")
        return parsed

    def test_current_image_has_explicit_owner_and_exact_data(self):
        parsed = self.inspect(self.valid)
        self.assertTrue(parsed.is_formatted)
        self.assertEqual(parsed.errors, [])
        self.assertEqual(self.valid[:8], b"COBJSTR4")
        record = parsed.objects[self.object_id]
        header = self.valid[record.entry.header_lba * 512:][:512]
        self.assertEqual(header[:8], b"CCOBJHD4")
        self.assertEqual(struct.unpack_from("<Q", header, 64)[0], 1)
        self.assertEqual(parsed.read_object(self.object_id), b"signed-fixture-placeholder")

    def test_checksummed_retired_slot_rejects_even_with_a_current_peer(self):
        for slot in (0, 1):
            for tag in (b"COJBSTR3", b"COBJSTR3"):
                data = bytearray(self.valid)
                start = slot * 512
                data[start:start + 8] = tag
                struct.pack_into("<I", data, start + 8, 3)
                struct.pack_into("<I", data, start + 76, zlib.crc32(data[start:start + 76]))
                parsed = self.inspect(data)
                self.assertFalse(parsed.is_formatted)
                self.assertIn("unsupported object-store format", parsed.errors[0])

    def test_corrupt_copy_recovers_but_two_corrupt_copies_reject(self):
        for count in (1, 2):
            data = bytearray(self.valid)
            for slot in range(count):
                data[slot * 512 + 76] ^= 1
            parsed = self.inspect(data)
            self.assertEqual(parsed.is_formatted, count == 1)
            self.assertEqual(bool(parsed.errors), count == 2)
        self.assertFalse(self.inspect(bytes(len(self.valid))).is_formatted)

    def test_checksummed_invalid_headers_never_publish_objects(self):
        parsed = inspector.ObjectStore(self.path)
        start = parsed.objects[self.object_id].entry.header_lba * 512
        for offset, replacement in [(8, struct.pack("<H", 3)), (10, struct.pack("<H", 400)),
                                    (64, bytes(8)), (88, struct.pack("<I", 2)),
                                    (72, b"\x01"), (140, b"\x01"), (144, b"\x01"),
                                    (384, b"\x01")]:
            with self.subTest(offset=offset):
                data = bytearray(self.valid)
                data[start + offset:start + offset + len(replacement)] = replacement
                data[start + 112:start + 116] = bytes(4)
                struct.pack_into("<I", data, start + 112, zlib.crc32(data[start:start + 384]))
                rejected = self.inspect(data)
                self.assertTrue(rejected.errors)
                self.assertNotIn(self.object_id, rejected.objects)

    def test_two_retired_directory_copies_do_not_become_empty(self):
        parsed = inspector.ObjectStore(self.path)
        data = bytearray(self.valid)
        for bank in range(2):
            start = (parsed.layout.directory_lba + bank * parsed.layout.directory_blocks) * 512
            end = start + parsed.layout.directory_blocks * 512
            for offset in range(start, end, writer.DIR_ENTRY_SIZE):
                if any(data[offset:offset + writer.DIR_ENTRY_SIZE]):
                    struct.pack_into("<I", data, offset + 28, zlib.crc32(data[offset:offset + 28]) ^ 0x3344_4952)
        rejected = self.inspect(data)
        self.assertTrue(rejected.errors)
        self.assertNotIn(self.object_id, rejected.objects)


if __name__ == "__main__":
    unittest.main()
