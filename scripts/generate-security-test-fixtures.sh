#!/usr/bin/env bash
# Creates fresh, independent TEST roots and exact signed policies. This does
# not provision production trust. The output directory must be newly empty.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE="${1:?expected source probe ELF}"
OUTPUT="${2:?expected fresh fixture directory}"
if [ ! -d "$OUTPUT" ] || [ -L "$OUTPUT" ]; then
    echo "error: fixture output must be a newly created directory" >&2
    exit 1
fi
shopt -s nullglob dotglob
ENTRIES=("$OUTPUT"/*)
if [ "${#ENTRIES[@]}" -ne 0 ]; then
    echo "error: fixture output must be empty; refusing to overwrite fixtures" >&2
    exit 1
fi
SIGNER="${ROOT}/target/debug/cluster-sign"
TOOLCHAIN="$(sed -n 's/^channel = "\([^"]*\)"/\1/p' "$ROOT/rust-toolchain.toml")"
(cd /tmp && cargo +"$TOOLCHAIN" build --locked --quiet --manifest-path "$ROOT/tools/cluster-sign/Cargo.toml")
"$SIGNER" generate "$OUTPUT/artifact.hex" "$OUTPUT/artifact.pub"
"$SIGNER" generate "$OUTPUT/deployment.hex" "$OUTPUT/deployment.pub"
install -m 0755 "$SOURCE" "$OUTPUT/probe.elf"
"$SIGNER" elf-sign "$OUTPUT/probe.elf" security_probe "$OUTPUT/artifact.hex" service 1 1 0x1 -
DIGEST="$("$SIGNER" sha256 "$OUTPUT/probe.elf")"
"$SIGNER" deployment-sign "$OUTPUT/admitted.cdep" security_probe security/probe.elf \
    "$DIGEST" 0 1 8 1 5000 "$OUTPUT/deployment.hex" \
    security.available=publish security.available=client security.missing=call tcpip=call \
    security.silent=publish security.silent=call
"$SIGNER" deployment-sign "$OUTPUT/alternate.cdep" security_probe security/probe.elf \
    "$DIGEST" 0 2 8 1 5000 "$OUTPUT/deployment.hex" \
    security.available=publish security.available=client security.missing=call tcpip=client \
    security.silent=publish security.silent=call
