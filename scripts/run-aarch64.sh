#!/usr/bin/env bash
#
# Build a bootable UEFI disk image for CharlotteOS / Catten and run it under
# QEMU. This script is written to work on macOS (including Apple Silicon), where
# the Linux-oriented `just create-image` recipe (losetup/parted/mount) cannot
# run. It uses mtools to populate a FAT filesystem image without needing to
# mount anything or use sudo.
#
# Requirements (install via Homebrew): qemu, mtools.
# The `display` mode additionally needs the LLVM archiver, which ships with the
# rustup `llvm-tools` component:  rustup component add llvm-tools
#
# Usage:
#   scripts/run-aarch64.sh [debug|release] [--display] [--gdb]
#
#   --display   Build with the framebuffer console (flanterm) and boot with a
#               ramfb display in a QEMU window, instead of the default headless
#               serial console.
#   --gdb       Start QEMU paused with a gdb stub on tcp::1234.
#
set -euo pipefail

ARCH="aarch64"
PROFILE="debug"
GDB=""
DISPLAY_MODE="0"

for arg in "$@"; do
    case "$arg" in
        debug|release) PROFILE="$arg" ;;
        --display)     DISPLAY_MODE="1" ;;
        --gdb)         GDB="-s -S" ;;
        *) echo "Unknown argument: $arg" >&2; exit 1 ;;
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
FIRMWARE="/opt/homebrew/share/qemu/edk2-aarch64-code.fd"

RELEASE_FLAG=""
if [ "$PROFILE" = "release" ]; then
    RELEASE_FLAG="--release"
fi

# The C `flanterm` dependency (pulled in by the `display` feature) is compiled
# by the `cc` crate. On macOS the default archiver is Apple's `ar`/`ranlib`,
# which cannot build a valid archive from the ELF cross-compiled objects (it
# silently produces an empty Mach-O archive, so the flanterm symbols go
# missing at link time). We point the `cc` crate at the LLVM archiver from the
# active Rust toolchain, which is ELF-aware. This is only needed for the
# display build; the headless build has no C dependency.
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
    FEATURES="acpi"
    echo ">>> Building Catten kernel (${ARCH}, ${PROFILE}, headless)..."
fi

cargo build --package catten --target "$TARGET_SPEC" \
    --no-default-features --features "$FEATURES" $RELEASE_FLAG

# --- Build a FAT32 EFI System Partition image with mtools. ---
echo ">>> Creating boot image ${IMAGE}..."
mkdir -p "$IMAGE_DIR"
dd if=/dev/zero of="$IMAGE" bs=1m count=128 status=none
# Format the whole image as FAT (no partition table needed; QEMU + edk2 will
# happily boot a FAT filesystem placed directly in the image).
mformat -i "$IMAGE" -F ::
mmd -i "$IMAGE" ::/EFI
mmd -i "$IMAGE" ::/EFI/BOOT
mcopy -i "$IMAGE" "./limine-binary/${EFI_BOOT_FILE}" "::/EFI/BOOT/${EFI_BOOT_FILE}"
mcopy -i "$IMAGE" "$KERNEL" "::/catten"
mcopy -i "$IMAGE" "./limine.conf" "::/limine.conf"

# --- Run under QEMU. ---
if [ "$DISPLAY_MODE" = "1" ]; then
    # ramfb gives Limine a GOP framebuffer that the flanterm console draws to.
    # A real display backend (the default cocoa window on macOS) must be present
    # for the firmware to establish a GOP mode; with `-display none` the
    # framebuffer comes back with zero dimensions. Note that GOP provisioning on
    # QEMU aarch64 `virt` + edk2 can be flaky between boots; if no usable
    # framebuffer is provided the kernel automatically falls back to the serial
    # console, which stays attached here so logs are visible either way.
    echo ">>> Booting under QEMU (framebuffer window + serial; Ctrl-A X to quit)..."
    exec qemu-system-aarch64 \
        -M virt,gic-version=3 \
        -cpu cortex-a710 \
        -smp 4 \
        -m 512M \
        -bios "$FIRMWARE" \
        -drive file="$IMAGE",format=raw,if=virtio \
        -device ramfb \
        -serial stdio \
        $GDB
else
    echo ">>> Booting under QEMU (serial on stdio; press Ctrl-A X to quit)..."
    exec qemu-system-aarch64 \
        -M virt,gic-version=3 \
        -cpu cortex-a710 \
        -smp 4 \
        -m 512M \
        -bios "$FIRMWARE" \
        -drive file="$IMAGE",format=raw,if=virtio \
        -serial stdio \
        -display none \
        $GDB
fi
