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
SERVICES=(ns echo client uart cclient servicemgr raft nvme nvme_client objstore objstore_client fs net nclient relmsg rclient tcpip tcpclient httpd time s3 s3_smoke kafka kafka_smoke kafka_fence_smoke kafka_step kafka_step_proc kafka_step_input rng observe disco frouter dns agent clusterctl deployd grantctl greet)

if [ "$CLEAN" = "1" ]; then
    echo ">>> Cleaning service target artifacts..."
    cargo clean --manifest-path "$MANIFEST" --target "$TARGET" \
        --target-dir crates/catten-services/target 2>/dev/null || true
    rm -rf "$BUNDLE"
    echo ">>> Forcing clean rebuild of all EL0 services..."
fi

cargo build --manifest-path "$MANIFEST" --target "$TARGET" \
    --target-dir crates/catten-services/target \
    --release -Z build-std=core,alloc

if [ "$MODE" = "embed" ]; then
    mkdir -p "$BUNDLE"
    for service in "${SERVICES[@]}"; do
        install -m 0755 "$OUTPUT/$service" \
            "$BUNDLE/$service.elf"
    done
    "$ROOT/scripts/sign-service-elfs.sh" "$BUNDLE" "${SERVICES[@]}"
    echo ">>> Staged and signed AArch64 EL0 service bundle at $BUNDLE."
elif [ "$MODE" = "check" ]; then
    # Reproduce the signed bundle from the just-built binaries and compare it
    # byte-for-byte. Signature validity alone does not prove that a staged ELF
    # corresponds to current source.
    CHECK_BUNDLE="$(mktemp -d /tmp/catten-service-check.XXXXXX)"
    trap 'rm -rf -- "$CHECK_BUNDLE"' EXIT
    for service in "${SERVICES[@]}"; do
        install -m 0755 "$OUTPUT/$service" "$CHECK_BUNDLE/$service.elf"
    done
    "$ROOT/scripts/sign-service-elfs.sh" "$CHECK_BUNDLE" >/dev/null
    stale=0
    for service in "${SERVICES[@]}"; do
        if [ ! -f "$BUNDLE/$service.elf" ]; then
            echo "error: staged AArch64 $service.elf is missing" >&2
            stale=1
        elif ! cmp -s "$BUNDLE/$service.elf" "$CHECK_BUNDLE/$service.elf"; then
            echo "error: staged AArch64 $service.elf is stale" >&2
            stale=1
        fi
    done
    for elf in "$BUNDLE"/*.elf; do
        [ -f "$elf" ] || continue
        if ! (cd /tmp && cargo run --quiet --manifest-path "$ROOT/tools/cluster-sign/Cargo.toml" \
            -- elf-verify "$elf" "$(basename "$elf" .elf)" \
            "3ddc95c26bd5f4022d95a4c6c8d074f577f11af7873e527b018b21be2c035463" \
            >/dev/null 2>&1); then
            echo "error: staged AArch64 $(basename "$elf") signature or identity is invalid" >&2
            stale=1
        fi
    done
    if [ "$stale" -ne 0 ]; then
        echo "run scripts/build-catten-services.sh --embed" >&2
        exit 1
    fi
    echo ">>> Staged AArch64 service bundle signatures verified."
fi
