#!/usr/bin/env bash
#
# build-external-elf.sh -- build an out-of-tree CharlotteOS EL0 ELF.
#
# The platform definition (target specification, linker script, toolchain, and
# build-std invocation) is owned by CharlotteOS. This script is context
# sensitive:
#
#   * in the CharlotteOS repository it reads the platform files from
#     crates/catten-services;
#   * in an exported application SDK it reads platform/<arch>/ next to itself.
#
# It ends at a stripped, layout-verified ELF. Signing (tools/cluster-sign
# elf-sign) and deployment (deployment-sign/deployment-notify) are separate,
# explicit steps owned by the application packaging pipeline.
#
# Usage:
#   build-external-elf.sh --manifest PATH --bin NAME --output PATH
#       [--arch aarch64|x86_64] [--profile dev|release] [--target-dir DIR]
#       [--toolchain CHANNEL] [--features LIST] [--no-strip]
#
# Requirements: the pinned Rust toolchain with rust-src and llvm-tools, and
# python3 (used to generate the build-local target specification).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -d "$SCRIPT_DIR/platform" ]; then
    SDK_BUNDLE=1
    PLATFORM_ROOT="$SCRIPT_DIR"
else
    SDK_BUNDLE=0
    PLATFORM_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
fi

MANIFEST=""
BIN=""
OUTPUT=""
ARCH="aarch64"
PROFILE="release"
TARGET_DIR=""
TOOLCHAIN=""
FEATURES=""
STRIP=1

while [ "$#" -gt 0 ]; do
    case "$1" in
        --manifest) MANIFEST="$2"; shift 2 ;;
        --bin) BIN="$2"; shift 2 ;;
        --output) OUTPUT="$2"; shift 2 ;;
        --arch) ARCH="$2"; shift 2 ;;
        --profile) PROFILE="$2"; shift 2 ;;
        --target-dir) TARGET_DIR="$2"; shift 2 ;;
        --toolchain) TOOLCHAIN="$2"; shift 2 ;;
        --features) FEATURES="$2"; shift 2 ;;
        --no-strip) STRIP=0; shift ;;
        *) echo "error: unknown argument $1" >&2; exit 2 ;;
    esac
done

if [ -z "$MANIFEST" ] || [ -z "$BIN" ] || [ -z "$OUTPUT" ]; then
    echo "usage: $0 --manifest PATH --bin NAME --output PATH" >&2
    echo "          [--arch aarch64|x86_64] [--profile dev|release]" >&2
    exit 2
fi
case "$ARCH" in
    aarch64|x86_64) ;;
    *) echo "error: unsupported architecture $ARCH" >&2; exit 2 ;;
esac
case "$PROFILE" in
    dev|release) ;;
    *) echo "error: unsupported profile $PROFILE" >&2; exit 2 ;;
esac

if [ "$SDK_BUNDLE" = "1" ]; then
    BASE_JSON="$PLATFORM_ROOT/platform/$ARCH/target.json"
    LINK_X="$PLATFORM_ROOT/platform/$ARCH/link.x"
    if [ -z "$TOOLCHAIN" ] && [ -f "$PLATFORM_ROOT/VERSION" ]; then
        TOOLCHAIN="$(sed -n 's/^toolchain=//p' "$PLATFORM_ROOT/VERSION" | head -1)"
    fi
else
    BASE_JSON="$PLATFORM_ROOT/crates/catten-services/$ARCH-unknown-none.json"
    LINK_X="$PLATFORM_ROOT/crates/catten-services/link.x"
    if [ -z "$TOOLCHAIN" ] && [ -f "$PLATFORM_ROOT/rust-toolchain.toml" ]; then
        TOOLCHAIN="$(sed -n 's/^channel = "\(.*\)"/\1/p' "$PLATFORM_ROOT/rust-toolchain.toml" | head -1)"
    fi
fi
[ -f "$BASE_JSON" ] || { echo "error: missing target specification $BASE_JSON" >&2; exit 1; }
[ -f "$LINK_X" ] || { echo "error: missing linker script $LINK_X" >&2; exit 1; }

MANIFEST="$(cd "$(dirname "$MANIFEST")" && pwd)/$(basename "$MANIFEST")"
OUTPUT_DIR="$(dirname "$OUTPUT")"
mkdir -p "$OUTPUT_DIR"
OUTPUT="$(cd "$OUTPUT_DIR" && pwd)/$(basename "$OUTPUT")"
if [ -z "$TARGET_DIR" ]; then
    TARGET_DIR="$(dirname "$MANIFEST")/target/charlotte"
