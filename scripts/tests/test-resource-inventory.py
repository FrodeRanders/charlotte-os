#!/usr/bin/env python3
"""Check MIR intake distinctions without compiling or declaring security coverage."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "audit-resource-inventory.py"
SPEC = importlib.util.spec_from_file_location("resource_inventory", SCRIPT)
inventory = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(inventory)


def parse(body):
    return inventory.extract((
        "fn example(_1: &mut alloc::vec::Vec<u8>) -> () {\n"
        "    let mut _0: (); // return place in scope 0 at crates/catten/src/example.rs:1:1: 1:2\n"
        "    let mut _2: alloc::vec::Vec<u8>;\n"
        + body + "\n}\n"
    ).splitlines())


class ResourceInventory(unittest.TestCase):
    def test_generic_function_types_and_const_comparisons_do_not_end_callee(self):
        callee = "Factory::<fn(u8) -> Vec<u8>, { N < 3 }>::make"
        self.assertEqual(inventory.before_arguments(callee + "(move _1)"), callee)

    def test_empty_constructor_and_fallible_growth_remain_distinct(self):
        rows, counts = parse("""
        _2 = alloc::vec::Vec::<u8>::new() -> [return: bb1, unwind unreachable]; // scope 0 at crates/catten/src/example.rs:2:3: 2:9
        _0 = alloc::vec::Vec::<u8>::try_reserve_exact(move _1, const 9_usize) -> [return: bb2, unwind unreachable]; // scope 0 at crates/catten/src/example.rs:3:3: 3:9
        """)
        self.assertEqual([r["kind"] for r in rows], ["empty_collection", "collection_operation"])
        self.assertEqual(counts, {"functions": 1, "calls": 2})

    def test_reference_clone_is_not_reported_as_new_backing(self):
        self.assertEqual(inventory.call_kind("<alloc::sync::Arc<T> as Clone>::clone", "Arc<T>", ""), "reference_clone")

    def test_explicit_and_implicit_drop_keep_type_and_source(self):
        rows, _ = parse("""
        _0 = core::mem::drop::<alloc::vec::Vec<u8>>(move _2) -> [return: bb1, unwind unreachable]; // scope 0 at crates/catten/src/example.rs:4:3: 4:9
        drop(_2) -> [return: bb2, unwind unreachable]; // scope 0 at crates/catten/src/example.rs:5:3: 5:9
        """)
        self.assertEqual([r["kind"] for r in rows], ["explicit_drop", "implicit_drop"])
        self.assertEqual(rows[1]["value_type"], "alloc::vec::Vec<u8>")
        self.assertEqual(rows[1]["source"], ("crates/catten/src/example.rs", 5, 3))

    def test_macro_origin_keeps_containing_kernel_function(self):
        rows, _ = parse("""
        _2 = alloc::vec::Vec::<u8>::with_capacity(const 9_usize) -> [return: bb1, unwind unreachable]; // scope 0 at /Users/example/.rustup/toolchains/nightly/lib/rustlib/src/rust/library/alloc/src/macros.rs:10:3: 10:9
        """)
        self.assertEqual(rows[0]["source"][0], "<rust-src>/library/alloc/src/macros.rs")
        self.assertEqual(rows[0]["body_origin"], ("crates/catten/src/example.rs", 1, 1))

    def test_missing_span_remains_a_review_obligation(self):
        rows, _ = parse("_2 = alloc::vec::Vec::<u8>::with_capacity(const 9_usize) -> [return: bb1, unwind unreachable];")
        self.assertIsNone(rows[0]["source"])
        self.assertIsNotNone(rows[0]["body_origin"])

    def test_static_initializer_does_not_inherit_preceding_function(self):
        rows, _ = inventory.extract("""fn before() -> () {
        let mut _0: (); // return place in scope 0 at crates/catten/src/example.rs:1:1: 1:2
        _0 = alloc::vec::Vec::<u8>::new() -> [return: bb1, unwind unreachable]; // scope 0 at crates/catten/src/example.rs:2:1: 2:2
}
static CACHE: alloc::vec::Vec<u8> = {
        let mut _0: alloc::vec::Vec<u8>; // return place in scope 0 at crates/catten/src/example.rs:4:1: 4:2
        _0 = alloc::vec::Vec::<u8>::new() -> [return: bb1, unwind unreachable]; // scope 0 at crates/catten/src/example.rs:5:1: 5:2
}
        """.splitlines())
        self.assertEqual(rows[0]["body_kind"], "function")
        self.assertEqual(rows[1]["body_kind"], "static")
        self.assertTrue(rows[1]["body"].startswith("static CACHE:"))
        self.assertEqual(rows[1]["body_origin"], ("crates/catten/src/example.rs", 4, 1))

    def test_owned_result_does_not_claim_the_producer_allocated(self):
        self.assertEqual(inventory.call_kind("Profile::encode", "Result<alloc::vec::Vec<u8>, E>", ""), "owned_result")
        self.assertEqual(inventory.call_kind("alloc::string::String::from_utf8", "Result<alloc::string::String, E>", ""), "owned_result")

    def test_allocator_function_item_is_retained(self):
        expression = "Factory::prepare(const alloc::boxed::Box::<T>::try_new)"
        self.assertEqual(inventory.call_kind("Factory::prepare", "Node<T>", expression), "allocator_factory_reference")

    def test_comments_and_incompatible_input_cannot_be_accepted(self):
        with self.assertRaises(ValueError):
            inventory.extract(["// _2 = alloc::vec::Vec::<u8>::new() -> [return: bb1];"])
        with self.assertRaises(ValueError):
            inventory.before_arguments("Vec::<broken(move _1)")

    def test_cli_coalesces_lowered_duplicates_without_repeating_profiles(self):
        mir = """fn example() -> () {
        let mut _0: (); // return place in scope 0 at crates/catten/src/main.rs:1:1: 1:2
        _0 = alloc::vec::Vec::<u8>::new() -> [return: bb1, unwind unreachable]; // scope 0 at crates/catten/src/main.rs:2:1: 2:2
        _0 = alloc::vec::Vec::<u8>::new() -> [return: bb2, unwind unreachable]; // scope 0 at crates/catten/src/main.rs:2:1: 2:2
        }
        """
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            source, output = directory / "input.mir", directory / "output.json"
            source.write_text(mir)
            subprocess.run([sys.executable, str(SCRIPT), "--mir", f"a={source}", "--mir", f"b={source}", "--date", "2026-11-04", "--output", str(output)], check=True, capture_output=True, text=True)
            report = json.loads(output.read_text())
        self.assertEqual(report["record_count"], 1)
        self.assertEqual(report["profiles"]["a"]["records"], 2)
        self.assertEqual(report["profiles"]["a"]["unique_records"], 1)
        self.assertEqual(report["records"][0]["profiles"], ["a", "b"])


if __name__ == "__main__":
    unittest.main()
