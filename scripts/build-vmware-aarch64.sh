#!/usr/bin/env bash
# Build an experimental AArch64 VMware Fusion appliance for Apple silicon.
# The boot disk contains BOOTAA64.EFI and a framebuffer-enabled kernel; the
# separate blank NVMe disk is reserved for first-boot object-store installation.
# Packaging is supported; Fusion ARM kernel/device bring-up is not qualified.
#
# Usage:
#   scripts/build-vmware-aarch64.sh [debug|release] [--minimal|--tests]
#                                 [--data-size-mib N] [--clean] [--replace]
set -euo pipefail

PROFILE="release"
BOOT_MODE="test"
DATA_SIZE_MIB="1024"
CLEAN="0"
REPLACE="0"

usage() {
    sed -n '/^# Usage:/,/^set -euo pipefail/p' "$0" | sed '$d; s/^# *//'
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        debug|release) PROFILE="$1"; shift ;;
        --minimal) BOOT_MODE="minimal"; shift ;;
        --tests) BOOT_MODE="test"; shift ;;
        --data-size-mib)
            [ "$#" -ge 2 ] || { echo "Missing value for --data-size-mib" >&2; exit 1; }
            DATA_SIZE_MIB="$2"; shift 2 ;;
        --clean) CLEAN="1"; shift ;;
        --replace) REPLACE="1"; shift ;;
        --help|-h) usage; exit 0 ;;
        *) echo "Unknown argument: $1" >&2; usage >&2; exit 1 ;;
    esac
done

if ! [[ "$DATA_SIZE_MIB" =~ ^[0-9]+$ ]] || [ "$DATA_SIZE_MIB" -lt 64 ]; then
    echo "error: --data-size-mib must be an integer of at least 64" >&2
    exit 1
fi
if ! command -v qemu-img >/dev/null 2>&1; then
    echo "error: qemu-img is required (on macOS: brew install qemu mtools)" >&2
    exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

APPLIANCE_PARENT="${ROOT_DIR}/os-images/vmware"
APPLIANCE_DIR="${APPLIANCE_PARENT}/CharlotteOS-AArch64.vmwarevm"
INSTANCE="vmware-aarch64"
if [ "$BOOT_MODE" = "minimal" ]; then
    APPLIANCE_DIR="${APPLIANCE_PARENT}/CharlotteOS-AArch64-Minimal.vmwarevm"
    INSTANCE="${INSTANCE}-minimal"
fi

check_destination() {
    if [ -L "$APPLIANCE_DIR" ]; then
        echo "error: refusing a symlink appliance destination: $APPLIANCE_DIR" >&2
        exit 1
    fi
    if [ -e "$APPLIANCE_DIR" ] && [ "$REPLACE" != "1" ]; then
        echo "error: VMware appliance already exists: $APPLIANCE_DIR" >&2
        echo "       move it aside, or pass --replace to discard its persistent data disk" >&2
        exit 1
    fi
    for lock in "$APPLIANCE_DIR"/*.lck; do
        if [ -e "$lock" ]; then
            echo "error: appliance is locked; shut down and close it in Fusion: $lock" >&2
            exit 1
        fi
    done
}
check_destination

BUILD_ARGS=("$PROFILE" --build-only --display --fresh-storage
            --instance vmware-aarch64)
if [ "$BOOT_MODE" = "minimal" ]; then
    BUILD_ARGS+=(--minimal)
else
    BUILD_ARGS+=(--tests)
fi
if [ "$CLEAN" = "1" ]; then BUILD_ARGS+=(--clean); fi
"${ROOT_DIR}/scripts/run-aarch64.sh" "${BUILD_ARGS[@]}"

# Prepare the complete replacement before discarding an existing appliance.
# The runner's preseeded QEMU data disk is intentionally not packaged: this
# appliance uses the ordinary blank-disk installation path, like the x86 VM.
mkdir -p "$APPLIANCE_PARENT"
STAGING="$(mktemp -d "${APPLIANCE_PARENT}/.aarch64-appliance.XXXXXX")"
trap 'rm -rf -- "$STAGING"' EXIT
RAW_BOOT="${ROOT_DIR}/os-images/charlotte-aarch64-${PROFILE}-${INSTANCE}.img"
VMDK_OPTIONS="subformat=monolithicSparse,compat6,adapter_type=lsilogic"
qemu-img convert -f raw -O vmdk -o "$VMDK_OPTIONS" \
    "$RAW_BOOT" "${STAGING}/charlotte-boot.vmdk"
qemu-img create -f vmdk -o "$VMDK_OPTIONS" \
    "${STAGING}/charlotte-data.vmdk" "${DATA_SIZE_MIB}M"
cp "${ROOT_DIR}/vmware/charlotte-os-aarch64.vmx" "${STAGING}/CharlotteOS.vmx"
if [ -d "${RAW_BOOT}.management" ]; then
    cp -R "${RAW_BOOT}.management" "${STAGING}/management"
fi

check_destination
if [ -e "$APPLIANCE_DIR" ]; then
    # --replace explicitly discards the fixed destination's data and NVRAM.
    rm -rf -- "$APPLIANCE_DIR"
fi
mv "$STAGING" "$APPLIANCE_DIR"
trap - EXIT

echo ">>> AArch64 VMware appliance built: ${APPLIANCE_DIR}/CharlotteOS.vmx"
echo ">>> Open the VMX in VMware Fusion on Apple silicon."
echo ">>> Experimental platform: Fusion ARM boot, UART and protected DMA are unqualified."
echo ">>> Networking is disabled until a Fusion ARM NIC driver is supported."
echo ">>> See docs/platforms/vmware-aarch64.md for bring-up limits."
