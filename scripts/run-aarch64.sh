#!/usr/bin/env bash
#
# Build a bootable UEFI disk image for CharlotteOS / Catten and run it under
# QEMU. Works on macOS (including Apple Silicon) and Linux.
#
# Requirements: qemu, mtools.  On macOS install via Homebrew:
#   brew install qemu mtools
# For HVF acceleration on Apple Silicon, use --hvf.
# For display (flanterm framebuffer console), use --display.
#
# Usage:
#   scripts/run-aarch64.sh [debug|release] [--display] [--gdb] [--hvf] [--smp N] [--timeout S]
#
#   debug|release  Build profile (default: debug)
#   --display      Build with framebuffer console (flanterm), boot with ramfb
#   --gdb          Start QEMU paused with gdb stub on tcp::1234
#   --hvf          Use Apple Hypervisor.Framework acceleration (macOS only)
#   --smp N        Number of CPUs (default: 4)
#   --timeout S    Kill QEMU after S seconds, capturing serial output (default: run interactively)
#
set -euo pipefail

ARCH="aarch64"
PROFILE="debug"
GDB=""
DISPLAY_MODE="0"
USE_HVF="0"
SMP="4"
TIMEOUT=""

while [ "$#" -gt 0 ]; do
    case "$1" in
        debug|release) PROFILE="$1"; shift ;;
        --display)     DISPLAY_MODE="1"; shift ;;
        --gdb)         GDB="-s -S"; shift ;;
        --hvf)         USE_HVF="1"; shift ;;
        --smp)
            [ "$#" -ge 2 ] || { echo "Missing value for --smp" >&2; exit 1; }
            SMP="$2"; shift 2 ;;
        --timeout)
            [ "$#" -ge 2 ] || { echo "Missing value for --timeout" >&2; exit 1; }
            TIMEOUT="$2"; shift 2 ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TARGET_SPEC="target_specs/${ARCH}-unknown-none-catten.json"
TARGET_DIR="${ARCH}-unknown-none-catten"
IMAGE_DIR="./os-images"
IMAGE="${IMAGE_DIR}/charlotte-${ARCH}-${PROFILE}.img"
KERNEL="./target/${TARGET_DIR}/${PROFILE}/catten"
EFI_BOOT_FILE="BOOTAA64.EFI"

# On macOS, firmware is under /opt/homebrew; on Linux it's under /usr/share.
if [ -f "/opt/homebrew/share/qemu/edk2-aarch64-code.fd" ]; then
    FIRMWARE="/opt/homebrew/share/qemu/edk2-aarch64-code.fd"
else
    FIRMWARE="/usr/share/AAVMF/AAVMF_CODE.fd"
fi

RELEASE_FLAG=""
if [ "$PROFILE" = "release" ]; then
    RELEASE_FLAG="--release"
fi

# Feature selection.
FEATURES="acpi"
BUILD_EXTRA=""
if [ "$DISPLAY_MODE" = "1" ]; then
    SYSROOT="$(rustc --print sysroot)"
    HOST_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"
    LLVM_AR="${SYSROOT}/lib/rustlib/${HOST_TRIPLE}/bin/llvm-ar"
    if [ ! -x "$LLVM_AR" ]; then
        echo "error: llvm-ar not found at ${LLVM_AR}" >&2
        echo "       run: rustup component add llvm-tools" >&2
        exit 1
    fi
    export AR_aarch64_unknown_none_catten="$LLVM_AR"
    FEATURES="acpi,display,virtio_gpu"
    echo ">>> Building Catten kernel (${ARCH}, ${PROFILE}, display)..."
else
    echo ">>> Building Catten kernel (${ARCH}, ${PROFILE}, headless)..."
fi

if [ "$USE_HVF" = "1" ]; then
    FEATURES="${FEATURES},hvf_compat"
fi

cargo build --package catten --target "$TARGET_SPEC" \
    --no-default-features --features "$FEATURES" $RELEASE_FLAG

# --- Build a FAT32 EFI System Partition image with mtools. ---
echo ">>> Creating boot image ${IMAGE}..."
mkdir -p "$IMAGE_DIR"
dd if=/dev/zero of="$IMAGE" bs=1m count=128 status=none
mformat -i "$IMAGE" -F ::
mmd -i "$IMAGE" ::/EFI
mmd -i "$IMAGE" ::/EFI/BOOT
mcopy -i "$IMAGE" "./limine-binary/${EFI_BOOT_FILE}" "::/EFI/BOOT/${EFI_BOOT_FILE}"
mcopy -i "$IMAGE" "$KERNEL" "::/catten"
mcopy -i "$IMAGE" "./limine.conf" "::/limine.conf"

# --- QEMU options ---
QEMU_OPTS=(
    -M virt,gic-version=3
    -m 512M
    -bios "$FIRMWARE"
    -drive file="$IMAGE",format=raw,if=virtio
)

if [ "$USE_HVF" = "1" ]; then
    QEMU_OPTS+=(-accel hvf -cpu host)
else
    QEMU_OPTS+=(-cpu cortex-a710)
fi

QEMU_OPTS+=(-smp "$SMP")

if [ "$DISPLAY_MODE" = "1" ]; then
    QEMU_OPTS+=(-device ramfb)
else
    QEMU_OPTS+=(-display none)
fi

if [ -n "$TIMEOUT" ]; then
    LOG="/tmp/charlotte-serial.log"
    QEMU_OPTS+=(-serial "file:${LOG}")
    echo ">>> Booting under QEMU (${TIMEOUT}s timeout, serial to ${LOG})..."
    qemu-system-aarch64 "${QEMU_OPTS[@]}" $GDB &
    QPID=$!
    sleep "$TIMEOUT"
    kill "$QPID" 2>/dev/null || true
    wait "$QPID" 2>/dev/null || true
    echo ">>> Serial log (${LOG}):"
    cat "$LOG"
    REQUIRED_MARKERS=(
        "[EL0] SUCCESS:"
        "[raft] SUCCESS:"
        "[EL0 IPC] SUCCESS:"
        "[EL0 IPC block] SUCCESS:"
        "[EL0 IPC cross-AS] SUCCESS:"
        "[EL0 IPC memory] SUCCESS:"
        "[EL0 IPC memory cancel] SUCCESS:"
        "[EL0 IPC memory copy] SUCCESS:"
        "[EL0 xLP] SUCCESS:"
        "[PP] SUCCESS:"
        "[sitas] SUCCESS:"
        "[service] SUCCESS:"
        "[cq wait] SUCCESS:"
        "[device] SUCCESS:"
        "[uart] SUCCESS:"
    )
    missing=0
    for marker in "${REQUIRED_MARKERS[@]}"; do
        if ! grep -Fq "$marker" "$LOG"; then
            echo "error: deferred self-test marker missing: ${marker}" >&2
            missing=1
        fi
    done
    if [ "$missing" -ne 0 ]; then
        exit 1
    fi
    echo ">>> All required deferred self-test markers observed."
else
    QEMU_OPTS+=(-serial stdio)
    if [ "$DISPLAY_MODE" = "1" ]; then
        echo ">>> Booting under QEMU (framebuffer window + serial; Ctrl-A X to quit)..."
    else
        echo ">>> Booting under QEMU (serial on stdio; press Ctrl-A X to quit)..."
    fi
    exec qemu-system-aarch64 "${QEMU_OPTS[@]}" $GDB
fi
