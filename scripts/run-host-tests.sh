#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
toolchain="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$repo_root/rust-toolchain.toml")"
host_work_dir="$(mktemp -d "${TMPDIR:-/tmp}/charlotte-host-tests.XXXXXX")"

rg --version
trap 'rmdir "$host_work_dir"' EXIT

# Cargo discovers .cargo/config.toml from the invocation directory. CharlotteOS'
# root config builds core for freestanding targets, so invoke host tests from a
# temporary directory while retaining absolute manifest paths.
cd "$host_work_dir"

manifests=()
for manifest_path in "$repo_root"/crates/*/Cargo.toml; do
    crate_dir="${manifest_path%/Cargo.toml}"
    if ! rg --quiet '#[[:space:]]*\[[[:space:]]*test[[:space:]]*\]' \
        "$crate_dir" --glob '*.rs'; then
        continue
    fi
    if rg --quiet '^[[:space:]]*test[[:space:]]*=[[:space:]]*false' \
        "$manifest_path"; then
        echo "error: ${manifest_path#"$repo_root"/} contains tests but disables its test harness" >&2
        exit 1
    fi
    manifests+=("${manifest_path#"$repo_root"/}")
done

for manifest in "${manifests[@]}"; do
    cargo "+$toolchain" test --manifest-path "$repo_root/$manifest"
done

cargo "+$toolchain" run --quiet \
    --manifest-path "$repo_root/tools/cluster-sign/Cargo.toml" \
    -- selftest
