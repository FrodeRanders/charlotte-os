#!/usr/bin/env bash
# Build the aarch64 kernel and boot it headless in QEMU on macOS, capturing
# serial output. Uses macOS disk tooling (hdiutil/diskutil) in place of
# losetup/parted/mkfs.fat. Requires the rustup llvm-ar for the flanterm C lib.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

LLVM_BIN="$(rustc --print sysroot)/lib/rustlib/aarch64-apple-darwin/bin"
export AR_aarch64_unknown_none_catten="$LLVM_BIN/llvm-ar"
export RANLIB_aarch64_unknown_none_catten="$LLVM_BIN/llvm-ranlib"

KDIR="/var/folders/ld/gmfpcx6n12j8w922_4_sbcvc0000gn/T/opencode/catten-img"
mkdir -p "$KDIR"
IMG="$KDIR/charlotte-aarch64.img"
LOG="$KDIR/boot.log"
FW=/opt/homebrew/share/qemu/edk2-aarch64-code.fd
TIMEOUT="${1:-60}"

echo "== building kernel =="
cargo build --package catten --target target_specs/aarch64-unknown-none-catten.json

echo "== creating $IMG =="
rm -f "$IMG"
dd if=/dev/zero of="$IMG" bs=1m count=256 2>/dev/null
DEV="$(hdiutil attach -nomount -imagekey diskimage-class=CRawDiskImage "$IMG" | head -1 | awk '{print $1}')"
diskutil partitionDisk "$DEV" GPT "MS-DOS FAT32" CATTEN 100% >/dev/null
VOL=/Volumes/CATTEN
mkdir -p "$VOL/EFI/BOOT"
cp limine-binary/BOOTAA64.EFI "$VOL/EFI/BOOT/BOOTAA64.EFI"
cp target/aarch64-unknown-none-catten/debug/catten "$VOL/catten"
cp limine.conf "$VOL/limine.conf"
sync
diskutil unmount "$VOL" >/dev/null
hdiutil detach "$DEV" >/dev/null

echo "== booting (${TIMEOUT}s cap) =="
qemu-system-aarch64 \
  -M virt,gic-version=3 -cpu cortex-a710 -smp 8 -m 512M \
  -bios "$FW" \
  -drive file="$IMG",format=raw,if=none,id=hd0 \
  -device virtio-blk-device,drive=hd0 \
  -nographic -serial mon:stdio >"$LOG" 2>&1 &
QPID=$!
sleep "$TIMEOUT"
kill "$QPID" 2>/dev/null || true
wait "$QPID" 2>/dev/null || true
echo "== log at $LOG =="
