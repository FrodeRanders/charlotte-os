#!/usr/bin/env bash
# Build the AArch64 EL0 service bundle and optionally refresh the kernel's
# embedded lifecycle-test images as one coherent ABI set.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="build"
if [ "${1:-}" = "--embed" ]; then
    MODE="embed"
elif [ "${1:-}" = "--check" ]; then
    MODE="check"
elif [ "$#" -ne 0 ]; then
    echo "usage: $0 [--embed|--check]" >&2
    exit 1
fi

MANIFEST="crates/catten-services/Cargo.toml"
TARGET="crates/catten-services/aarch64-unknown-none.json"
OUTPUT="crates/catten-services/target/aarch64-unknown-none/release"

cargo +nightly build --manifest-path "$MANIFEST" --target "$TARGET" \
    --release -Z build-std=core,alloc

if [ "$MODE" = "embed" ]; then
    for service in ns echo client uart cclient raft; do
        install -m 0755 "$OUTPUT/$service" \
            "crates/catten/src/self_test/$service.elf"
    done
    echo ">>> Refreshed embedded service bundle (ns, echo, client, uart, cclient, raft)."
elif [ "$MODE" = "check" ]; then
    stale=0
    for service in ns echo client uart cclient raft; do
        if ! cmp -s "$OUTPUT/$service" \
            "crates/catten/src/self_test/$service.elf"; then
            echo "error: embedded $service.elf is stale" >&2
            stale=1
        fi
    done
    if [ "$stale" -ne 0 ]; then
        echo "run scripts/build-catten-services.sh --embed" >&2
        exit 1
    fi
    echo ">>> Embedded service bundle matches the current release build."
fi
