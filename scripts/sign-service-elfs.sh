#!/usr/bin/env bash
#
# sign-service-elfs.sh — sign every staged service ELF in a bundle directory
# with the cluster's private key (the version-controlled development key by
# default, or $CLUSTER_SIGN_PRIVATE_KEY). Called by the build scripts after
# staging so that every image the kernel embeds — and therefore every image
# the EL0 loader accepts — carries a valid .note.charlotte-sig signature.
#
# Usage: scripts/sign-service-elfs.sh <bundle-dir>
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE="${1:?usage: sign-service-elfs.sh <bundle-dir>}"

if [ -n "${CLUSTER_SIGN_PRIVATE_KEY:-}" ]; then
    PRIVATE_KEY="$CLUSTER_SIGN_PRIVATE_KEY"
else
    KEY_FILE="$ROOT/tools/cluster-sign/dev-key.hex"
    # The key file carries a comment line; take the last non-empty line.
    PRIVATE_KEY="$(grep -v '^#' "$KEY_FILE" | tr -d '[:space:]')"
fi

for elf in "$BUNDLE"/*.elf; do
    [ -f "$elf" ] || continue
    # The tool builds cleanly only when cargo's config discovery starts
    # outside the repo (the root config pins build-std for the kernel
    # toolchain); run it from /tmp with an explicit manifest path.
    (cd /tmp && cargo run --quiet --manifest-path "$ROOT/tools/cluster-sign/Cargo.toml" \
        -- elf-sign "$elf" "$PRIVATE_KEY" >/dev/null)
    echo ">>> Signed $(basename "$elf")."
done
