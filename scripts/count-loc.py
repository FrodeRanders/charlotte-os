#!/usr/bin/env python3
"""Refresh loc.txt from tracked working-tree files using cloc and explicit scopes.

No downloaded dependency tree is traversed. Rust test attribution is a source
inventory (paths plus selected inline items), not a compiled-image size estimate.
"""

import argparse
from collections import defaultdict
import json
from pathlib import Path
import re
import subprocess
import tempfile
import textwrap

ROOT = Path(__file__).resolve().parents[1]
DEMOS = {"echo", "greet"}
PROBES = {
    "client", "cclient", "nclient", "rclient", "tcpclient", "nvme_client",
    "objstore_client", "smoke", "security_probe", "shutdown_probe", "s3_smoke",
    "kafka_smoke", "kafka_fence_smoke", "kafka_step_input", "kafka_step_proc",
}
VALIDATION_FILES = {
    "scripts/generate-security-test-fixtures.sh",
    "scripts/prepare-local-deployment-fixture.sh",
    "scripts/prepare-management-tls-fixture.py",
    "scripts/secure-boot-test-requirements.txt",
}
COMPONENTS = [
    "Kernel", "Kernel demonstrations", "Userspace services/drivers",
    "Userspace demonstrations", "Userspace test applications",
    "Shared runtimes/protocols/libraries", "Host tooling/build/configuration",
]


def cloc(*args):
    result = subprocess.run(
        ["cloc", "--quiet", "--json", *args], cwd=ROOT, check=True,
        text=True, capture_output=True,
    )
    return json.loads(result.stdout)


def component(path):
    parts = Path(path).parts
    if path == "crates/catten/src/demo.rs":
        return "Kernel demonstrations"
    if path.startswith("crates/catten/"):
        return "Kernel"
    if path.startswith("crates/catten-user/") or path.startswith(
        "crates/catten-services/examples/"
    ):
        return "Userspace demonstrations"
    if path.startswith("crates/catten-services/src/bin/"):
        name = Path(parts[4]).stem
        if name in DEMOS:
            return "Userspace demonstrations"
        if name in PROBES:
            return "Userspace test applications"
        return "Userspace services/drivers"
    if "examples" in parts:
        return "Host tooling/build/configuration"
    if path.startswith("crates/"):
        return "Shared runtimes/protocols/libraries"
    return "Host tooling/build/configuration"


def dedicated_test(path):
    p = Path(path)
    return (
        component(path) == "Userspace test applications"
        or path in {"crates/catten-services/src/bin/raft.rs",
                    "crates/catten-services/src/bin/servicemgr.rs"}
        or path in VALIDATION_FILES
        or any(part in {"tests", "self_test", "test_data", "test_support", "fixtures"}
               or part.endswith("_tests") for part in p.parts)
        or p.stem in {"tests", "boot_fixture", "hash_contract", "audit_receive_lease"}
        or p.stem.startswith(("test_", "test-", "check-", "verify-"))
        or p.stem.endswith(("_tests", "_test", "-test", "-tests", "_smoke", "-probe"))
        or path.startswith(".github/workflows/")
    )


# Preserve offsets/newlines while masking Rust comments, strings and characters.
# Lifetimes are left intact; raw-string hashes and nested block comments are
# handled explicitly so HTML, assembly and source snippets cannot fake items.
NONCODE = re.compile(
    r'''//[^\n]*|/\*|\b(?:br|cr|r)(\#{0,255})"|(?:b|c)?"|'''
    r'''b?'(?:\\(?:u\{[0-9A-Fa-f_]+\}|x[0-9A-Fa-f]{2}|[^\n])|[^\\'\n])' ''',
    re.VERBOSE,
)


