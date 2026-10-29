#!/bin/bash
# Build the sbsa-ref firmware stack for QEMU TCG: TF-A (handing the
# bootloader/UEFI off at EL1) + edk2 QemuSbsa UEFI, packaged into
# SBSA_FLASH0.fd / SBSA_FLASH1.fd.
#
# Why this exists: QEMU TCG's EL2 handling is unreliable (E2H/VHE emulation is
# broken, and EL1 system-register writes from EL2 alias into the EL2 MMU), and
# the CharlotteOS kernel targets EL1/EL0. Limine base revision 6 enters a
# kernel at EL2+VHE when the boot firmware is at EL2, so the whole boot chain
# (TF-A -> UEFI -> Limine -> kernel) is made to run at EL1 instead.
#
# This script is the reproducible record of every workaround discovered while
# bringing sbsa-ref up. See docs/platforms/sbsa-ref.md for the full write-up.
#
# Requires (Homebrew on macOS):
#   aarch64-elf-gcc, aarch64-elf-binutils, autoconf, automake, acpica (iasl),
#   openssl@3, make (GNU Make 4.3+), gnu-sed, Python 3.11+. A cross-compiler for edk2's UEFI
#   is provided by aarch64-elf-gcc.
#
# Usage:
#   scripts/build-sbsa-firmware.sh [WORKDIR]
#   WORKDIR defaults to ./target/firmware-src. Outputs:
#   $WORKDIR/out/SBSA_FLASH{0,1}.fd

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKDIR="${1:-$ROOT_DIR/target/firmware-src}"
mkdir -p "$WORKDIR"
WORKDIR="$(cd "$WORKDIR" && pwd)"
OUT="$WORKDIR/out"
mkdir -p "$OUT"
LOG_DIR="$(mktemp -d "$OUT/build.XXXXXX")"
NCPU="$(sysctl -n hw.ncpu 2>/dev/null || echo 8)"

# edk2's VfrCompile uses grouped targets (&:). Make 3.81 accepts that syntax
# without enforcing the group, running generators concurrently over the same
# files. Check the feature, including recursive invocations from edk2.
MAKE_BIN=""
for candidate in gmake make; do
    command -v "$candidate" >/dev/null 2>&1 || continue
    features="$("$candidate" -s -f - <<'MAKE'
features:
	@echo $(.FEATURES)
MAKE
)"
    case " $features " in
        *" grouped-target "*) MAKE_BIN="$(command -v "$candidate")"; break ;;
    esac
done
if [ -z "$MAKE_BIN" ]; then
    echo "error: SBSA firmware requires GNU Make 4.3+ (grouped-target support)" >&2
    exit 1
fi
HOST_TOOLS="$WORKDIR/host-tools/bin"
mkdir -p "$HOST_TOOLS"
ln -sf "$MAKE_BIN" "$HOST_TOOLS/make"
if command -v gsed >/dev/null 2>&1; then
    ln -sf "$(command -v gsed)" "$HOST_TOOLS/sed"
fi
export PATH="$HOST_TOOLS:$PATH"

echo ">>> Working directory: $WORKDIR"
echo ">>> GNU Make: $MAKE_BIN"
echo ">>> Build logs: $LOG_DIR"

clone() {
    local repo="$1" dir="$2" revision="$3"
    if [ ! -d "$dir" ]; then
        git init "$dir"
        git -C "$dir" remote add origin "$repo"
        git -C "$dir" fetch --depth 1 origin "$revision"
        git -C "$dir" checkout --detach FETCH_HEAD
    else
        echo "    (reusing $dir)"
    fi
    if [ "$(git -C "$dir" rev-parse HEAD)" != "$revision" ]; then
        echo "error: $dir must be at pinned revision $revision; existing checkout left intact" >&2
        exit 1
    fi
}

echo ">>> Cloning sources"
# Keep all three TF-A stages on v2.11, including our EL1/GIC adaptations.
# Pin the edk2 family too: floating default branches made the old build recipe
# depend on whichever BaseTools and platform revisions happened to be current.
clone https://github.com/ARM-software/arm-trusted-firmware.git "$WORKDIR/tf-a" f2735ebccf5173f74c0458736ec526276106097e
clone https://github.com/tianocore/edk2.git "$WORKDIR/edk2" 58206682fe3212b80f662f6a62c375cce85d595e
clone https://github.com/tianocore/edk2-platforms.git "$WORKDIR/edk2-platforms" a46be4f60a0dd7b6aae0f46e6237a398a5f4151a
clone https://github.com/tianocore/edk2-non-osi.git "$WORKDIR/edk2-non-osi" 7ac12d81e02b323bffdf1ef3c188ea33c2185c91

