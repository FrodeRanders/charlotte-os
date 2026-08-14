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
#   openssl@3. A cross-compiler for edk2's UEFI is provided by aarch64-elf-gcc.
#
# Usage:
#   scripts/build-sbsa-firmware.sh [WORKDIR]
#   WORKDIR defaults to ./target/firmware-src. Outputs:
#   $WORKDIR/out/SBSA_FLASH{0,1}.fd

set -euo pipefail

WORKDIR="${1:-$PWD/target/firmware-src}"
OUT="$WORKDIR/out"
mkdir -p "$OUT"
NCPU="$(sysctl -n hw.ncpu 2>/dev/null || echo 8)"

echo ">>> Working directory: $WORKDIR"

clone() {
    local repo="$1" dir="$2" branch="${3:-}"
    if [ ! -d "$dir" ]; then
        if [ -n "$branch" ]; then
            git clone --depth 1 --branch "$branch" "$repo" "$dir"
        else
            git clone --depth 1 "$repo" "$dir"
        fi
    else
        echo "    (reusing $dir)"
    fi
}

echo ">>> Cloning sources"
# TF-A v2.11: the sbsa-ref images are built from the v2.11 tag (the prebuilt
# upstream BL1 in edk2-non-osi is v2.11.0-774, and later TF-A moved the SRAM
# layout so a v2.11 BL1 cannot load a master BL2).
clone https://github.com/ARM-software/arm-trusted-firmware.git "$WORKDIR/tf-a" v2.11
clone https://github.com/tianocore/edk2.git "$WORKDIR/edk2"
clone https://github.com/tianocore/edk2-platforms.git "$WORKDIR/edk2-platforms"
clone https://github.com/tianocore/edk2-non-osi.git "$WORKDIR/edk2-non-osi"

# =============================================================================
# Third-party patches (tracked patch files under patches/, applied with git
# apply; every repo below is a git checkout). Idempotent: each application is
# guarded by a marker grep so re-runs skip already-patched trees.
# =============================================================================

apply_patch() {
    local repo="$1" patch="$2" marker="$3" file="$4" what="$5"
    if ! grep -q "$marker" "$repo/$file"; then
        echo ">>> $what"
        git -C "$repo" apply "$patch"
    else
        echo "    ($(basename "$patch") already applied)"
    fi
}

PATCHES="$PWD/patches"

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
make -C "$WORKDIR/tf-a" PLAT=qemu_sbsa CROSS_COMPILE=aarch64-elf- \
    aarch64-oc=aarch64-elf-objcopy DEBUG=0 all fip -j"$NCPU" > "$OUT/tfa-build.log" 2>&1

echo ">>> Building fiptool"
OPENSSL_DIR="$(brew --prefix openssl@3 2>/dev/null || echo /usr)"
make -C "$WORKDIR/tf-a/tools/fiptool" OPENSSL_DIR="$OPENSSL_DIR" \
    CPPFLAGS="-D_GNU_SOURCE -D_DARWIN_C_SOURCE -D_UUID_T" > "$OUT/fiptool-build.log" 2>&1
FIPTOOL="$WORKDIR/tf-a/tools/fiptool/fiptool"

# The freshly built BL1 panics when built with aarch64-elf-gcc (a toolchain
# quirk); the upstream BL1 works, so we reuse it from the upstream FIP.
echo ">>> Extracting upstream BL1 and packing FIP (orig BL1 + rebuilt BL2/BL31)"
TFA="$WORKDIR/tf-a/build/qemu_sbsa/release"
UPSTREAM_FIP="$WORKDIR/edk2-non-osi/Platform/Qemu/Sbsa/fip.bin"
# The upstream prebuilt FIP in edk2-non-osi is a full FIP; unpack to get BL1.
rm -rf "$WORKDIR/fiptool-unpack" && mkdir -p "$WORKDIR/fiptool-unpack"
"$FIPTOOL" unpack "$UPSTREAM_FIP" --force --outdir "$WORKDIR/fiptool-unpack" > /dev/null 2>&1 || true
# If the upstream FIP has no BL1, fall back to the freshly built one.
if [ ! -f "$WORKDIR/fiptool-unpack/bl1.bin" ]; then
    cp "$TFA/bl1.bin" "$WORKDIR/fiptool-unpack/bl1.bin"
