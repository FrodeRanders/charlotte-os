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
SERVICES=(ns echo client uart cclient servicemgr raft nvme nvme_client objstore objstore_client fs net nclient relmsg rclient tcpip tcpclient httpd observe disco frouter dns agent clusterctl greet)
# Services signed with the cluster's private key before the kernel embeds
# them: the EL0 loader verifies agent.elf's signature note at every boot, and
# the note-signed greet.elf is the cluster-deployed artifact. This is the
# demo keypair (tools/cluster-sign generate); the real key ceremony is
# future work.
DEMO_PRIVATE_KEY="40be67ef70344da61676fbf898dc8e63f3c79d628ae90bb91d3a83948e48947d3ddc95c26bd5f4022d95a4c6c8d074f577f11af7873e527b018b21be2c035463"
SIGNED_SERVICES=(agent greet)

sign_elf() {
    local elf="$1"
    # The tool builds cleanly only when cargo's config discovery starts
    # outside the repo (the root config pins build-std for the kernel
    # toolchain); run it from /tmp with an explicit manifest path.
    (cd /tmp && cargo run --quiet --manifest-path "$ROOT/tools/cluster-sign/Cargo.toml"         -- elf-sign "$elf" "$DEMO_PRIVATE_KEY" >/dev/null)
}

verify_elf() {
    local elf="$1"
    (cd /tmp && cargo run --quiet --manifest-path "$ROOT/tools/cluster-sign/Cargo.toml"         -- elf-verify "$elf" "3ddc95c26bd5f4022d95a4c6c8d074f577f11af7873e527b018b21be2c035463")
}

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
    for service in "${SIGNED_SERVICES[@]}"; do
        echo ">>> Signing $service.elf (cluster Ed25519 note)..."
        sign_elf "$BUNDLE/$service.elf"
    done
    echo ">>> Staged AArch64 EL0 service bundle at $BUNDLE."
elif [ "$MODE" = "check" ]; then
    stale=0
    for service in "${SERVICES[@]}"; do
        if [ -n "${SIGNED_SERVICES[*]}" ] && [[ " ${SIGNED_SERVICES[*]} " == *" $service "* ]]; then
            # The signed ELFs are deliberately not byte-identical to the
            # build output; verify their signatures instead.
            if ! verify_elf "$BUNDLE/$service.elf" >/dev/null 2>&1; then
                echo "error: staged AArch64 $service.elf signature is invalid" >&2
                stale=1
            fi
        elif ! cmp -s "$OUTPUT/$service" \
            "$BUNDLE/$service.elf"; then
            echo "error: staged AArch64 $service.elf is absent or stale" >&2
            stale=1
        fi
    done
    if [ "$stale" -ne 0 ]; then
        echo "run scripts/build-catten-services.sh --embed" >&2
        exit 1
    fi
    echo ">>> Staged AArch64 service bundle matches the current release build (signatures verified)."
fi
