#!/usr/bin/env bash
# Create CharlotteOS's disposable FAT32 UEFI boot image without mounting it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=lib/boot-common.sh
source "${SCRIPT_DIR}/lib/boot-common.sh"

ARCH="x86_64"
PROFILE="debug"
KERNEL=""
OUTPUT=""
SIZE_MIB=""
VOLUME_LABEL=""

usage() {
    cat >&2 <<'EOF'
usage: scripts/create-boot-image.sh [options]
  --arch ARCH       x86_64, aarch64, or riscv64 (default: x86_64)
  --profile PROFILE debug or release (default: debug)
  --kernel PATH     override the inferred kernel path
  --output PATH     override os-images/charlotte-ARCH-PROFILE.img
  --size-mib N      override the architecture's boot-image size
  --config PATH     override limine.conf (also: CATTEN_LIMINE_CONFIG)
EOF
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --arch)
            [ "$#" -ge 2 ] || { echo "Missing value for --arch" >&2; exit 1; }
            ARCH="$2"; shift 2 ;;
        --profile)
            [ "$#" -ge 2 ] || { echo "Missing value for --profile" >&2; exit 1; }
            PROFILE="$2"; shift 2 ;;
        --kernel)
            [ "$#" -ge 2 ] || { echo "Missing value for --kernel" >&2; exit 1; }
            KERNEL="$2"; shift 2 ;;
        --output)
            [ "$#" -ge 2 ] || { echo "Missing value for --output" >&2; exit 1; }
            OUTPUT="$2"; shift 2 ;;
        --size-mib)
            [ "$#" -ge 2 ] || { echo "Missing value for --size-mib" >&2; exit 1; }
            SIZE_MIB="$2"; shift 2 ;;
        --config)
            [ "$#" -ge 2 ] || { echo "Missing value for --config" >&2; exit 1; }
            CATTEN_LIMINE_CONFIG="$2"; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; usage; exit 1 ;;
    esac
done

case "$PROFILE" in
    debug|release) ;;
    *) echo "error: --profile must be 'debug' or 'release'" >&2; exit 1 ;;
esac

case "$ARCH" in
    x86_64)
        TARGET_DIR="x86_64-unknown-none-catten"
        EFI_BOOT_FILE="BOOTX64.EFI"
        DEFAULT_SIZE_MIB="64"
        VOLUME_LABEL="CATOS"
        ;;
    aarch64)
        TARGET_DIR="aarch64-unknown-none-catten"
        EFI_BOOT_FILE="BOOTAA64.EFI"
        DEFAULT_SIZE_MIB="128"
        ;;
    riscv64)
        TARGET_DIR="riscv64gc-unknown-none-catten"
        EFI_BOOT_FILE="BOOTRISCV64.EFI"
        DEFAULT_SIZE_MIB="128"
        ;;
    *) echo "error: unsupported architecture: ${ARCH}" >&2; exit 1 ;;
esac

catten_boot_init "$ROOT_DIR"
cd "$ROOT_DIR"

KERNEL="${KERNEL:-${ROOT_DIR}/target/${TARGET_DIR}/${PROFILE}/catten}"
OUTPUT="${OUTPUT:-${ROOT_DIR}/os-images/charlotte-${ARCH}-${PROFILE}.img}"
SIZE_MIB="${SIZE_MIB:-${DEFAULT_SIZE_MIB}}"

catten_boot_report_kernel "$KERNEL"
catten_boot_create_uefi_image \
    "$OUTPUT" \
    "$SIZE_MIB" \
    "${ROOT_DIR}/limine-binary/${EFI_BOOT_FILE}" \
    "$KERNEL" \
    "$CATTEN_BOOT_LIMINE_CONFIG" \
    "$VOLUME_LABEL"
echo ">>> Boot image ready: ${OUTPUT}"