fi
cp "$WORKDIR/fiptool-unpack/bl1.bin" "$WORKDIR/fiptool-unpack/bl1.bin.orig"
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
git submodule update --init --depth 1 > /dev/null 2>&1 || true
make -C BaseTools clean > /dev/null 2>&1 || true
make -C BaseTools -j"$NCPU" > "$OUT/edk2-basetools.log" 2>&1

# Point the flash composition at our TF-A images.
cp "$OUT/fip.bin" "$WORKDIR/edk2-non-osi/Platform/Qemu/Sbsa/fip.bin"
cp "$WORKDIR/fiptool-unpack/bl1.bin.orig" "$WORKDIR/edk2-non-osi/Platform/Qemu/Sbsa/bl1.bin"

# Workspace: edk2-platforms and edk2-non-osi must be real subdirectories of
# WORKSPACE; edk2's own Conf is symlinked so $WORKSPACE/Conf resolves.
ln -sfn "$WORKDIR/edk2/Conf" "$WORKDIR/Conf"
sed -i '' 's|^ACTIVE_PLATFORM.*|ACTIVE_PLATFORM       = edk2-platforms/Platform/Qemu/SbsaQemu/SbsaQemu.dsc|' "$WORKDIR/edk2/Conf/target.txt"
sed -i '' 's|^TARGET_ARCH.*|TARGET_ARCH           = AARCH64|' "$WORKDIR/edk2/Conf/target.txt"
sed -i '' 's|^TOOL_CHAIN_TAG.*|TOOL_CHAIN_TAG       = GCC|' "$WORKDIR/edk2/Conf/target.txt"

export WORKSPACE="$WORKDIR"
export PACKAGES_PATH="$WORKDIR/edk2:$WORKDIR/edk2-platforms:$WORKDIR/edk2-non-osi"
export EDK_TOOLS_PATH="$WORKDIR/edk2/BaseTools"
export PYTHONPATH="$WORKDIR/edk2/BaseTools/Source/Python"
export PATH="$WORKDIR/edk2/BaseTools/BinWrappers/PosixLike:/opt/homebrew/bin:$PATH"
export GCC_AARCH64_PREFIX=aarch64-elf-

python3 "$WORKDIR/edk2/BaseTools/Source/Python/build/build.py" \
    -b RELEASE -a AARCH64 -t GCC -j"$NCPU" > "$OUT/edk2-build.log" 2>&1

# =============================================================================
# Package the flash images
# =============================================================================
echo ">>> Packaging SBSA_FLASH0.fd / SBSA_FLASH1.fd"
cp "$WORKDIR/Build/SbsaQemu/RELEASE_GCC/FV/SBSA_FLASH0.fd" "$OUT/SBSA_FLASH0.fd"
cp "$WORKDIR/Build/SbsaQemu/RELEASE_GCC/FV/SBSA_FLASH1.fd" "$OUT/SBSA_FLASH1.fd"
truncate -s 256M "$OUT/SBSA_FLASH0.fd"
truncate -s 256M "$OUT/SBSA_FLASH1.fd"

echo
echo ">>> Done. Outputs:"
echo "    $OUT/SBSA_FLASH0.fd  (TF-A: EL1 handoff + CPU-topology SMC)"
echo "    $OUT/SBSA_FLASH1.fd  (edk2 QemuSbsa UEFI)"
echo
echo ">>> To run:"
echo "    qemu-system-aarch64 -M sbsa-ref -cpu neoverse-n1 -smp 1 -m 512M \\"
echo "      -pflash $OUT/SBSA_FLASH0.fd -pflash $OUT/SBSA_FLASH1.fd \\"
echo "      -drive if=none,file=<boot.img>,format=raw,id=nvme0 \\"
echo "      -device nvme,drive=nvme0,serial=cat0"
