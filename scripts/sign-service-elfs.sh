#!/usr/bin/env bash
#
# sign-service-elfs.sh — sign every staged service ELF in a bundle directory
# with the cluster's private key (the version-controlled development key by
# default, or $CLUSTER_SIGN_PRIVATE_KEY). Called by the build scripts after
# staging so that every image the kernel embeds — and therefore every image
# the EL0 loader accepts — carries a valid .note.charlotte-sig signature.
#
# Usage: scripts/sign-service-elfs.sh <bundle-dir> [service-name ...]
#
# With no service names, every ELF in the bundle is signed. Supplying names
# signs only those ELFs, which lets a later staging step add one artifact
# without needlessly re-signing the existing bundle.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE="${1:?usage: sign-service-elfs.sh <bundle-dir> [service-name ...]}"
shift
BUNDLE="$(cd "$BUNDLE" && pwd)"
POLICY="$ROOT/crates/catten-services/artifact-policy.tsv"

if [ -n "${CLUSTER_SIGN_PRIVATE_KEY:-}" ]; then
    PRIVATE_KEY="$CLUSTER_SIGN_PRIVATE_KEY"
else
    KEY_FILE="$ROOT/tools/cluster-sign/dev-key.hex"
    # The key file carries a comment line; take the last non-empty line.
    PRIVATE_KEY="$(grep -v '^#' "$KEY_FILE" | tr -d '[:space:]')"
fi

ELFS=()
if [ "$#" -eq 0 ]; then
    for elf in "$BUNDLE"/*.elf; do
        [ -f "$elf" ] || continue
        ELFS+=("$elf")
    done
else
    for name in "$@"; do
        case "$name" in
            */*|*.elf)
                echo "error: service name must be a bare name without .elf: $name" >&2
                exit 1
                ;;
        esac
        elf="$BUNDLE/$name.elf"
        if [ ! -f "$elf" ]; then
            echo "error: requested service ELF is missing: $elf" >&2
            exit 1
        fi
        ELFS+=("$elf")
    done
fi

for elf in "${ELFS[@]}"; do
    name="$(basename "$elf" .elf)"
    matches="$(awk -v wanted="$name" '$1 == wanted { count++ } END { print count + 0 }' "$POLICY")"
    if [ "$matches" -ne 1 ]; then
        echo "error: $name must have exactly one blessing policy row in $POLICY" >&2
        exit 1
    fi
    row="$(awk -v wanted="$name" '$1 == wanted { print; exit }' "$POLICY")"
    read -r policy_name class version rollback flags provenance <<EOF
$row
EOF
    if [ "$policy_name" != "$name" ]; then
        echo "error: blessing policy parser mismatch for $name" >&2
        exit 1
    fi
    # The tool builds cleanly only when cargo's config discovery starts
    # outside the repo (the root config pins build-std for the kernel
    # toolchain); run it from /tmp with an explicit manifest path.
    (cd /tmp && cargo run --quiet --manifest-path "$ROOT/tools/cluster-sign/Cargo.toml" \
        -- elf-sign "$elf" "$name" "$PRIVATE_KEY" "$class" "$version" "$rollback" \
        "$flags" "$provenance" >/dev/null)
    echo ">>> Blessed $(basename "$elf") as $name ($class, release $version, flags $flags)."
done
