#!/usr/bin/env python3
"""Extract review candidates from pinned-rustc MIR, without assigning security dispositions.

MIR is an unstable human format. This is a checkpoint aid, not an allocation,
reachability or lock-safety proof. See docs/reference/security-closure-plan.md.
"""
import argparse
import collections
import hashlib
import json
from pathlib import Path
import re
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
SPAN = re.compile(r"// .*?\bscope \d+ at (.+\.rs):(\d+):(\d+):")
LOCAL = re.compile(r"\s*let (?:mut )?(_\d+): (.+?);\s*(?://|$)")
OWNED = re.compile(r"alloc::(?:vec::Vec|boxed::Box|sync::Arc|rc::Rc|string::String|collections::)")
COLLECTION = re.compile(r"<?(?:alloc::(?:vec::Vec|boxed::Box|sync::Arc|rc::Rc|string::String|collections::)|hashbrown::)")
GROW = re.compile(r"::(?:try_)?(?:new(?:_uninit|_zeroed|_cyclic)?(?:_in)?|with_capacity(?:_in)?|reserve(?:_exact)?|resize(?:_with)?|push(?:_str|_back|_front)?|insert|append|extend(?:_from_slice|_from_within)?|shrink_to(?:_fit)?|make_mut|into_boxed_slice|clone|from|from_iter|or_insert(?:_with)?)(?:::<.*>)?$")


def normalized(text):
    return re.sub(r"(?:/[^ <>\n]*)?/lib/rustlib/src/rust/library/", "<rust-src>/library/", text)


def before_arguments(expression):
    """Find actual arguments, skipping generic fn types, closures and consts."""
    angles = braces = 0
    for index, char in enumerate(expression):
        if char == "{":
            braces += 1
        elif char == "}":
            braces -= 1
        elif not braces:
            if char == "<":
                angles += 1
            elif char == ">" and (index == 0 or expression[index - 1] != "-"):
                angles -= 1
            elif char == "(" and angles == 0:
                return expression[:index]
    raise ValueError(f"Unrecognized MIR call: {expression[:200]}")


def call_kind(callee, result_type, expression):
    if re.search(r"(?:^|::)mem::drop(?:::<.*>)?$", callee):
        return "explicit_drop"
    if re.match(r"<?alloc::(?:sync::Arc|rc::Rc)", callee) and callee.endswith("::clone"):
        return "reference_clone"
    if re.match(r"alloc::(?:vec::Vec|collections::|string::String)|hashbrown::", callee) and re.search(r"::new(?:_in)?$", callee):
        return "empty_collection"
    if "alloc::alloc::" in callee and re.search(r"::(?:alloc|alloc_zeroed|realloc|allocate(?:_zeroed)?|grow(?:_zeroed)?|shrink)$", callee):
        return "allocator_backend"
    if callee == "alloc::fmt::format" or re.search(r"::(?:collect|to_vec|to_owned|to_string)(?:::<.*>)?$", callee):
        return "collection_or_copy"
    if COLLECTION.match(callee) and GROW.search(callee):
        return "collection_operation"
    if "ConcurrentQueue" in callee and re.search(r"::(?:un)?bounded$", callee):
        return "dependency_factory"
    if re.search(r"(?:Box|Arc|Rc)(?:::<[^{}]*>)?::(?:try_)?new", expression):
        return "allocator_factory_reference"
    if OWNED.search(result_type) and not callee.startswith(("core::mem::", "Option::<", "<Option<")):
        return "owned_result"
    return None