# =============================================================================
# Third-party patches (tracked patch files under patches/, applied with git
# apply; every repo below is a git checkout). Idempotent: each application is
# guarded by a marker grep so re-runs skip already-patched trees.
# =============================================================================

apply_patch() {
    local repo="$1" patch="$2" marker="$3" file="$4" what="$5"
    if ! grep -q "$marker" "$repo/$file"; then
        echo ">>> $what"
        # edk2 keeps CRLF sources; the tracked patch has LF context lines.
        git -C "$repo" apply --ignore-space-change "$patch"
    else
        echo "    ($(basename "$patch") already applied)"
    fi
}

PATCHES="$ROOT_DIR/patches"

# (1) Hand BL33 (UEFI/Limine) off at EL1 instead of EL2 (QEMU TCG EL2 is
#     unreliable and the kernel targets EL1/EL0).
apply_patch "$WORKDIR/tf-a" \
    "$PATCHES/tf-a/0002-sbsa-bl33-entry-el1.patch" \
    "CharlotteOS" "plat/qemu/common/qemu_bl2_setup.c" \
    "TF-A: forcing BL33 entry at EL1"

# (2) SIP_SVC_GET_CPU_TOPOLOGY (SMC 202), required by the current edk2
#     QemuSbsa HardwareInfoLib but missing from TF-A v2.11; without it the
#     UEFI loops calling ResetShutdown().
apply_patch "$WORKDIR/tf-a" \
    "$PATCHES/tf-a/0003-sbsa-sip-smc-cpu-topology.patch" \
    "GET_CPU_TOPOLOGY" "plat/qemu/qemu_sbsa/sbsa_sip_svc.c" \
    "TF-A: adding SIP_SVC_GET_CPU_TOPOLOGY (SMC 202)"

# (3) Disable GIC security (GICD_CTLR.DS=1) in BL31. The whole boot chain runs
#     at Non-secure EL1, and with DS=0 QEMU drops NS writes to GICD_IGROUPR and
#     reports LPIs as Group 0, which silently breaks SPI + MSI delivery to the
#     kernel. See docs/platforms/sbsa-ref.md "GIC security".
apply_patch "$WORKDIR/tf-a" \
    "$PATCHES/tf-a/0001-sbsa-gic-disable-security-ds.patch" \
    "CTLR_DS_BIT" "plat/qemu/qemu_sbsa/sbsa_gic.c" \
    "TF-A: disabling GIC security (DS=1) in plat_qemu_gic_init"

# =============================================================================
# Build TF-A + fiptool + FIP
# =============================================================================
echo ">>> Building TF-A (qemu_sbsa)"
# Build the firmware first; the top-level `fip` target rebuilds its host tool
# without the macOS flags below. Pack the FIP explicitly after that tool build.
make -C "$WORKDIR/tf-a" PLAT=qemu_sbsa CROSS_COMPILE=aarch64-elf- \
    aarch64-oc=aarch64-elf-objcopy DEBUG=0 all -j"$NCPU" > "$LOG_DIR/tfa-build.log" 2>&1

echo ">>> Building fiptool"
OPENSSL_DIR="$(brew --prefix openssl@3 2>/dev/null || echo /usr)"
make -C "$WORKDIR/tf-a/tools/fiptool" OPENSSL_DIR="$OPENSSL_DIR" \
    CPPFLAGS="-D_GNU_SOURCE -D_DARWIN_C_SOURCE -D_UUID_T" > "$LOG_DIR/fiptool-build.log" 2>&1
FIPTOOL="$WORKDIR/tf-a/tools/fiptool/fiptool"

# BL1 is a separate flash payload, not a member of the FIP. Build it with the
# same revision and compiler as BL2/BL31 instead of silently mixing upstream
# binaries or falling back after a failed unpack. This triplet boots under TCG.
echo ">>> Staging matching BL1 and packing BL2/BL31 FIP"
TFA="$WORKDIR/tf-a/build/qemu_sbsa/release"
cp "$TFA/bl1.bin" "$OUT/bl1.bin"
"$FIPTOOL" create --tb-fw "$TFA/bl2.bin" --soc-fw "$TFA/bl31.bin" "$OUT/fip.bin" > /dev/null 2>&1

# =============================================================================
# Patch the edk2 build tool
# =============================================================================
# edk2's build.py only converts PlatformFile to a PathClass when no -p was
# given; the SbsaQemu build passes -p, so convert the plain string there too.
apply_patch "$WORKDIR/edk2" \
    "$PATCHES/edk2/0001-build-py-pathclass-p.patch" \
    "isinstance(self.PlatformFile" "BaseTools/Source/Python/build/build.py" \
    "edk2: patching build tool (-p PathClass conversion)"

