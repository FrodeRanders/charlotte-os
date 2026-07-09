#!/usr/bin/env bash
#
# Build a bootable UEFI disk image for CharlotteOS / Catten and run it under
# QEMU. This script is written to work on macOS (including Apple Silicon), where
# the Linux-oriented `just create-image` recipe (losetup/parted/mount) cannot
# run. It uses mtools to populate a FAT filesystem image without needing to
# mount anything or use sudo.
#
# Requirements (install via Homebrew): qemu, mtools.
#
# Usage:
#   scripts/run-aarch64.sh [debug|release] [--gdb]
#
set -euo pipefail

ARCH="aarch64"
PROFILE="${1:-debug}"
GDB=""
if [ "${2:-}" = "--gdb" ] || [ "${1:-}" = "--gdb" ]; then
    GDB="-s -S"
fi
if [ "${1:-}" = "--gdb" ]; then
    PROFILE="debug"
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TARGET_SPEC="target_specs/${ARCH}-unknown-none-catten.json"
TARGET_DIR="${ARCH}-unknown-none-catten"
IMAGE_DIR="./os-images"
IMAGE="${IMAGE_DIR}/charlotte-${ARCH}-${PROFILE}.img"
KERNEL="./target/${TARGET_DIR}/${PROFILE}/catten"
EFI_BOOT_FILE="BOOTAA64.EFI"
FIRMWARE="/opt/homebrew/share/qemu/edk2-aarch64-code.fd"

# --- Build the kernel (headless: no display feature, so the PL011 serial
#     console is the log sink; avoids the flanterm C dependency). ---
echo ">>> Building Catten kernel (${ARCH}, ${PROFILE})..."
RELEASE_FLAG=""
if [ "$PROFILE" = "release" ]; then
    RELEASE_FLAG="--release"
fi
cargo build --package catten --target "$TARGET_SPEC" --no-default-features --features acpi $RELEASE_FLAG

# --- Build a FAT32 EFI System Partition image with mtools. ---
echo ">>> Creating boot image ${IMAGE}..."
mkdir -p "$IMAGE_DIR"
# 128 MiB raw image.
dd if=/dev/zero of="$IMAGE" bs=1m count=128 status=none
# Format the whole image as FAT32 (no partition table needed; QEMU + edk2 will
# happily boot a FAT filesystem placed directly in the image).
mformat -i "$IMAGE" -F ::
mmd -i "$IMAGE" ::/EFI
mmd -i "$IMAGE" ::/EFI/BOOT
mcopy -i "$IMAGE" "./limine-binary/${EFI_BOOT_FILE}" "::/EFI/BOOT/${EFI_BOOT_FILE}"
mcopy -i "$IMAGE" "$KERNEL" "::/catten"
mcopy -i "$IMAGE" "./limine.conf" "::/limine.conf"

# --- Run under QEMU with the serial console on stdio. ---
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
