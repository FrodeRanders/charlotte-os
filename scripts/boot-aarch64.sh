#!/bin/bash
# Assemble a bootable FAT image + boot QEMU (assumes kernel is already built).
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="aarch64-unknown-none-catten"
PROFILE="debug"
IMAGE="$PROJECT_DIR/os-images/charlotte-aarch64-catten.img"
KERNEL="$PROJECT_DIR/target/${TARGET}/${PROFILE}/catten"

echo "=== Creating bootable FAT image at $IMAGE ==="
dd if=/dev/zero of="$IMAGE" bs=1M count=64 status=none
mformat -i "$IMAGE" -F -v CATOS ::
mmd -i "$IMAGE" ::/EFI
mmd -i "$IMAGE" ::/EFI/BOOT
mcopy -i "$IMAGE" "$PROJECT_DIR/limine-binary/BOOTAA64.EFI" ::/EFI/BOOT/BOOTAA64.EFI
mcopy -i "$IMAGE" "$KERNEL" ::/catten
mcopy -i "$IMAGE" "$PROJECT_DIR/limine.conf" ::/limine.conf

echo "=== Booting QEMU (serial output to terminal) ==="
echo "--- CharlotteOS boot output (self-tests run at boot) ---"
qemu-system-aarch64 \
    -M virt,gic-version=3 \
    -cpu cortex-a710 \
    -smp 4 \
    -m 512M \
    -bios /opt/homebrew/share/qemu/edk2-aarch64-code.fd \
    -drive "if=none,file=$IMAGE,format=raw,id=drive0" \
    -device virtio-blk-device,drive=drive0 \
    -serial stdio \
    -display none \
    -no-reboot