# =============================================================================
# Build edk2 QemuSbsa UEFI
# =============================================================================
echo ">>> Building edk2 QemuSbsa (this takes a while)"
cd "$WORKDIR/edk2"
git submodule update --init --depth 1 > "$LOG_DIR/edk2-submodules.log" 2>&1
make -C BaseTools clean > "$LOG_DIR/edk2-clean.log" 2>&1
make -C BaseTools -j"$NCPU" > "$LOG_DIR/edk2-basetools.log" 2>&1

# Point the flash composition at our TF-A images.
cp "$OUT/fip.bin" "$WORKDIR/edk2-non-osi/Platform/Qemu/Sbsa/fip.bin"
cp "$OUT/bl1.bin" "$WORKDIR/edk2-non-osi/Platform/Qemu/Sbsa/bl1.bin"

# Workspace: edk2-platforms and edk2-non-osi must be real subdirectories of
# WORKSPACE; edk2's own Conf is symlinked so $WORKSPACE/Conf resolves.
ln -sfn "$WORKDIR/edk2/Conf" "$WORKDIR/Conf"
for config in target tools_def build_rule; do
    if [ ! -f "$WORKDIR/edk2/Conf/$config.txt" ]; then
        cp "$WORKDIR/edk2/BaseTools/Conf/$config.template" "$WORKDIR/edk2/Conf/$config.txt"
    fi
done

export WORKSPACE="$WORKDIR"
export PACKAGES_PATH="$WORKDIR/edk2:$WORKDIR/edk2-platforms:$WORKDIR/edk2-non-osi"
export EDK_TOOLS_PATH="$WORKDIR/edk2/BaseTools"
export PYTHONPATH="$WORKDIR/edk2/BaseTools/Source/Python"
export PATH="$WORKDIR/edk2/BaseTools/BinWrappers/PosixLike:/opt/homebrew/bin:$PATH"
export GCC_AARCH64_PREFIX=aarch64-elf-

python3 "$WORKDIR/edk2/BaseTools/Source/Python/build/build.py" \
    -p edk2-platforms/Platform/Qemu/SbsaQemu/SbsaQemu.dsc \
    -b RELEASE -a AARCH64 -t GCC -n "$NCPU" > "$LOG_DIR/edk2-build.log" 2>&1

# =============================================================================
# Package the flash images
# =============================================================================
echo ">>> Packaging SBSA_FLASH0.fd / SBSA_FLASH1.fd"
cp "$WORKDIR/Build/SbsaQemu/RELEASE_GCC/FV/SBSA_FLASH0.fd" "$OUT/SBSA_FLASH0.fd"
cp "$WORKDIR/Build/SbsaQemu/RELEASE_GCC/FV/SBSA_FLASH1.fd" "$OUT/SBSA_FLASH1.fd"
truncate -s 256M "$OUT/SBSA_FLASH0.fd"
truncate -s 256M "$OUT/SBSA_FLASH1.fd"

python3 - "$WORKDIR" "$ROOT_DIR/patches" "$MAKE_BIN" <<'PY'
import hashlib
import json
from pathlib import Path
import subprocess
import sys

work, patches = map(Path, sys.argv[1:3])
def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()
provenance = {
    'sources': {name: subprocess.check_output(
        ['git', '-C', str(work / name), 'rev-parse', 'HEAD'], text=True).strip()
        for name in ('tf-a', 'edk2', 'edk2-platforms', 'edk2-non-osi')},
    'patches_sha256': {str(p.relative_to(patches)): digest(p)
        for family in ('tf-a', 'edk2') for p in sorted((patches / family).glob('*.patch'))},
    'make': subprocess.check_output([sys.argv[3], '--version'], text=True).splitlines()[0],
    'compiler': subprocess.check_output(['aarch64-elf-gcc', '--version'], text=True).splitlines()[0],
    'artifacts_sha256': {name: digest(work / 'out' / name)
        for name in ('SBSA_FLASH0.fd', 'SBSA_FLASH1.fd', 'bl1.bin', 'fip.bin')},
}
(work / 'out' / 'provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')
PY

echo
echo ">>> Done. Outputs:"
echo "    $OUT/SBSA_FLASH0.fd  (TF-A: EL1 handoff + CPU-topology SMC)"
echo "    $OUT/SBSA_FLASH1.fd  (edk2 QemuSbsa UEFI)"
echo
echo ">>> To run:"
echo "    CATTEN_SBSA_FLASH0='$OUT/SBSA_FLASH0.fd' \\"
echo "    CATTEN_SBSA_FLASH1='$OUT/SBSA_FLASH1.fd' \\"
echo "      '$ROOT_DIR/scripts/run-aarch64.sh' --sbsa-ref --no-network --timeout 240"
