#!/usr/bin/env bash
# Build the AArch64 EL0 service bundle and optionally stage it as an
# architecture-qualified, generated kernel input.
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
BUNDLE="$ROOT/target/embedded-services/aarch64-unknown-none"
SERVICES=(ns echo client uart cclient servicemgr raft nvme nvme_client objstore objstore_client fs net nclient tcpip)

if [ "$CLEAN" = "1" ]; then
    echo ">>> Cleaning service target artifacts..."
    cargo clean --manifest-path "$MANIFEST" --target "$TARGET" 2>/dev/null || true
    rm -rf "$BUNDLE"
    echo ">>> Forcing clean rebuild of all EL0 services..."
fi

cargo build --manifest-path "$MANIFEST" --target "$TARGET" \
    --release -Z build-std=core,alloc

if [ "$MODE" = "embed" ]; then
    mkdir -p "$BUNDLE"
    for service in "${SERVICES[@]}"; do
        install -m 0755 "$OUTPUT/$service" \
            "$BUNDLE/$service.elf"
    done
    echo ">>> Staged AArch64 EL0 service bundle at $BUNDLE."
elif [ "$MODE" = "check" ]; then
    stale=0
    for service in "${SERVICES[@]}"; do
        if ! cmp -s "$OUTPUT/$service" \
            "$BUNDLE/$service.elf"; then
            echo "error: staged AArch64 $service.elf is absent or stale" >&2
            stale=1
        fi
    done
    if [ "$stale" -ne 0 ]; then
        echo "run scripts/build-catten-services.sh --embed" >&2
        exit 1
    fi
    echo ">>> Staged AArch64 service bundle matches the current release build."
fi
