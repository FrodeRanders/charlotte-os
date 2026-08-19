#!/bin/bash
# Boot x86_64 CharlotteOS in QEMU (TCG, single CPU, serial to file).
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="x86_64-unknown-none-catten"
TARGET_SPEC="$PROJECT_DIR/target_specs/${TARGET}.json"
PROFILE="debug"
IMAGE="/tmp/catten-x86.img"
KERNEL="$PROJECT_DIR/target/${TARGET}/${PROFILE}/catten"

if command -v brew >/dev/null 2>&1; then
    QEMU_PREFIX="$(brew --prefix qemu 2>/dev/null || true)"
fi
QEMU_PREFIX="${QEMU_PREFIX:-}"
FW="${QEMU_PREFIX:+$QEMU_PREFIX/share/qemu/edk2-x86_64-code.fd}"
if [[ -z "$FW" || ! -f "$FW" ]]; then
    for candidate in \
        /opt/homebrew/share/qemu/edk2-x86_64-code.fd \
        /usr/local/share/qemu/edk2-x86_64-code.fd \
        /usr/share/qemu/edk2-x86_64-code.fd; do
        if [[ -f "$candidate" ]]; then
            FW="$candidate"
            break
        fi
    done
fi

make_fat_image() {
    if ! command -v mformat >/dev/null 2>&1 || \
       ! command -v mmd >/dev/null 2>&1 || \
       ! command -v mcopy >/dev/null 2>&1; then
        if [[ "$(uname -s)" == "Darwin" ]]; then
            echo "mtools is required; install it with: brew install mtools" >&2
        else
            echo "mtools is required (mformat, mmd, mcopy)" >&2
        fi
        return 1
    fi

    # Use a block size supported by both GNU and BSD dd.
    dd if=/dev/zero of="$IMAGE" bs=1048576 count=64 >/dev/null 2>&1
    mformat -i "$IMAGE" -F -v CATOS ::
    mmd -i "$IMAGE" ::/EFI
    mmd -i "$IMAGE" ::/EFI/BOOT
    mcopy -i "$IMAGE" "$PROJECT_DIR/limine-binary/BOOTX64.EFI" ::/EFI/BOOT/BOOTX64.EFI
    mcopy -i "$IMAGE" "$KERNEL" ::/catten
    mcopy -i "$IMAGE" "$PROJECT_DIR/limine.conf" ::/limine.conf
}

if [[ "${CATTEN_SKIP_KERNEL_BUILD:-0}" == "1" ]]; then
    if [[ ! -f "$KERNEL" ]]; then
        echo "CATTEN_SKIP_KERNEL_BUILD=1 but $KERNEL does not exist" >&2
        exit 1
    fi
else
    echo "=== Building x86_64 kernel ==="
    cargo build --package catten --target "$TARGET_SPEC"
fi

echo "=== Creating bootable FAT image ==="
make_fat_image

if [[ -z "$FW" || ! -f "$FW" ]]; then
    echo "QEMU EDK2 firmware not found; install qemu with Homebrew" >&2
    exit 1
fi

if [[ ! -x "$(command -v qemu-system-x86_64 || true)" ]]; then
    echo "qemu-system-x86_64 not found; install qemu with Homebrew" >&2
    exit 1
fi

echo "=== Booting QEMU x86_64 (TCG, smp=1, 90s) ==="
qemu-system-x86_64 \
    -M q35 \
    -cpu max \
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
