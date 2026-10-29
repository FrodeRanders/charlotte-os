#!/usr/bin/env python3
"""Exercise the production RNG ring helpers with a deterministic device step.

The model advances 0x00ff to 0x0100 between byte accesses, without concurrent
Rust memory access. Isolated byte-copy mutations must fail; neither is deployed.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[2]
RESULTS = []


def function(source, name):
    start = source.index("fn " + name + "(")
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


MODEL = r'''
use std::{cell::Cell, marker::PhantomData};
struct SharedDmaMemory<'a> {
    word: Cell<u16>, transition: Cell<bool>, published: Cell<u16>,
    lifetime: PhantomData<&'a ()>,
}
#[allow(dead_code)]
impl SharedDmaMemory<'_> {
    fn new() -> Self { Self { word: Cell::new(0xff), transition: Cell::new(true),
        published: Cell::new(0xff), lifetime: PhantomData } }
    fn step(&self) { if self.transition.replace(false) { self.word.set(0x100); } }
    fn read_volatile_into(&self, _: usize, bytes: &mut [u8]) -> Result<(), ()> {
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = self.word.get().to_le_bytes()[index];
            self.step();
        }
        Ok(())
    }
    fn read_volatile_u16_le(&self, _: usize) -> Option<u16> {
        let word = self.word.get(); self.step(); Some(word)
    }
    fn write_volatile_from(&mut self, _: usize, bytes: &[u8]) -> Result<(), ()> {
        for (index, byte) in bytes.iter().copied().enumerate() {
            let mut word = self.word.get().to_le_bytes(); word[index] = byte;
            self.word.set(u16::from_le_bytes(word));
            if index == 0 { self.published.set(self.word.get()); }
        }
        Ok(())
    }
    fn write_volatile_u16_le(&mut self, _: usize, word: u16) -> Result<(), ()> {
        self.word.set(word); self.published.set(word); Ok(())
    }
}
#[test]
fn completion_index_is_one_observation() {
    let ring = SharedDmaMemory::new();
    let observed = ring_read_u16(&ring, 0).unwrap();
    assert!(matches!(observed, 0xff | 0x100), "torn used index: {observed:#x}");
    assert_eq!(ring_read_u16(&ring, 0), Some(0x100));
}
#[test]
fn publication_has_no_intermediate_index() {
    let mut ring = SharedDmaMemory::new();
    assert!(ring_write_u16(&mut ring, 0, 0x100));
    assert_eq!(ring.word.get(), 0x100);
    assert_eq!(ring.published.get(), 0x100, "torn available index");
}
'''


class RngRingAccess(unittest.TestCase):
    def control(self, mutation=None, test_filter=None):
        source = (ROOT / "crates/catten-services/src/bin/rng.rs").read_text()
        helpers = "\n".join(function(source, name) for name in
                            ("ring_write", "ring_write_u16", "ring_read_u16"))
        if mutation:
            old, new = mutation
            self.assertEqual(helpers.count(old), 1)
            helpers = helpers.replace(old, new)
        toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
        harness = "#![allow(dead_code)]\n" + MODEL + helpers
        with tempfile.TemporaryDirectory(prefix="charlotte-rng-ring-") as folder:
            path = Path(folder)
            (path / "probe.rs").write_text(harness)
            built = subprocess.run(["rustc", "+" + toolchain, "--edition=2024", "--test", "-O",
                                    str(path / "probe.rs"), "-o", str(path / "probe")],
                                   capture_output=True, text=True)
            self.assertEqual(built.returncode, 0, built.stderr)
            run = subprocess.run([str(path / "probe")] + ([test_filter] if test_filter else []),
                                 capture_output=True, text=True)
            RESULTS.append({"mutation": mutation, "harness": harness,
                            "harness_sha256": hashlib.sha256(harness.encode()).hexdigest(),
                            "executable_sha256": hashlib.sha256((path / "probe").read_bytes()).hexdigest(),
                            "exit_code": run.returncode,
                            "diagnostics": (run.stdout + run.stderr).replace(folder, "<control>").splitlines()})
            return run.returncode, run.stdout + run.stderr

    def test_current_helpers(self):
        status, text = self.control()
        self.assertEqual(status, 0, text)
        self.assertIn("2 passed; 0 failed", text)

    def test_byte_completion_read_is_rejected(self):
        status, text = self.control(("ring.read_volatile_u16_le(offset)",
            "let mut bytes = [0; 2]; ring.read_volatile_into(offset, &mut bytes).ok()?; Some(u16::from_le_bytes(bytes))"),
            "completion_index_is_one_observation")
        self.assertEqual(status, 101, text)
        self.assertIn("torn used index: 0x1ff", text)

    def test_byte_publication_is_rejected(self):
        status, text = self.control(("ring.write_volatile_u16_le(offset, value).is_ok()",
                                    "ring_write(ring, offset, &value.to_le_bytes())"),
                                   "publication_has_no_intermediate_index")
        self.assertEqual(status, 101, text)
        self.assertIn("torn available index", text)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json-output", type=Path)
    args, remaining = parser.parse_known_args()
    program = unittest.main(argv=[__file__] + remaining, exit=False)
    if args.json_output:
        args.json_output.write_text(json.dumps(RESULTS, indent=2) + "\n")
    raise SystemExit(not program.result.wasSuccessful())
