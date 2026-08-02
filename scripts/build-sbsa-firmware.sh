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
# bringing sbsa-ref up. See docs/sbsa-ref-bringup.md for the full write-up.
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
    local repo="$1" dir="$2"
    if [ ! -d "$dir" ]; then
        git clone --depth 1 "$repo" "$dir"
    else
        echo "    (reusing $dir)"
    fi
}

echo ">>> Cloning sources"
clone https://github.com/ARM-software/arm-trusted-firmware.git "$WORKDIR/tf-a"
clone https://github.com/tianocore/edk2.git "$WORKDIR/edk2"
clone https://github.com/tianocore/edk2-platforms.git "$WORKDIR/edk2-platforms"
clone https://github.com/tianocore/edk2-non-osi.git "$WORKDIR/edk2-non-osi"

# =============================================================================
# TF-A patches
# =============================================================================

# (1) Hand BL33 (UEFI/Limine) off at EL1 instead of EL2.
TFBL2="$WORKDIR/tf-a/plat/qemu/common/qemu_bl2_setup.c"
if ! grep -q "CharlotteOS" "$TFBL2"; then
    echo ">>> TF-A: forcing BL33 entry at EL1"
    perl -0pi -e 's/\t\/\* Figure out what mode we enter the non-secure world in \*\//\t\/\* CharlotteOS: hand the bootloader (BL33) off at EL1.\n\t * QEMU TCG EL2 is unreliable and the kernel targets EL1\/EL0. *\//' "$TFBL2"
    perl -0pi -e 's/mode = \(el_implemented\(2\) != EL_IMPL_NONE\) \? MODE_EL2 : MODE_EL1;/mode = MODE_EL1;/' "$TFBL2"
fi

# (2) Implement SIP_SVC_GET_CPU_TOPOLOGY (SMC 202), required by the current
#     edk2 QemuSbsa HardwareInfoLib but missing from TF-A v2.11. Without it the
#     UEFI loops calling ResetShutdown().
SVC="$WORKDIR/tf-a/plat/qemu/qemu_sbsa/sbsa_sip_svc.c"
if ! grep -q "GET_CPU_TOPOLOGY" "$SVC"; then
    echo ">>> TF-A: adding SIP_SVC_GET_CPU_TOPOLOGY (SMC 202)"
    # define
    perl -0pi -e 's/#define SIP_SVC_GET_CPU_NODE SIP_FUNCTION_ID\(201\)/#define SIP_SVC_GET_CPU_NODE SIP_FUNCTION_ID(201)\n#define SIP_SVC_GET_CPU_TOPOLOGY SIP_FUNCTION_ID(202)/' "$SVC"
    # topology storage
    perl -0pi -e 's/} dynamic_platform_info;/} dynamic_platform_info;\n\nstatic struct {\n\tuint32_t sockets;\n\tuint32_t clusters;\n\tuint32_t cores;\n\tuint32_t threads;\n} cpu_topology;/' "$SVC"
    # reader (insert before sip_svc_init)
    python3 - "$SVC" <<'EOF'
import sys
p = sys.argv[1]
s = open(p).read()
reader = '''
void read_cpu_topology_from_dt(void *dtb)
{
	int node;
	const fdt32_t *prop;

	node = fdt_path_offset(dtb, "/cpus/topology");
	if (node < 0) {
		cpu_topology.sockets = 1;
		cpu_topology.clusters = 1;
		cpu_topology.cores = dynamic_platform_info.num_cpus;
		cpu_topology.threads = 1;
		return;
	}
	prop = fdt_getprop(dtb, node, "threads", NULL);
	cpu_topology.threads = prop ? fdt32_ld(prop) : 1;
	prop = fdt_getprop(dtb, node, "cores", NULL);
	cpu_topology.cores = prop ? fdt32_ld(prop) : dynamic_platform_info.num_cpus;
	prop = fdt_getprop(dtb, node, "clusters", NULL);
	cpu_topology.clusters = prop ? fdt32_ld(prop) : 1;
	prop = fdt_getprop(dtb, node, "sockets", NULL);
	cpu_topology.sockets = prop ? fdt32_ld(prop) : 1;
}

'''
marker = 'void sip_svc_init(void)'
assert marker in s
s = s.replace(marker, reader + marker, 1)
# call it in sip_svc_init
s = s.replace(
    '\tread_cpuinfo_from_dt(dtb);',
    '\tread_cpuinfo_from_dt(dtb);\n\tread_cpu_topology_from_dt(dtb);', 1)
# SMC case
s = s.replace(
    '\tcase SIP_SVC_GET_CPU_NODE:\n\t\tindex = x1;\n\t\tif (index < PLATFORM_CORE_COUNT) {\n\t\t\tSMC_RET3(handle, NULL,\n\t\t\t\tdynamic_platform_info.cpu[index].nodeid,\n\t\t\t\tdynamic_platform_info.cpu[index].mpidr);\n\t\t} else {\n\t\t\tSMC_RET1(handle, SMC_ARCH_CALL_INVAL_PARAM);\n\t\t}',
    '\tcase SIP_SVC_GET_CPU_NODE:\n\t\tindex = x1;\n\t\tif (index < PLATFORM_CORE_COUNT) {\n\t\t\tSMC_RET3(handle, NULL,\n\t\t\t\tdynamic_platform_info.cpu[index].nodeid,\n\t\t\t\tdynamic_platform_info.cpu[index].mpidr);\n\t\t} else {\n\t\t\tSMC_RET1(handle, SMC_ARCH_CALL_INVAL_PARAM);\n\t\t}\n\n\tcase SIP_SVC_GET_CPU_TOPOLOGY:\n\t\tSMC_RET5(handle, NULL,\n\t\t\tcpu_topology.sockets,\n\t\t\tcpu_topology.clusters,\n\t\t\tcpu_topology.cores,\n\t\t\tcpu_topology.threads);', 1)
open(p, 'w').write(s)
EOF
fi

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
BUILDPY="$WORKDIR/edk2/BaseTools/Source/Python/build/build.py"
if ! grep -q "isinstance(self.PlatformFile" "$BUILDPY"; then
    echo ">>> Patching edk2 build tool (-p PathClass conversion)"
    python3 - "$BUILDPY" <<'EOF'
import sys
p = sys.argv[1]
s = open(p).read()
old = """            self.PlatformFile = PathClass(NormFile(PlatformFile, self.WorkspaceDir), self.WorkspaceDir)

        self.GetToolChainAndFamilyFromDsc (self.PlatformFile)"""
new = """            self.PlatformFile = PathClass(NormFile(PlatformFile, self.WorkspaceDir), self.WorkspaceDir)
        else:
            # -p was given: BuildOptions.PlatformFile is a plain string; convert
            # it to a PathClass so the workspace database can inspect it.
            if not isinstance(self.PlatformFile, PathClass):
                self.PlatformFile = PathClass(NormFile(self.PlatformFile, self.WorkspaceDir), self.WorkspaceDir)

        self.GetToolChainAndFamilyFromDsc (self.PlatformFile)"""
assert old in s, "edk2 build.py pattern not found"
s = s.replace(old, new)
open(p, 'w').write(s)
EOF
fi

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