fi
mkdir -p "$TARGET_DIR"
TARGET_DIR="$(cd "$TARGET_DIR" && pwd)"

if ! command -v python3 >/dev/null 2>&1; then
    echo "error: python3 is required to generate the build-local target specification" >&2
    exit 1
fi

GENERATED_JSON="$TARGET_DIR/charlotte-$ARCH.json"
python3 - "$BASE_JSON" "$GENERATED_JSON" "$LINK_X" <<'PY'
import json
import sys

source, dest, link = sys.argv[1], sys.argv[2], sys.argv[3]
with open(source, encoding="utf-8") as handle:
    spec = json.load(handle)
spec.setdefault("pre-link-args", {})["gnu-lld"] = ["-T" + link]
with open(dest, "w", encoding="utf-8") as handle:
    json.dump(spec, handle, indent=2)
    handle.write("\n")
PY

CARGO=(cargo)
RUSTC=(rustc)
if [ -n "$TOOLCHAIN" ]; then
    if command -v rustup >/dev/null 2>&1; then
        CARGO=(cargo "+$TOOLCHAIN")
        RUSTC=(rustc "+$TOOLCHAIN")
    else
        echo "error: toolchain $TOOLCHAIN is required but rustup is not installed" >&2
        exit 1
    fi
fi

SYSROOT="$("${RUSTC[@]}" --print sysroot)"
HOST_TRIPLE="$("${RUSTC[@]}" -vV | awk '/^host:/ {print $2}')"
OBJCOPY="$SYSROOT/lib/rustlib/$HOST_TRIPLE/bin/llvm-objcopy"
READOBJ="$SYSROOT/lib/rustlib/$HOST_TRIPLE/bin/llvm-readobj"
if [ ! -x "$OBJCOPY" ] || [ ! -x "$READOBJ" ]; then
    echo "error: llvm-tools is required to build a CharlotteOS ELF" >&2
    echo "       run: rustup component add llvm-tools rust-src" >&2
    exit 1
fi
if [ ! -d "$SYSROOT/lib/rustlib/src/rust/library" ]; then
    echo "error: rust-src is required for -Z build-std" >&2
    echo "       run: rustup component add llvm-tools rust-src" >&2
    exit 1
fi

PROFILE_ARGS=()
PROFILE_DIR="release"
if [ "$PROFILE" = "dev" ]; then
    PROFILE_ARGS=()
    PROFILE_DIR="debug"
else
    PROFILE_ARGS=(--release)
fi
FEATURE_ARGS=()
if [ -n "$FEATURES" ]; then
    FEATURE_ARGS=(--features "$FEATURES")
fi

echo ">>> Building $BIN for $ARCH ($PROFILE)"
SMOLTCP_IFACE_MAX_ADDR_COUNT="${SMOLTCP_IFACE_MAX_ADDR_COUNT:-17}" \
"${CARGO[@]}" build \
    --manifest-path "$MANIFEST" \
    --bin "$BIN" \
    ${FEATURE_ARGS[@]+"${FEATURE_ARGS[@]}"} \
    --target "$GENERATED_JSON" \
    --target-dir "$TARGET_DIR" \
    ${PROFILE_ARGS[@]+"${PROFILE_ARGS[@]}"} \
    -Z json-target-spec \
    -Z build-std=core,alloc,compiler_builtins \
    -Z build-std-features=compiler-builtins-mem

BUILT="$TARGET_DIR/charlotte-$ARCH/$PROFILE_DIR/$BIN"
if [ ! -f "$BUILT" ]; then
    echo "error: expected built binary at $BUILT" >&2
    exit 1
fi

if [ "$STRIP" = "1" ]; then
    "$OBJCOPY" --strip-all "$BUILT" "$OUTPUT"
else
    install -m 0755 "$BUILT" "$OUTPUT"
fi

PAGE_SIZE=4096
declare -a LOAD_STARTS=()
declare -a LOAD_ENDS=()
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
    LOAD_COUNT=$((LOAD_COUNT + 1))
done < <("$READOBJ" --elf-output-style=GNU -l "$OUTPUT")

if [ "$LOAD_COUNT" -eq 0 ]; then
    echo "ERROR: ELF has no LOAD segments" >&2
    exit 1
fi

ENTRY="$("$READOBJ" -h "$OUTPUT" | awk '/Entry:/ {print $2}')"
SIZE="$(wc -c < "$OUTPUT")"
echo ">>> ELF: $OUTPUT ($SIZE bytes, $LOAD_COUNT LOAD segments, entry $ENTRY)"
echo ">>> Signing is a separate step: cluster-sign elf-sign <elf> <name> <key> ..."
