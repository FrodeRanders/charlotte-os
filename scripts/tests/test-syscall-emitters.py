#!/usr/bin/env python3
"""Verify every AArch64 syscall wrapper has a matching svcN emitter.

The AArch64 emitters are a hand-maintained match table; a missing arm turns a
syscall into a userspace panic at runtime. This check derives the declared
number list and every `svcN(SyscallNumber::Variant, ...)` call site from the
source and requires the corresponding emitter arm to exist in that arity's
table. Regression evidence: docs/reports/audits/2026-10-10-security-audit.md
(SEC-34).
"""

import re
import sys
from pathlib import Path

SOURCE = Path(__file__).resolve().parents[2] / "crates/catten-syscall/src/lib.rs"


def parse_declared_numbers(text):
    match = re.search(r"define_syscall_numbers!\((.*?)\n\);", text, re.DOTALL)
    if match is None:
        raise SystemExit("error: define_syscall_numbers! invocation not found")
    numbers = {}
    for variant, number in re.findall(r"\(\s*(\w+)\s*,\s*(\d+)\s*\)", match.group(1)):
        numbers[variant] = int(number)
    if not numbers:
        raise SystemExit("error: syscall number list is empty")
    return numbers


def parse_emitters(text):
    emitters = {}
    for function_match in re.finditer(r"unsafe fn (svc\d+(?:_x\d)?)\s*\(", text):
        name = function_match.group(1)
        line_start = text.rfind("\n", 0, function_match.start()) + 1
        prefix = text[max(0, line_start - 240) : function_match.start()]
        if 'target_arch = "aarch64"' not in prefix.split("#[cfg(target_arch = \"x86_64\")]")[-1]:
            continue
        brace = text.find("{", function_match.end())
        if brace < 0:
            continue
        depth = 0
        end = brace
        for index in range(brace, len(text)):
            if text[index] == "{":
                depth += 1
            elif text[index] == "}":
                depth -= 1
                if depth == 0:
                    end = index
                    break
        body = text[brace : end + 1]
        arms = set()
        for arm_number, svc_number in re.findall(r'(\d+)\s*=>\s*asm!\([^\n]*?"svc #(\d+)"', body):
            if arm_number != svc_number:
                raise SystemExit(
                    f"error: {name} arm {arm_number} emits svc #{svc_number}"
                )
            arms.add(int(arm_number))
        if arms:
            emitters[name] = arms
    return emitters


def parse_calls(text):
    calls = []
    for call in re.finditer(r"(svc\d+(?:_x\d)?)\(\s*SyscallNumber::(\w+)", text):
        calls.append((call.group(1), call.group(2)))
    return calls


def main():
    text = SOURCE.read_text()
    numbers = parse_declared_numbers(text)
    emitters = parse_emitters(text)
    failures = []
    for function, variant in parse_calls(text):
        if variant not in numbers:
            failures.append(f"call {function}(SyscallNumber::{variant}) names an undeclared syscall")
            continue
        number = numbers[variant]
        if function not in emitters:
            failures.append(f"{function} has no AArch64 emitter table")
        elif number not in emitters[function]:
            failures.append(
                f"{function}(SyscallNumber::{variant}) needs an AArch64 `{number} => asm!(\"svc #{number}\")` arm"
            )
    if failures:
        for failure in failures:
            print(f"FAIL: {failure}")
        return 1
    print(
        f"syscall emitter coverage: {len(parse_calls(text))} call sites across "
        f"{len(emitters)} arities checked"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
