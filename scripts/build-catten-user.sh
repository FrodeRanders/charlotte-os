#!/usr/bin/env bash
#
# build-catten-user.sh — compile catten-user, extract the raw binary,
# update the embedded sitas-user.bin and the ENTRY_OFFSET constant, and
# (optionally) rebuild the kernel.
#
# Usage:
#   scripts/build-catten-user.sh [--embed]
#
#   --embed   Also copy the resulting .bin into the kernel's self_test/
#            directory and update ENTRY_OFFSET in el0_sitas.rs.
#
# Requirements: rustc +nightly with aarch64-unknown-none target,
#               llvm-objcopy (included with rustup component llvm-tools).
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MANIFEST="crates/catten-user/Cargo.toml"
TARGET_JSON="crates/catten-user/aarch64-unknown-none.json"
TARGET_DIR="crates/catten-user/target/aarch64-unknown-none/debug"
BIN_NAME="catten-user"
EMBED=0

for arg in "$@"; do
    case "$arg" in
        --embed) EMBED=1 ;;
        *) echo "Unknown argument: $arg" >&2; exit 1 ;;
    esac
done

echo ">>> Building $BIN_NAME ..."
cargo +nightly build --manifest-path "$MANIFEST" \
    --target "$TARGET_JSON" \
    -Z build-std=core,alloc 2>&1 | tail -3

echo ">>> Extracting raw binary ..."
SYSROOT="$(rustc +nightly --print sysroot)"
OBJCOPY="$SYSROOT/lib/rustlib/aarch64-apple-darwin/bin/llvm-objcopy"
"$OBJCOPY" -O binary "$TARGET_DIR/$BIN_NAME" /tmp/catten-user.bin

SIZE=$(wc -c < /tmp/catten-user.bin)
echo ">>> Binary size: $SIZE bytes"

if [ "$EMBED" -eq 1 ]; then
    DEST="crates/catten/src/self_test/sitas-user.bin"
    cp /tmp/catten-user.bin "$DEST"
    echo ">>> Copied binary to $DEST"

    # Read the ELF entry point and first LOAD VA to compute the correct
    # ENTRY_OFFSET (entry - first_LOAD_VA = offset from CODE_VADDR).
    ENTRY=$("$SYSROOT/lib/rustlib/aarch64-apple-darwin/bin/llvm-readobj" \
        -h "$TARGET_DIR/$BIN_NAME" | awk '/Entry:/ {print $2}')
    FIRST_LOAD_VA=$("$SYSROOT/lib/rustlib/aarch64-apple-darwin/bin/llvm-readobj" \
        -l "$TARGET_DIR/$BIN_NAME" | awk '/Type: PT_LOAD/{getline; getline; if(/VirtualAddress/){print $2; exit}}')
    if [ -n "$ENTRY" ] && [ -n "$FIRST_LOAD_VA" ]; then
        CORRECT_OFFSET=$((ENTRY - FIRST_LOAD_VA))
        SITAS_RS="crates/catten/src/self_test/el0_sitas.rs"
        OLD_OFFSET=$(grep "const ENTRY_OFFSET" "$SITAS_RS" | grep -o '0x[0-9a-fA-F]*')
        if [ "$OLD_OFFSET" != "0x${CORRECT_OFFSET#0x}" ]; then
            echo ">>> ENTRY_OFFSET mismatch: file has $OLD_OFFSET, computed $CORRECT_OFFSET"
            echo "    Update const ENTRY_OFFSET in $SITAS_RS manually, then rebuild the kernel."
        else
            echo ">>> ENTRY_OFFSET verified ($OLD_OFFSET)."
        fi
    fi

    echo ""
    echo ">>> Rebuilding the kernel ('cargo build -p catten --target ...') ..."
    cargo build --package catten \
        --target target_specs/aarch64-unknown-none-catten.json \
        --no-default-features --features acpi 2>&1 | tail -3
fi

echo ""
echo ">>> Done. Binary at /tmp/catten-user.bin ($SIZE bytes)."
