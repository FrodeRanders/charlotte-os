#!/bin/bash
# Boot x86_64 CharlotteOS in QEMU (TCG, single CPU, serial to file).
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="x86_64-unknown-none-catten"
PROFILE="debug"
IMAGE="/tmp/catten-x86.img"
KERNEL="$PROJECT_DIR/target/${TARGET}/${PROFILE}/catten"
FW="/opt/homebrew/share/qemu/edk2-x86_64-code.fd"

echo "=== Creating bootable FAT image ==="
dd if=/dev/zero of="$IMAGE" bs=1M count=64 status=none
mformat -i "$IMAGE" -F -v CATOS ::
mmd -i "$IMAGE" ::/EFI
mmd -i "$IMAGE" ::/EFI/BOOT
mcopy -i "$IMAGE" "$PROJECT_DIR/limine-binary/BOOTX64.EFI" ::/EFI/BOOT/BOOTX64.EFI
mcopy -i "$IMAGE" "$KERNEL" ::/catten
mcopy -i "$IMAGE" "$PROJECT_DIR/limine.conf" ::/limine.conf

echo "=== Booting QEMU x86_64 (TCG, smp=1, 90s) ==="
qemu-system-x86_64 \
    -M q35 \
    -cpu qemu64,+x2apic \
    -smp 1 \
    -m 512M \
    -drive "if=pflash,format=raw,unit=0,file=$FW,readonly=on" \
    -drive "if=none,file=$IMAGE,format=raw,id=drive0" \
    -device nvme,drive=drive0,serial=cat0 \
    -serial file:/tmp/catten-x86-serial.log \
    -display none \
    -no-reboot &
QPID=$!
sleep 90
kill $QPID 2>/dev/null || true
wait $QPID 2>/dev/null || true
cat /tmp/catten-x86-serial.log
