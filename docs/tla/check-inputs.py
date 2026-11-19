#!/usr/bin/env python3
"""Validate literal TLC registrations, including case on macOS filesystems."""

import argparse
from pathlib import Path
import re
import shlex


def validate_inputs(model_dir, selected_module=""):
    registry = (model_dir / "check.sh").read_text().replace("\\\n", " ")
    cases = []
    for line in registry.splitlines():
        if re.match(r"^run_(?:model|expected_violation)\s+", line):
            _, module, config, *_ = shlex.split(line, comments=True)
            if not selected_module or module == selected_module:
                cases.append((module, config))
    if not cases:
        raise ValueError(f"no configurations registered for {selected_module or 'TLC'}")

    # Path.exists() accepts the wrong capitalization on common macOS volumes.
    # Compare actual directory entries so this check behaves like Linux CI.
    names = {path.name for path in model_dir.iterdir()}
    errors = []
    modules = {module for module, _ in cases}
    required = {module + ".tla" for module in modules}
    required.update(config for _, config in cases)
    for name in sorted(required):
        if name not in names:
            similar = sorted(candidate for candidate in names
                             if candidate.casefold() == name.casefold())
            hint = f" (filename case differs: {', '.join(similar)})" if similar else ""
            errors.append(f"missing TLC input {name}{hint}")

    for module in sorted(modules):
        name = module + ".tla"
        if name not in names:
            continue
        header = re.search(r"^-+\s*MODULE\s+(\w+)\s*-+",
                           (model_dir / name).read_text(), re.MULTILINE)
        if header is None or header[1] != module:
            actual = header[1] if header else "no module header"
            errors.append(f"{name} must declare MODULE {module}; found {actual}")
    if errors:
        raise ValueError("\n".join(errors))
    return len(modules), len(cases)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("module", nargs="?", default="")
    args = parser.parse_args()
    try:
        modules, cases = validate_inputs(Path(__file__).resolve().parent, args.module)
    except (OSError, ValueError) as error:
        parser.exit(1, f"error: {error}\n")
    print(f">>> TLC inputs verified: {modules} modules, {cases} configurations")


if __name__ == "__main__":
    main()
