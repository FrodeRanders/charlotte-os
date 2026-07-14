#!/bin/bash
# QEMU AArch64 SMP-2 under explicit TCG for cross-core SGI test
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="aarch64-unknown-none-catten"
PROFILE="debug"
IMAGE="$PROJECT_DIR/os-images/charlotte-aarch64-smp2tcg.img"
KERNEL="$PROJECT_DIR/target/${TARGET}/${PROFILE}/catten"

echo "=== TCG cross-core SGI test ==="
echo "Host: $(uname -ms)"
echo "QEMU: $(qemu-system-aarch64 --version | head -1)"

dd if=/dev/zero of="$IMAGE" bs=1M count=64 status=none
mformat -i "$IMAGE" -F -v CATOS ::
mmd -i "$IMAGE" ::/EFI
mmd -i "$IMAGE" ::/EFI/BOOT
mcopy -i "$IMAGE" "$PROJECT_DIR/limine-binary/BOOTAA64.EFI" ::/EFI/BOOT/BOOTAA64.EFI
mcopy -i "$IMAGE" "$KERNEL" ::/catten
mcopy -i "$IMAGE" "$PROJECT_DIR/limine.conf" ::/limine.conf

echo "=== Booting with -accel tcg,thread=multi ==="
qemu-system-aarch64 \
    -M virt,gic-version=3 \
    -cpu cortex-a710 \
    -smp 2 \
    -m 512M \
    -bios /opt/homebrew/share/qemu/edk2-aarch64-code.fd \
    -drive "if=none,file=$IMAGE,format=raw,id=drive0" \
    -device virtio-blk-device,drive=drive0 \
    -serial file:/tmp/catten-smp2-tcg.log \
    -display none \
    -no-reboot \
    -accel tcg,thread=multi &
QPID=$!
sleep 90
kill $QPID 2>/dev/null || true
wait $QPID 2>/dev/null || true

echo "=== QEMU version info ==="
qemu-system-aarch64 --version 2>&1 | head -1
echo "=== TCG RESULT ==="
LC_ALL=C tr -d '\r' < /tmp/catten-smp2-tcg.log 2>/dev/null \
    | grep -aE "MPIDR|Testing Complete|xLP.*received|xLP.*timed|EL0 xLP.*SUCCESS|PP.*SUCCESS|sitas.*SUCCESS|panic|ABORT" \
    | head -40
