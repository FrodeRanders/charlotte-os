#!/usr/bin/env bash
#
# sign-service-elfs.sh — sign every staged service ELF in a bundle directory
# with the cluster's private key (the version-controlled development key by
# default, or the file at $CLUSTER_SIGN_KEY_FILE). Called by the build scripts after
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
source "$ROOT/scripts/lib/signing-policy.sh"
catten_require_development_trust
SIGN_TOOLCHAIN="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$ROOT/rust-toolchain.toml")"
BUNDLE="${1:?usage: sign-service-elfs.sh <bundle-dir> [service-name ...]}"
shift
BUNDLE="$(cd "$BUNDLE" && pwd)"
POLICY="$ROOT/crates/catten-services/artifact-policy.tsv"

KEY_FILE="$(catten_signing_key_file "$ROOT")"
echo ">>> DEVELOPMENT signing: fixture-root images are not suitable for real credentials."

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

[ "${#ELFS[@]}" -ne 0 ] || exit 0

# Build once per batch using the pinned host toolchain. Each ELF still gets a
# separate signer process/key owner, without starting Cargo for every artifact.
# Starting outside the repo avoids its freestanding build-std configuration.
SIGNER="$(cd /tmp && cargo +"$SIGN_TOOLCHAIN" build --locked --quiet \
    --manifest-path "$ROOT/tools/cluster-sign/Cargo.toml" --bin cluster-sign \
    --message-format=json | python3 -c '
import json, sys
executable = None
for line in sys.stdin:
    row = json.loads(line)
    if row.get("reason") == "compiler-message":
        sys.stderr.write(row.get("message", {}).get("rendered") or "")
    if row.get("reason") == "compiler-artifact" and row.get("target", {}).get("name") == "cluster-sign":
        executable = row.get("executable") or executable
if executable is None:
    raise SystemExit("error: Cargo did not identify the pinned signing executable")
print(executable)
')"

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
    (cd /tmp && "$SIGNER" elf-sign "$elf" "$name" "$KEY_FILE" "$class" "$version" "$rollback" \
        "$flags" "$provenance" >/dev/null)
    echo ">>> Blessed $(basename "$elf") as $name ($class, release $version, flags $flags)."
done
