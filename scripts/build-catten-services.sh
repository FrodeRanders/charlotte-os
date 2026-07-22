#!/usr/bin/env bash
# Build the AArch64 EL0 service bundle and optionally refresh the kernel's
# embedded lifecycle-test images as one coherent ABI set.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="build"
CLEAN=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --embed) MODE="embed"; shift ;;
        --check) MODE="check"; shift ;;
        --clean) CLEAN=1; shift ;;
        *) echo "usage: $0 [--embed|--check] [--clean]" >&2; exit 1 ;;
    esac
done

MANIFEST="crates/catten-services/Cargo.toml"
TARGET="crates/catten-services/aarch64-unknown-none.json"
OUTPUT="crates/catten-services/target/aarch64-unknown-none/release"

if [ "$CLEAN" = "1" ]; then
    echo ">>> Cleaning service target artifacts..."
    cargo clean --manifest-path "$MANIFEST" --target "$TARGET" 2>/dev/null || true
    rm -f crates/catten/src/self_test/{ns,echo,client,uart,cclient,raft}.elf
    echo ">>> Forcing clean rebuild of all EL0 services..."
fi

cargo build --manifest-path "$MANIFEST" --target "$TARGET" \
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
