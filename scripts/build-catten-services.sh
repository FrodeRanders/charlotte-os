#!/usr/bin/env bash
# Build the AArch64 EL0 service bundle and optionally stage it as an
# architecture-qualified, generated kernel input.
#
# Every staged ELF is signed with the cluster's private key (the
# version-controlled development key in tools/cluster-sign/dev-key.hex by
# default, $CLUSTER_SIGN_PRIVATE_KEY for a live key) before the kernel
# embeds it: the EL0 loader refuses any image that is not validly signed, so
# the whole bundle must carry signature notes.
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
SERVICES=(ns echo client uart cclient servicemgr raft nvme nvme_client objstore objstore_client fs net nclient relmsg rclient tcpip tcpclient httpd observe disco frouter dns agent clusterctl greet)

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
    "$ROOT/scripts/sign-service-elfs.sh" "$BUNDLE"
    echo ">>> Staged and signed AArch64 EL0 service bundle at $BUNDLE."
elif [ "$MODE" = "check" ]; then
    # The staged ELFs are deliberately not byte-identical to the build
    # output (each carries an embedded signature note); verify the
    # signatures instead, and require every staged service to be present.
    stale=0
    for service in "${SERVICES[@]}"; do
        if [ ! -f "$BUNDLE/$service.elf" ]; then
            echo "error: staged AArch64 $service.elf is missing" >&2
            stale=1
        fi
    done
    for elf in "$BUNDLE"/*.elf; do
        [ -f "$elf" ] || continue
        if ! (cd /tmp && cargo run --quiet --manifest-path "$ROOT/tools/cluster-sign/Cargo.toml" \
            -- elf-verify "$elf" "3ddc95c26bd5f4022d95a4c6c8d074f577f11af7873e527b018b21be2c035463" \
            >/dev/null 2>&1); then
            echo "error: staged AArch64 $(basename "$elf") signature is invalid" >&2
            stale=1
        fi
    done
    if [ "$stale" -ne 0 ]; then
        echo "run scripts/build-catten-services.sh --embed" >&2
        exit 1
    fi
    echo ">>> Staged AArch64 service bundle signatures verified."
fi
