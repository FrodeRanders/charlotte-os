#!/usr/bin/env bash
# Build the aarch64 kernel and boot it in QEMU on macOS.
# Usage: qemu-boot-macos.sh [timeout_s] [smp_count] [--headless]
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
SMP="${2:-2}"
HEADLESS=false
if [[ "${3:-}" == "--headless" ]]; then HEADLESS=true; fi

echo "== building kernel (hvf_compat) =="
cargo build --package catten --features hvf_compat --target target_specs/aarch64-unknown-none-catten.json

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

echo "== booting (${TIMEOUT}s cap, smp=${SMP}, headless=${HEADLESS}) =="
# HVF is required on QEMU 11.0.2 macOS — TCG serial output is broken.
QEMU_OPTS=(
  -M virt,gic-version=3 -accel hvf -cpu host -smp "$SMP" -m 512M
  -bios "$FW"
  -drive file="$IMG",format=raw,if=none,id=hd0
  -device virtio-blk-device,drive=hd0
)

if $HEADLESS; then
  QEMU_OPTS+=(-nographic -serial mon:stdio)
  qemu-system-aarch64 "${QEMU_OPTS[@]}" >"$LOG" 2>&1 &
  QPID=$!
  sleep "$TIMEOUT"
  kill "$QPID" 2>/dev/null || true
  wait "$QPID" 2>/dev/null || true
else
  # Graphical mode: keep the window open for interactive terminal use.
  # Serial output captured to log file for debugging.
  QEMU_OPTS+=(
    -device ramfb
    -device qemu-xhci,id=xhci -device usb-kbd,bus=xhci.0
    -serial file:"$LOG" -monitor none
  )
  echo "== graphical mode: QEMU window with flanterm. Press Ctrl-C to stop. =="
  echo "== serial log → $LOG =="
  qemu-system-aarch64 "${QEMU_OPTS[@]}" 2>/dev/null &
  QPID=$!
  # Don't kill — let the user interact with the terminal.
  wait "$QPID" 2>/dev/null || true
fi
echo "== log at $LOG =="
