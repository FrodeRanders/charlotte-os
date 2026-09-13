#!/usr/bin/env bash
#
# export-app-sdk.sh -- package the CharlotteOS application SDK.
#
# The tarball contains everything a new out-of-tree CharlotteOS project needs
# to build an EL0 ELF and sign/deploy artifacts, without a CharlotteOS
# checkout:
#
#   build-external-elf.sh   platform build wrapper (context sensitive)
#   platform/<arch>/        target specification and linker script
#   signer/                 standalone workspace with cluster-sign and
#                           charlotte-launch
#   keys/                   the publicly known development keys
#   VERSION                 sdk schema, OS revision, pinned toolchain
#
# Usage:
#   scripts/export-app-sdk.sh [--output PATH]
#
# The output defaults to target/charlotte-app-sdk-<short-revision>.tar.gz and
# a <output>.sha256 sidecar is written.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTPUT=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --output) OUTPUT="$2"; shift 2 ;;
        *) echo "usage: $0 [--output PATH]" >&2; exit 2 ;;
    esac
done

REVISION="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
TOOLCHAIN="$(sed -n 's/^channel = "\(.*\)"/\1/p' "$ROOT/rust-toolchain.toml" | head -1)"
if [ -z "$OUTPUT" ]; then
    OUTPUT="$ROOT/target/charlotte-app-sdk-${REVISION:0:12}.tar.gz"
fi
mkdir -p "$(dirname "$OUTPUT")"

STAGE="$(mktemp -d "${TMPDIR:-/tmp}/charlotte-app-sdk.XXXXXX")"
trap 'rm -rf -- "$STAGE"' EXIT
SDK="$STAGE/charlotte-app-sdk"
mkdir -p "$SDK/platform/aarch64" "$SDK/platform/x86_64" "$SDK/signer/tools" \
    "$SDK/signer/crates" "$SDK/keys"

install -m 0755 "$ROOT/scripts/build-external-elf.sh" "$SDK/build-external-elf.sh"

for arch in aarch64 x86_64; do
    install -m 0644 "$ROOT/crates/catten-services/$arch-unknown-none.json" \
        "$SDK/platform/$arch/target.json"
    install -m 0644 "$ROOT/crates/catten-services/link.x" \
        "$SDK/platform/$arch/link.x"
done

tar -C "$ROOT/tools" --exclude target --exclude Cargo.lock --exclude .DS_Store \
    -cf - cluster-sign | tar -C "$SDK/signer/tools" -xf -
tar -C "$ROOT/crates" --exclude target --exclude .DS_Store -cf - charlotte-launch \
    | tar -C "$SDK/signer/crates" -xf -

cat > "$SDK/signer/Cargo.toml" <<'EOF'
[workspace]
members = ["tools/cluster-sign", "crates/charlotte-launch"]
resolver = "3"
EOF

for key in dev-key.hex dev-operations-key.hex dev-operations-key.pub \
    dev-recipient-key.hex dev-recipient-key.pub; do
    if [ -f "$ROOT/tools/cluster-sign/$key" ]; then
        install -m 0644 "$ROOT/tools/cluster-sign/$key" "$SDK/keys/$key"
    fi
done

cat > "$SDK/VERSION" <<EOF
sdk_schema=1
os_revision=$REVISION
toolchain=$TOOLCHAIN
EOF

cat > "$SDK/README.md" <<'EOF'
# CharlotteOS application SDK

This bundle is exported from a CharlotteOS revision and contains the platform
inputs for out-of-tree application builds. It does not include the kernel or
any service binary.

- `build-external-elf.sh` builds a stripped, layout-verified EL0 ELF for an
  app's Cargo binary target. It generates a build-local target specification
  with an absolute path to `platform/<arch>/link.x` and runs cargo with the
  pinned toolchain, `-Z json-target-spec`, and `-Z build-std`.
- `platform/<arch>/` holds the target specification and linker script for
  `aarch64` and `x86_64`.
- `signer/` is a standalone Cargo workspace with `cluster-sign` and its launch
  ABI dependency. Build it with the pinned toolchain, then use `elf-sign`,
  `sha256`, `deployment-sign`, `deployment-notify`, and `deployment-status`.
- `keys/` holds the publicly known development keys only. Production keys are
  held offline and never belong in an SDK.
- `VERSION` records the SDK schema, the CharlotteOS revision, and the pinned
  toolchain.

Toolchain prerequisites: `rust-src` and `llvm-tools`.

```sh
rustup component add rust-src llvm-tools
rustup toolchain install "$(sed -n 's/^toolchain=//p' VERSION)"
(cd signer && cargo +"$(sed -n 's/^toolchain=//p' VERSION)" build)
./build-external-elf.sh --manifest /path/to/app/Cargo.toml --bin app --output /tmp/app.elf
```
EOF

rm -f "$OUTPUT"
tar -czf "$OUTPUT" -C "$STAGE" charlotte-app-sdk

if command -v sha256sum >/dev/null 2>&1; then
    DIGEST="$(sha256sum "$OUTPUT" | awk '{print $1}')"
else
    DIGEST="$(shasum -a 256 "$OUTPUT" | awk '{print $1}')"
fi
printf '%s  %s\n' "$DIGEST" "$(basename "$OUTPUT")" > "$OUTPUT.sha256"

echo ">>> Exported $OUTPUT"
echo ">>> sha256 $DIGEST"
