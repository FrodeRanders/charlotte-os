#!/usr/bin/env bash
# Build a two-disk CharlotteOS VMware appliance.
#
# The boot VMDK contains the FAT32 Limine/kernel image. The persistent NVMe
# VMDK starts blank: objstore formats it on first boot and the kernel seeds
# missing signed service artifacts from its immutable bootstrap bundle.
#
# Usage:
#   scripts/build-vmware-x86_64.sh [debug|release] [--data-size-mib N]
#                                      [--clean] [--replace]
set -euo pipefail

PROFILE="release"
DATA_SIZE_MIB="1024"
CLEAN="0"
REPLACE="0"

while [ "$#" -gt 0 ]; do
    case "$1" in
        debug|release) PROFILE="$1"; shift ;;
        --data-size-mib)
            [ "$#" -ge 2 ] || { echo "Missing value for --data-size-mib" >&2; exit 1; }
            DATA_SIZE_MIB="$2"; shift 2 ;;
        --clean) CLEAN="1"; shift ;;
        --replace) REPLACE="1"; shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if ! [[ "$DATA_SIZE_MIB" =~ ^[0-9]+$ ]] || [ "$DATA_SIZE_MIB" -lt 64 ]; then
    echo "error: --data-size-mib must be an integer of at least 64" >&2
    exit 1
fi
if ! command -v qemu-img >/dev/null 2>&1; then
    echo "error: qemu-img is required to convert raw images to VMDK" >&2
    exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

APPLIANCE_DIR="${ROOT_DIR}/os-images/vmware/CharlotteOS.vmwarevm"
if [ -e "$APPLIANCE_DIR" ] && [ "$REPLACE" != "1" ]; then
    echo "error: VMware appliance already exists: $APPLIANCE_DIR" >&2
    echo "       move it aside, or pass --replace to discard its persistent data disk" >&2
    exit 1
fi

BUILD_ARGS=(
    "$PROFILE"
    --build-only
    --instance vmware
    --blank-storage
    --net-test
    --nic e1000e
    --data-size-mib "$DATA_SIZE_MIB"
    --fresh-storage
)
if [ "$CLEAN" = "1" ]; then
    BUILD_ARGS+=(--clean)
fi
"${ROOT_DIR}/scripts/run-x86_64.sh" "${BUILD_ARGS[@]}"

RAW_BOOT="${ROOT_DIR}/os-images/charlotte-x86_64-${PROFILE}-vmware.img"
RAW_DATA="${ROOT_DIR}/os-images/x86-data-vmware.img"
BOOT_VMDK="${APPLIANCE_DIR}/charlotte-boot.vmdk"
DATA_VMDK="${APPLIANCE_DIR}/charlotte-data.vmdk"

if [ -e "$APPLIANCE_DIR" ]; then
    # The target is fixed above and --replace is explicit. Removing the whole
    # bundle also drops VMware's NVRAM, redo logs, and lock metadata, all of
    # which must not leak into a newly generated appliance.
    rm -rf "$APPLIANCE_DIR"
fi
mkdir -p "$APPLIANCE_DIR"
qemu-img convert -f raw -O vmdk \
    -o subformat=monolithicSparse,compat6,adapter_type=lsilogic \
    "$RAW_BOOT" "$BOOT_VMDK"
qemu-img convert -f raw -O vmdk \
    -o subformat=monolithicSparse,compat6,adapter_type=lsilogic \
    "$RAW_DATA" "$DATA_VMDK"
cp "${ROOT_DIR}/vmware/charlotte-os.vmx" "${APPLIANCE_DIR}/CharlotteOS.vmx"

echo ">>> VMware appliance complete:"
echo "    ${APPLIANCE_DIR}/CharlotteOS.vmx"
echo ">>> Open that VMX in VMware Fusion/Workstation, or import/convert the"
echo "    VMDKs for ESXi. Serial output will be written beside the VMX."
echo ">>> The appliance includes an E1000E adapter on VMware NAT."
