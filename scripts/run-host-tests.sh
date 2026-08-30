#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
toolchain="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$repo_root/rust-toolchain.toml")"
host_cargo="$(rustup which --toolchain "$toolchain" cargo)"
host_rustc="$(rustup which --toolchain "$toolchain" rustc)"
host_rustdoc="$(rustup which --toolchain "$toolchain" rustdoc)"
host_work_dir="$(mktemp -d "${TMPDIR:-/tmp}/charlotte-host-tests.XXXXXX")"
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
    manifests+=("${manifest_path#"$repo_root"/}")
done

for manifest in "${manifests[@]}"; do
    # A package may legitimately disable test harnesses for its freestanding
    # binary targets while retaining host unit tests in its library. Cargo
    # applies `test = false` per target, so let it select the enabled targets.
    RUSTC="$host_rustc" RUSTDOC="$host_rustdoc" "$host_cargo" test \
        --manifest-path "$repo_root/$manifest"
done

RUSTC="$host_rustc" RUSTDOC="$host_rustdoc" "$host_cargo" run --quiet \
    --manifest-path "$repo_root/tools/cluster-sign/Cargo.toml" \
    -- selftest