def rust_mask(text):
    chars = list(text)
    cursor = 0
    while match := NONCODE.search(text, cursor):
        token = match.group()
        end = match.end()
        if token == "/*":
            depth = 1
            while depth:
                next_token = re.search(r"/\*|\*/", text[end:])
                if next_token is None:
                    raise ValueError("Unterminated Rust block comment")
                depth += 1 if next_token.group() == "/*" else -1
                end += next_token.end()
        elif match.group(1) is not None:
            close = '"' + match.group(1)
            found = text.find(close, end)
            if found < 0:
                raise ValueError("Unterminated Rust raw string")
            end = found + len(close)
        elif token in {'"', 'b"', 'c"'}:
            while end < len(text):
                if text[end] == "\\":
                    end += 2
                elif text[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
            else:
                raise ValueError("Unterminated Rust string")
        for index in range(match.start(), end):
            if chars[index] != "\n":
                chars[index] = " "
        cursor = end
    return "".join(chars)


ITEM_START = re.compile(
    r"#\s*\[\s*(?:cfg\s*\(\s*test\s*\)|test)\s*\]"
    r"|\bmod\s+(?:tests|\w+_tests)\s*(?=\{)"
    r"|\bfn\s+test_\w+\b"
)


def item_end(mask, start):
    parens = brackets = 0
    macro_call = False
    for index in range(start, len(mask)):
        char = mask[index]
        if char == "(":
            macro_call |= mask[start:index].rstrip().endswith("!")
            parens += 1
        elif char == ")":
            parens -= 1
        elif char == "[":
            brackets += 1
        elif char == "]":
            brackets -= 1
        elif not parens and not brackets:
            if char == ";" or (char == "," and macro_call):
                return index + 1
            if char == "{":
                depth = 1
                for end in range(index + 1, len(mask)):
                    depth += (mask[end] == "{") - (mask[end] == "}")
                    if not depth:
                        return end + 1
                raise ValueError("Unbalanced Rust test item")
    raise ValueError("Unterminated Rust test item")


def split_rust(text):
    mask = rust_mask(text)
    flags = [False] * len(text.splitlines(keepends=True))
    starts = {match.start() for match in ITEM_START.finditer(mask)}
    for match in re.finditer(r"#\s*\[\s*cfg\s*\(", mask):
        end = mask.index("]", match.end()) + 1
        gate = re.sub(r"\s+", "", text[match.start():end])
        # These gates require tests on every target. Do not classify a generic
        # any(kernel_tests, target_arch=...) gate as purely test-related.
        if gate in {'#[cfg(feature="kernel_tests")]',
                    '#[cfg(any(test,feature="kernel_tests"))]'} or gate.startswith(
                        '#[cfg(all(feature="kernel_tests",'):
            starts.add(match.start())
    for start in sorted(starts):
        end = item_end(mask, start)
        first_line = text.count("\n", 0, start)
        last_line = text.count("\n", 0, end - 1)
        flags[first_line:last_line + 1] = [True] * (last_line - first_line + 1)
    lines = text.splitlines(keepends=True)
    test = "".join(line if flag else "\n" for flag, line in zip(flags, lines))
    other = "".join("\n" if flag else line for flag, line in zip(flags, lines))
    return test, other


def source_files(data):
    return {str(Path(name).resolve().relative_to(ROOT)): values
            for name, values in data.items() if name not in {"header", "SUM"}}


def test_counts(files):
    counts = {path: values["code"] if dedicated_test(path) else 0
              for path, values in files.items()}
    pairs = {}
    with tempfile.TemporaryDirectory(prefix="charlotte-loc-") as directory:
        for index, (path, values) in enumerate(files.items()):
            if values["language"] != "Rust" or dedicated_test(path):
                continue
            try:
                test, other = split_rust((ROOT / path).read_text())
            except ValueError as error:
                raise ValueError(f"{path}: {error}") from error
            if not test.strip():
                continue
            pairs[index] = path
            for suffix, content in [("test", test), ("other", other)]:
                Path(directory, f"{index}_{suffix}.rs").write_text(content)
        fragments = cloc(directory, "--by-file", "--skip-uniqueness")
        code = {Path(name).stem: row["code"] for name, row in fragments.items()
                if name not in {"header", "SUM"}}
        for index, path in pairs.items():
            test = code.get(f"{index}_test", 0)
            other = code.get(f"{index}_other", 0)
            if test + other != files[path]["code"]:
                raise ValueError(f"Rust split changed cloc code total: {path}: "
                                 f"{test}+{other} != {files[path]['code']}")
            counts[path] = test
    return counts


def language_table(data):
    lines = [f"{'Language':<22} {'files':>7} {'blank':>10} {'comment':>10} {'code':>12}", "-" * 65]
    for name, row in sorted(data.items(), key=lambda item: item[1]["code"], reverse=True):
        lines.append(f"{name:<22} {row['nFiles']:>7,} {row['blank']:>10,} "
                     f"{row['comment']:>10,} {row['code']:>12,}")
    total = {key: sum(row[key] for row in data.values())
             for key in ["nFiles", "blank", "comment", "code"]}
    lines.extend(["-" * 65, f"{'SUM':<22} {total['nFiles']:>7,} {total['blank']:>10,} "
                  f"{total['comment']:>10,} {total['code']:>12,}"])
    return "\n".join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "loc.txt")
    args = parser.parse_args()
    source = cloc("--vcs=git", "--exclude-dir=limine-binary,docs", "--exclude-ext=lock,md",
                  r"--not-match-f=^loc\.txt$", "--force-lang=Linker Script,x", "--by-file")
    docs = cloc('--vcs=git ls-files -- docs "*.md"', "--exclude-dir=limine-binary",
                "--force-lang=Text,cfg")
    files = source_files(source)
    tests = test_counts(files)
    languages = defaultdict(lambda: dict(nFiles=0, blank=0, comment=0, code=0))
    groups = defaultdict(lambda: [0, 0, 0])
    for path, row in files.items():
        lang = languages[row["language"]]
        lang["nFiles"] += 1
        for key in ["blank", "comment", "code"]:
            lang[key] += row[key]
        group = groups[component(path)]
        group[0] += 1
        group[1] += row["code"]
        group[2] += tests[path]
    total = sum(row[1] for row in groups.values())
    testing = sum(row[2] for row in groups.values())
    rust_testing = sum(tests[path] for path, row in files.items() if row["language"] == "Rust")
    assert total == source["SUM"]["code"]
    lines = ["CharlotteOS maintained repository counts", "Generated with cloc " + str(source["header"]["cloc_version"]),
             "", "Scope: Git-tracked working-tree files, including current local edits.",
             "Excluded: ignored/untracked files, downloaded/build dependencies, bundled",
             "limine-binary/, dependency lockfiles and this report. Standard cloc duplicate",
             "filtering applies within each section; binaries/unrecognized files are ignored.",
             "Linker *.x files count as Linker Script; TLC *.cfg inputs count as Text.",
             "", "SOURCE, TESTS, TOOLING AND CONFIGURATION", "", language_table(dict(languages)),
             "", "SOURCE DISTRIBUTION (same source total; shared dependencies counted once)", "",
             f"{'Component':<35} {'files':>6} {'code':>10} {'test/validation':>16} {'other code':>12}", "-" * 83]
    for name in COMPONENTS:
        count, code, test = groups[name]
        lines.append(f"{name:<35} {count:>6,} {code:>10,} {test:>16,} {code-test:>12,}")
    lines.extend(["-" * 83, f"{'SUM':<35} {len(files):>6,} {total:>10,} {testing:>16,} {total-testing:>12,}",
                  f"Identified test/validation code: {testing:,} lines ({100*testing/total:.1f}% of source code).", "",
                  f"Rust-only test/validation code: {rust_testing:,} of {languages['Rust']['code']:,} code lines",
                  f"({100*rust_testing/languages['Rust']['code']:.1f}%).", "",
                  "Attribution rules and limits:",
                  "- Kernel includes native self-tests and adjacent subsystem fixtures; kernel",
                  "  demonstrations are listed separately. 'Other code' approximates kernel",
                  "  proper but still includes unclassified diagnostic/fixture scaffolding.",
                  "- Userspace demonstrations: catten-user, catten-services/examples, echo, greet.",
                  textwrap.fill("- Userspace test applications: " + ", ".join(sorted(PROBES)) + ".",
                                width=79, subsequent_indent="  "),
                  "- Runtime service binaries are separate from the shared catten-services SDK,",
                  "  catten-rt/syscall, protocol, connector, crypto and Raft libraries.",
                  "  Standalone raft/servicemgr fixture servers remain in the services category",
                  "  with their code counted as test-related; production DNS owns durable Raft.",
                  "- Dedicated tests/self_test/*_tests/fixtures paths, test/check/verify and",
                  "  smoke/probe filenames, CI workflows, selected fixture-generation inputs and",
                  "  boot_fixture/hash_contract/audit_receive_lease files count as validation.",
                  "- Inline Rust test-only cfg items (including kernel_tests), test modules,",
                  "  #[test] items and test_* helpers",
                  "  are split into fragments; cloc checks test + other == original per file.",
                  "- This is an identifiable-source lower bound, not a build-size estimate.",
                  "  Already cfg(test)-gated code is included; scattered fixture constants,",
                  "  imports, call sites and mixed boot/test-runner branches may remain in other",
                  "  code. No claim is made that all other code belongs in a stripped image.",
                  "  Test-free images require gated boot registrations and bundle",
                  "  selection; docs/reference/boot-modes.md defines the implemented profiles",
                  "  and artifact checks. Mandatory startup memory validation is operational",
                  "  code; these counts do not classify every runtime invariant as a test.",
                  "- Formal models, model configurations and audit evidence are below, not in",
                  "  the source/test percentage. Prose and data also appear as cloc 'code'.", "",
                  "DOCUMENTATION, VERIFICATION MODELS AND RECORDS", "",
                  language_table({k: v for k, v in docs.items() if k not in {"header", "SUM"}}), "",
                  "Reproduce: python3 scripts/count-loc.py", "",
                  "Requires Python 3, Git and cloc; no third-party Python packages.",
                  "The classifier and explicit demo/probe lists live in scripts/count-loc.py.", ""])
    args.output.write_text("\n".join(lines))
    print(f"Wrote {args.output}: {total:,} source code lines; {testing:,} identified test/validation lines")


if __name__ == "__main__":
    main()