def extract(lines):
    function = origin = body_kind = None
    locals_ = {}
    calls = functions = 0
    records = []
    for line in lines:
        if line.startswith("fn "):
            function = before_arguments(line[3:].rstrip()).strip()
            body_kind = "function"
            origin = None
            locals_ = dict(re.findall(r"(_\d+): (.*?)(?=, _\d+:|\) ->)", line))
            functions += 1
        elif line.startswith(("const ", "static ")):
            body_kind = "constant" if line.startswith("const ") else "static"
            function = line.split(" = ", 1)[0].strip()
            origin = None
            locals_ = {}
        elif line.startswith("}"):
            function = origin = body_kind = None
        if function is None or line.lstrip().startswith("//"):
            continue
        span = SPAN.search(line)
        local = LOCAL.match(line)
        if local:
            locals_[local[1]] = local[2]
            if local[1] == "_0" and span:
                origin = (normalized(span[1]), int(span[2]), int(span[3]))
        if " -> [" not in line:
            continue
        implicit = re.match(r"\s*drop\((.+)\) -> \[", line)
        if implicit:
            callee = "drop"
            result_type = locals_.get(implicit[1], implicit[1])
            kind = "implicit_drop"
        elif " = " in line:
            calls += 1
            lhs, expression = line.split(" = ", 1)
            expression = expression.split(" -> [", 1)[0]
            callee = before_arguments(expression)
            result_type = locals_.get(lhs.strip(), "<projected or unknown result>")
            kind = call_kind(callee, result_type, expression)
            if kind is None:
                continue
        else:
            continue
        source = (normalized(span[1]), int(span[2]), int(span[3])) if span else None
        records.append({
            "kind": kind, "body": normalized(function), "body_kind": body_kind, "source": source,
            "body_origin": origin, "callee": normalized(callee),
            "value_type": normalized(result_type),
        })
    if not functions or not calls:
        raise ValueError("No MIR function/call bodies found; do not accept this input")
    return records, {"functions": functions, "calls": calls}


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mir", action="append", required=True, metavar="PROFILE=PATH")
    parser.add_argument("--date", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    merged, profiles = {}, {}
    for item in args.mir:
        profile, path = item.split("=", 1)
        if profile in profiles:
            parser.error(f"duplicate profile: {profile}")
        path = Path(path)
        with path.open() as stream:
            records, counts = extract(stream)
        identities = set()
        profiles[profile] = dict(counts, sha256=sha(path), records=len(records))
        for record in records:
            encoded = json.dumps(record, sort_keys=True)
            identity = hashlib.sha256(encoded.encode()).hexdigest()[:16]
            identities.add(identity)
            if identity not in merged:
                merged[identity] = dict(record, id=identity, profiles=[])
            if profile not in merged[identity]["profiles"]:
                merged[identity]["profiles"].append(profile)
        profiles[profile]["unique_records"] = len(identities)
    records = sorted(merged.values(), key=lambda r: (tuple(r["source"] or ("", 0, 0)), r["id"]))
    paths = sorted((ROOT / "crates/catten/src").rglob("*.rs"))
    source_hashes = {str(p.relative_to(ROOT)): sha(p) for p in paths}
    inputs = ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml",
              "crates/catten/Cargo.toml", "target_specs/aarch64-unknown-none-catten.json",
              "target_specs/x86_64-unknown-none-catten.json"]
    inputs += [f"crates/charlotte-launch/src/{name}.rs" for name in
               ["lib", "deployment", "operations", "operations_pickup", "operations_bundle", "release", "shutdown"]]
    result = {
        "schema": "charlotte.resource-review-inventory.v1", "date": args.date,
        "base_revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
        "pinned_toolchain": tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"],
        "extractor_sha256": sha(Path(__file__)), "kernel_source_sha256": source_hashes,
        "capture_inputs_sha256": {p: sha(ROOT / p) for p in inputs},
        "scope": "Primary kernel-crate MIR for the declared profiles; compiled bodies include unused/generic code. Records are review candidates, not findings, allocations performed, reachability or lock-safety proofs.",
        "limitations": [
            "Dependencies, optional profiles and compiler/allocator internals require explicit review; their bodies are not supplied by a primary-crate MIR dump.",
            "Collection operations can reuse prepared capacity; owned results can move existing storage; reference clones and empty constructors normally allocate no backing.",
            "Drop records include harmless guard releases and generic operands. An explicit drop call value_type describes its result; the consumed type remains in callee.",
            "The unstable MIR format and selected API patterns are not an exhaustive allocator/call-graph analysis. Missing source spans and opaque factories remain review obligations.",
        ],
        "profiles": profiles, "record_count": len(records),
        "counts_by_kind": dict(sorted(collections.Counter(r["kind"] for r in records).items())),
        "missing_source_spans": sum(r["source"] is None for r in records),
        "records": records,
    }
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: result[key] for key in ["record_count", "counts_by_kind", "missing_source_spans"]}, indent=2))


if __name__ == "__main__":
    main()
