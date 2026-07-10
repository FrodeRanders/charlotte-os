#!/bin/bash
# Quick boot script — single LP for self-tests, serial output only.
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="aarch64-unknown-none-catten"
PROFILE="debug"
IMAGE="$PROJECT_DIR/os-images/charlotte-aarch64-smp1.img"
KERNEL="$PROJECT_DIR/target/${TARGET}/${PROFILE}/catten"

echo "=== Creating bootable FAT image ==="
dd if=/dev/zero of="$IMAGE" bs=1M count=64 status=none
mformat -i "$IMAGE" -F -v CATOS ::
mmd -i "$IMAGE" ::/EFI
mmd -i "$IMAGE" ::/EFI/BOOT
mcopy -i "$IMAGE" "$PROJECT_DIR/limine-binary/BOOTAA64.EFI" ::/EFI/BOOT/BOOTAA64.EFI
mcopy -i "$IMAGE" "$KERNEL" ::/catten
mcopy -i "$IMAGE" "$PROJECT_DIR/limine.conf" ::/limine.conf

echo "=== Booting QEMU (smp=1, 60s timeout) ==="
qemu-system-aarch64 \
    -M virt,gic-version=3 \
    -cpu cortex-a710 \
    -smp 1 \
    -m 512M \
    -bios /opt/homebrew/share/qemu/edk2-aarch64-code.fd \
    -drive "if=none,file=$IMAGE,format=raw,id=drive0" \
    -device virtio-blk-device,drive=drive0 \
    -serial file:/tmp/charlotte-serial.log \
    -display none \
    -no-reboot &
QPID=$!
sleep 60
kill $QPID 2>/dev/null || true
wait $QPID 2>/dev/null || true
cat /tmp/charlotte-serial.log
