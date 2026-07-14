#!/usr/bin/env bash
#
# build-catten-user.sh — compile catten-user, strip the ELF for embedding, and
# (optionally) rebuild the kernel.
#
# Usage:
#   scripts/build-catten-user.sh [--embed]
#
#   --embed   Also copy the stripped ELF into the kernel's self_test/
#             directory.
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
PAGE_SIZE=4096

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

echo ">>> Stripping ELF for embedding ..."
SYSROOT="$(rustc +nightly --print sysroot)"
OBJCOPY="$SYSROOT/lib/rustlib/aarch64-apple-darwin/bin/llvm-objcopy"
READOBJ="$SYSROOT/lib/rustlib/aarch64-apple-darwin/bin/llvm-readobj"
"$OBJCOPY" --strip-all "$TARGET_DIR/$BIN_NAME" /tmp/catten-user.elf

SIZE=$(wc -c < /tmp/catten-user.elf)
echo ">>> Embedded ELF size: $SIZE bytes"

echo ">>> Validating ELF LOAD segments ..."
declare -a LOAD_STARTS=()
declare -a LOAD_ENDS=()
declare -a LOAD_FLAGS=()
LOAD_COUNT=0
while read -r type _offset virt _phys _filesz memsz f1 f2 _align _rest; do
    [ "$type" = "LOAD" ] || continue

    flags="$f1"
    if [[ "$f2" != 0x* ]]; then
        flags="${flags}${f2}"
    fi
    if [[ "$flags" == *W* && "$flags" == *E* ]]; then
        echo "ERROR: LOAD at $virt is writable and executable ($flags)" >&2
        exit 1
    fi

    start=$((virt))
    size=$((memsz))
    page_start=$((start & ~(PAGE_SIZE - 1)))
    page_end=$(((start + size + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1)))
    for ((i = 0; i < LOAD_COUNT; i++)); do
        if ((page_start < LOAD_ENDS[i] && page_end > LOAD_STARTS[i])); then
            printf 'ERROR: LOAD at %s overlaps prior LOAD within 4 KiB pages\n' "$virt" >&2
            exit 1
        fi
    done

    LOAD_STARTS[LOAD_COUNT]=$page_start
    LOAD_ENDS[LOAD_COUNT]=$page_end
    LOAD_FLAGS[LOAD_COUNT]=$flags
    LOAD_COUNT=$((LOAD_COUNT + 1))
done < <("$READOBJ" --elf-output-style=GNU -l /tmp/catten-user.elf)

if [ "$LOAD_COUNT" -eq 0 ]; then
    echo "ERROR: ELF has no LOAD segments" >&2
    exit 1
fi
echo ">>> LOAD segments verified ($LOAD_COUNT segments, no W+X or page overlap)."

if [ "$EMBED" -eq 1 ]; then
    DEST="crates/catten/src/self_test/sitas-user.elf"
    cp /tmp/catten-user.elf "$DEST"
    echo ">>> Copied ELF to $DEST"

    # Read the ELF entry point. The kernel ELF loader starts exactly there.
    ENTRY=$("$READOBJ" -h "$TARGET_DIR/$BIN_NAME" | awk '/Entry:/ {print $2}')
    if [ -n "$ENTRY" ]; then
        echo ">>> ELF entry verified ($ENTRY)."
    fi

    echo ""
    echo ">>> Rebuilding the kernel ('cargo build -p catten --target ...') ..."
    cargo build --package catten \
        --target target_specs/aarch64-unknown-none-catten.json \
        --no-default-features --features acpi 2>&1 | tail -3
fi

echo ""
echo ">>> Done. ELF at /tmp/catten-user.elf ($SIZE bytes)."
