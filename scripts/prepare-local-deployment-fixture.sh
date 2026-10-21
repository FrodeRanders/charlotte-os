#!/usr/bin/env bash
# Signed local-cache fixture; the normal agent still verifies the selected pin.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUNDLE="${1:?expected native signed service bundle}"
OUTPUT="${2:?expected signed fixture output path}"
source "$ROOT/scripts/lib/signing-policy.sh"
TOOLCHAIN="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$ROOT/rust-toolchain.toml")"
(cd /tmp && cargo +"$TOOLCHAIN" build --locked --quiet --manifest-path "$ROOT/tools/cluster-sign/Cargo.toml")
SIGNER="$ROOT/target/debug/cluster-sign"
VARIANTS=()
ARM="$ROOT/target/embedded-services/aarch64-unknown-none/greet.elf"
X86="$ROOT/target/embedded-services/x86_64-unknown-none/greet.elf"
# Canonical ordering lets both fixture nodes carry exactly the same decision.
if [ -f "$ARM" ] && [ -f "$X86" ]; then
    PRIMARY="$ARM"
    VARIANTS=("--variant=fixture/greet-x86.elf,$X86,0,0,3")
else
    PRIMARY="$BUNDLE/greet.elf"
fi
mkdir -p "$(dirname "$OUTPUT")"
# Keep each run's signed parts in a fresh private directory for inspection.
PARTS="$(mktemp -d "${OUTPUT}.parts.XXXXXX")"
# Each phase has a signed node pin, so the ordinary automatic controller cannot
# relocate the test's initial peer before its cross-node call is observed.
for sequence in 1 2 3; do
    for node in 0x42e278ea 0x42e27737; do
        "$SIGNER" deployment-sign "${PARTS}/${sequence}.${node}" greet fixture/greet-arm.elf "$PRIMARY" \
            "$node" "$sequence" 4 1 5000 "$(catten_signing_key_file "$ROOT")" "${VARIANTS[@]}" greet=publish
    done
done
python3 - "$OUTPUT" "$PARTS" <<'PYTHON'
from pathlib import Path
import sys
output=Path(sys.argv[1])
parts=Path(sys.argv[2])
records=[]
for sequence in [1,2,3]:
    for node in ['0x42e278ea','0x42e27737']:
        data=(parts/f'{sequence}.{node}').read_bytes()
        records.append(len(data).to_bytes(4,'little')+data)
output.write_bytes(b''.join(records))
PYTHON
