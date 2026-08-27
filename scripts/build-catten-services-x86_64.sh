#!/usr/bin/env bash
# Build, stage, and sign the x86_64 EL0 service bundle embedded by the kernel.
#
# Usage:
#   scripts/build-catten-services-x86_64.sh [--clean]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CLEAN=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --clean) CLEAN=1; shift ;;
        *) echo "usage: $0 [--clean]" >&2; exit 1 ;;
    esac
done

MANIFEST="crates/catten-services/Cargo.toml"
TARGET="crates/catten-services/x86_64-unknown-none.json"
OUTPUT="crates/catten-services/target/x86_64-unknown-none/release"
BUNDLE="$ROOT/target/embedded-services/x86_64-unknown-none"
BUNDLE_PARENT="$(dirname "$BUNDLE")"
SERVICES=(
    ns observe nvme objstore nvme_client objstore_client echo raft client
    servicemgr ahci virtio_blk net e1000e nclient disco frouter dns agent
    greet relmsg rclient tcpip tcpclient httpd time s3 s3_smoke kafka kafka_smoke deployd
    rng fs clusterctl grantctl
)

if [ "$CLEAN" = "1" ]; then
    echo ">>> Cleaning x86_64 service target artifacts..."
    cargo clean --manifest-path "$MANIFEST" --target "$TARGET" \
        --target-dir crates/catten-services/target
    rm -rf "$BUNDLE"
fi

build_bins=()
for service in "${SERVICES[@]}"; do
    build_bins+=(--bin "$service")
done

echo ">>> Building x86_64 EL0 services..."
cargo build --manifest-path "$MANIFEST" --target "$TARGET" \
    --target-dir crates/catten-services/target \
    --release -Z build-std=core,alloc "${build_bins[@]}"

mkdir -p "$BUNDLE_PARENT"
STAGING="$(mktemp -d "$BUNDLE_PARENT/.x86_64-services.XXXXXX")"
cleanup() {
    if [ -n "${STAGING:-}" ] && [ -d "$STAGING" ]; then
        rm -rf -- "$STAGING"
    fi
}
trap cleanup EXIT

for service in "${SERVICES[@]}"; do
    install -m 0755 "$OUTPUT/$service" "$STAGING/$service.elf"
done
"$ROOT/scripts/sign-service-elfs.sh" "$STAGING" >/dev/null

rm -rf -- "$BUNDLE"
mv "$STAGING" "$BUNDLE"
STAGING=""
trap - EXIT

echo ">>> Staged and signed x86_64 EL0 service bundle at $BUNDLE."
