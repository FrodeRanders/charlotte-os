#!/usr/bin/env bash
#
# Build a bootable UEFI disk image for CharlotteOS / Catten (x86_64) and run it
# under QEMU. This is the x86_64 counterpart of scripts/run-aarch64.sh.
#
# Requirements: qemu, mtools.  On macOS install via Homebrew:
#   brew install qemu mtools
#
# The x86_64 port boots multi-LP, runs EL0 at ring 3 through the SYSCALL
# ABI, and passes the kernel-side + ring-3 deferred self-test suite, including
# the NVMe/AHCI/virtio-blk storage stack (behind VT-d or AMD-Vi) and the
# virtio-net/E1000E + cluster-discovery networking path.
#
# Usage:
#   scripts/run-x86_64.sh [debug|release] [--clean] [--gdb] [--gdb-port PORT] [--kvm]
#                         [--instance NAME] [--smp N] [--timeout S]
#                         [--iommu intel|amd] [--block nvme|ahci|virtio]
#                         [--no-network]
#                         [--net-test|--disco-test|--dns-test|--deploy-test]
#                         [--tcpip-test|--http-test|--dhcp-test] [--live-upgrade-test]
#                         [--nic virtio|e1000e] [--mac ADDRESS]
#                         [--net-listen PORT|--net-connect HOST:PORT]
#                         [--fresh-storage|--reuse-storage|--blank-storage]
#                         [--data-size-mib N] [--build-only]
#
#   debug|release  Build profile (default: debug)
#   --clean        Remove all cached x86_64 target artifacts before building
#   --gdb          Start QEMU paused with a gdb stub
#   --gdb-port PORT  GDB stub port (default: 1234)
#   --kvm          Use Linux KVM with the host x86-64 CPU
#   --instance NAME  Use separate boot/log files for this VM
#   --smp N        Number of CPUs (default: 4)
#   --timeout S    Kill QEMU after S seconds, capturing serial output
#                  (default: run interactively)
#   --iommu intel|amd  DMA remapping unit (default: intel)
#   --block nvme|ahci|virtio  Block device transport (default: nvme)
#   --no-network   Do not attach a NIC or launch network-backed services
#   --net-test     Verify the default userspace Ethernet-driver capability
#   --nic MODEL    Select virtio or e1000e for QEMU networking (default: virtio)
#   --disco-test   Run the cluster discovery test (implies --net-test)
#   --dns-test     Run the distributed DNS test (implies --disco-test)
#   --deploy-test  Run cluster deployment, clusterctl, and dynamic join tests
#                  (implies --dns-test; both guests must use this option)
#   --tcpip-test   Exchange TCP data through the userspace smoltcp service
#                  (requires two socket-linked guests)
#   --http-test    Serve the HTTP state keyhole through SLIRP host forwarding
#   --dhcp-test    Acquire a DHCP lease through the tcpip service (single guest)
#   --live-upgrade-test  Run the isolated persistent service-upgrade test
#   --mac ADDRESS  Set the guest NIC MAC address
#   --net-listen PORT  Put the guest NIC on a QEMU socket LAN and listen
#   --net-connect HOST:PORT  Connect the guest NIC to a QEMU socket LAN
#   --fresh-storage  Recreate this instance's persistent block-device image
#   --reuse-storage  Keep it even when the signed service bundle changed
#   --blank-storage  Create an empty data disk for first-boot formatting/seeding
#   --data-size-mib N  Empty data-disk size with --blank-storage (default: 64)
#   --build-only    Produce the boot/data images without starting QEMU
#   --el0-smoke    Build + sign the x86_64 `smoke` service ELF and run the
#                  EL0 Rust-ELF round-trip self-test
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=lib/boot-common.sh
source "${SCRIPT_DIR}/lib/boot-common.sh"

ARCH="x86_64"
PROFILE="debug"
GDB=""
GDB_PORT="1234"
USE_KVM="0"
SMP="4"
TIMEOUT=""
CLEAN_BUILD="0"
INSTANCE=""
FRESH_STORAGE="0"
REUSE_STORAGE="0"
BLANK_STORAGE="0"
DATA_SIZE_MIB="${CATTEN_DATA_SIZE_MIB:-64}"
BUILD_ONLY="0"
EL0_SMOKE="0"
IOMMU="intel"
BLOCK="nvme"
NETWORK="1"
NET_TEST="0"
DISCO_TEST="0"
DNS_TEST="0"
TCPIP_TEST="0"
HTTP_TEST="0"
DHCP_TEST="0"
DEPLOY_TEST="0"
LIVE_UPGRADE_TEST="0"
HTTP_HOST_PORT="${CATTEN_HTTP_HOST_PORT:-8080}"
NET_BACKEND="user"
NET_MAC="52:54:00:12:34:56"
NET_DEVICE="virtio"

while [ "$#" -gt 0 ]; do
    case "$1" in
        debug|release) PROFILE="$1"; shift ;;
        --clean)       CLEAN_BUILD="1"; shift ;;
        --gdb)         GDB="-S"; shift ;;
        --kvm)         USE_KVM="1"; shift ;;
        --gdb-port)
            [ "$#" -ge 2 ] || { echo "Missing value for --gdb-port" >&2; exit 1; }
            GDB_PORT="$2"; shift 2 ;;
        --instance)
            [ "$#" -ge 2 ] || { echo "Missing value for --instance" >&2; exit 1; }
            INSTANCE="$2"; shift 2 ;;
        --smp)
            [ "$#" -ge 2 ] || { echo "Missing value for --smp" >&2; exit 1; }
            SMP="$2"; shift 2 ;;
        --timeout)
            [ "$#" -ge 2 ] || { echo "Missing value for --timeout" >&2; exit 1; }
            TIMEOUT="$2"; shift 2 ;;
        --iommu)
            [ "$#" -ge 2 ] || { echo "Missing value for --iommu" >&2; exit 1; }
            IOMMU="$2"; shift 2 ;;
        --block)
            [ "$#" -ge 2 ] || { echo "Missing value for --block" >&2; exit 1; }
            BLOCK="$2"; shift 2 ;;
        --no-network) NETWORK="0"; shift ;;
        --net-test)    NET_TEST="1"; shift ;;
        --disco-test)  NET_TEST="1"; DISCO_TEST="1"; shift ;; # implies --net-test
        --dns-test)    NET_TEST="1"; DISCO_TEST="1"; DNS_TEST="1"; shift ;; # implies --disco-test
        --deploy-test) NET_TEST="1"; DISCO_TEST="1"; DNS_TEST="1"; DEPLOY_TEST="1"; shift ;; # implies --dns-test
        --tcpip-test)  NET_TEST="1"; TCPIP_TEST="1"; shift ;; # implies --net-test
        --http-test)   NET_TEST="1"; HTTP_TEST="1"; shift ;; # implies --net-test
        --dhcp-test)   NET_TEST="1"; DHCP_TEST="1"; shift ;; # implies --net-test
        --live-upgrade-test) LIVE_UPGRADE_TEST="1"; shift ;;
        --net-listen)
            [ "$#" -ge 2 ] || { echo "Missing value for --net-listen" >&2; exit 1; }
            NET_BACKEND="listen:$2"; shift 2 ;;
        --net-connect)
            [ "$#" -ge 2 ] || { echo "Missing value for --net-connect" >&2; exit 1; }
            NET_BACKEND="connect:$2"; shift 2 ;;
        --mac)
            [ "$#" -ge 2 ] || { echo "Missing value for --mac" >&2; exit 1; }
            NET_MAC="$2"; shift 2 ;;
        --nic)
            [ "$#" -ge 2 ] || { echo "Missing value for --nic" >&2; exit 1; }
            NET_DEVICE="$2"; shift 2 ;;
        --fresh-storage) FRESH_STORAGE="1"; shift ;;
        --reuse-storage) REUSE_STORAGE="1"; shift ;;
        --blank-storage) BLANK_STORAGE="1"; shift ;;
        --data-size-mib)
            [ "$#" -ge 2 ] || { echo "Missing value for --data-size-mib" >&2; exit 1; }
            DATA_SIZE_MIB="$2"; shift 2 ;;
        --build-only)    BUILD_ONLY="1"; shift ;;
        --el0-smoke)     EL0_SMOKE="1"; shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

catten_boot_validate_port "--gdb-port" "$GDB_PORT"
catten_boot_validate_instance "$INSTANCE"
catten_boot_validate_positive_integer "--smp" "$SMP"
if [ -n "$TIMEOUT" ]; then
    catten_boot_validate_positive_integer "--timeout" "$TIMEOUT"
fi
if [ "$FRESH_STORAGE" = "1" ] && [ "$REUSE_STORAGE" = "1" ]; then
    echo "error: --fresh-storage and --reuse-storage are mutually exclusive" >&2
    exit 1
fi
if ! [[ "$DATA_SIZE_MIB" =~ ^[0-9]+$ ]] || [ "$DATA_SIZE_MIB" -lt 16 ]; then
    echo "error: --data-size-mib must be an integer of at least 16" >&2
    exit 1
fi
catten_boot_validate_port "CATTEN_HTTP_HOST_PORT" "$HTTP_HOST_PORT"
if [ "$NET_BACKEND" != "user" ] && [ "$NETWORK" != "1" ]; then
    echo "error: socket networking is incompatible with --no-network" >&2
    exit 1
fi
if [ "$NETWORK" != "1" ] && { [ "$NET_TEST" = "1" ] || [ "$DISCO_TEST" = "1" ] \
    || [ "$DNS_TEST" = "1" ] || [ "$DEPLOY_TEST" = "1" ] || [ "$TCPIP_TEST" = "1" ] \
    || [ "$HTTP_TEST" = "1" ] || [ "$DHCP_TEST" = "1" ]; }; then
    echo "error: network verification options are incompatible with --no-network" >&2
    exit 1
fi
if [ "$NET_DEVICE" != "virtio" ] && [ "$NET_DEVICE" != "e1000e" ]; then
    echo "error: --nic must be 'virtio' or 'e1000e'" >&2
    exit 1
fi
if [ "$TCPIP_TEST" = "1" ] && [ "$NET_BACKEND" = "user" ]; then
    echo "error: --tcpip-test requires --net-listen or --net-connect" >&2
    exit 1
fi
if [ "$HTTP_TEST" = "1" ] && [ "$NET_BACKEND" != "user" ]; then
    echo "error: --http-test requires the default user network (hostfwd)" >&2
    exit 1
fi
if [ "$USE_KVM" = "1" ] && { [ "$(uname -s)" != "Linux" ] || [ ! -r /dev/kvm ] || [ ! -w /dev/kvm ]; }; then
    echo "error: --kvm requires a Linux host with accessible /dev/kvm" >&2
    exit 1
fi

cd "$ROOT_DIR"
catten_boot_init "$ROOT_DIR"
catten_boot_require_commands mformat mmd mcopy
LIMINE_CONFIG="$CATTEN_BOOT_LIMINE_CONFIG"

TARGET_SPEC="target_specs/${ARCH}-unknown-none-catten.json"
TARGET_DIR="${ARCH}-unknown-none-catten"
IMAGE_DIR="./os-images"
INSTANCE_SUFFIX=""
if [ -n "$INSTANCE" ]; then
    INSTANCE_SUFFIX="-${INSTANCE}"
fi
IMAGE="${IMAGE_DIR}/charlotte-${ARCH}-${PROFILE}${INSTANCE_SUFFIX}.img"
DATA_IMAGE="${IMAGE_DIR}/x86-data${INSTANCE_SUFFIX}.img"
DATA_BUNDLE_HASH="${DATA_IMAGE}.bundle-sha256"
KERNEL="./target/${TARGET_DIR}/${PROFILE}/catten"
EFI_BOOT_FILE="BOOTX64.EFI"

# Resolve the edk2 x86_64 firmware shipped with QEMU only when this invocation
# will actually boot QEMU. Image-only consumers such as VMware need no QEMU
# firmware or system emulator.
FW=""
if [ "$BUILD_ONLY" != "1" ]; then
    if command -v brew >/dev/null 2>&1; then
        QEMU_PREFIX="$(brew --prefix qemu 2>/dev/null || true)"
        [ -n "$QEMU_PREFIX" ] && FW="${QEMU_PREFIX}/share/qemu/edk2-x86_64-code.fd"
    fi
    if [ -z "$FW" ] || [ ! -f "$FW" ]; then
        for candidate in \
            /opt/homebrew/share/qemu/edk2-x86_64-code.fd \
            /usr/local/share/qemu/edk2-x86_64-code.fd \
            /usr/share/qemu/edk2-x86_64-code.fd; do
            if [ -f "$candidate" ]; then
                FW="$candidate"
                break
            fi
        done
    fi
    if [ -z "$FW" ] || [ ! -f "$FW" ]; then
        echo "error: QEMU edk2 x86_64 firmware not found (edk2-x86_64-code.fd)" >&2
        echo "       install qemu via Homebrew: brew install qemu" >&2
        exit 1
    fi

    if ! command -v qemu-system-x86_64 >/dev/null 2>&1; then
        echo "error: qemu-system-x86_64 not found" >&2
        exit 1
    fi
fi
RELEASE_FLAG=""
if [ "$PROFILE" = "release" ]; then
    RELEASE_FLAG="--release"
fi

if [ "$CLEAN_BUILD" = "1" ]; then
    echo ">>> Cleaning cached ${ARCH} kernel and dependency artifacts..."
    cargo clean --target "$TARGET_SPEC"
fi

FEATURES="acpi"
if [ "$NET_TEST" = "1" ]; then
    FEATURES="${FEATURES},virtio_net_test"
fi
if [ "$DISCO_TEST" = "1" ]; then
    FEATURES="${FEATURES},disco_net_test"
    # Enable cross-node verification when two instances are linked via socket.
    if [ "$NET_BACKEND" != "user" ]; then
        FEATURES="${FEATURES},disco_cross_node_test"
    fi
fi
if [ "$DNS_TEST" = "1" ]; then
    FEATURES="${FEATURES},dns_net_test"
fi
if [ "$DEPLOY_TEST" = "1" ]; then
    FEATURES="${FEATURES},deploy_net_test,clusterctl_test"
fi
if [ "$TCPIP_TEST" = "1" ]; then
    FEATURES="${FEATURES},tcpip_net_test"
fi
if [ "$HTTP_TEST" = "1" ]; then
    FEATURES="${FEATURES},http_net_test"
fi
if [ "$DHCP_TEST" = "1" ]; then
    FEATURES="${FEATURES},dhcp_test"
fi
if [ "$LIVE_UPGRADE_TEST" = "1" ]; then
    FEATURES="${FEATURES},live_upgrade_test"
fi
if [ "${CATTEN_SCHEDULER_TRACE:-0}" = "1" ]; then
    FEATURES="${FEATURES},scheduler_trace"
fi

# Build and sign the x86_64 service bundle. The bootstrap set is embedded at
# compile time and the same signed artifacts seed the persistent object-store
# image, so the bundle must exist before the kernel build.
echo ">>> Building and signing the x86_64 bootstrap service bundle..."
SERVICE_BUNDLE="${ROOT_DIR}/target/embedded-services/x86_64-unknown-none"
SERVICE_NAMES="ns observe nvme objstore nvme_client objstore_client echo raft client servicemgr ahci virtio_blk net e1000e nclient disco frouter dns agent greet relmsg rclient tcpip tcpclient httpd time fs clusterctl"
if [ "${CATTEN_SKIP_EMBED_BUILD:-0}" = "1" ]; then
    for svc in $SERVICE_NAMES; do
        if [ ! -f "$SERVICE_BUNDLE/$svc.elf" ]; then
            echo "error: CATTEN_SKIP_EMBED_BUILD=1 but $SERVICE_BUNDLE/$svc.elf is missing" >&2
            exit 1
        fi
    done
    echo ">>> Reusing staged x86_64 service bundle."
else
    "${ROOT_DIR}/scripts/build-catten-services-x86_64.sh"
fi
export CATTEN_X86_64_SERVICE_BUNDLE="$SERVICE_BUNDLE"

if [ "$EL0_SMOKE" = "1" ]; then
    echo ">>> Building and signing the x86_64 smoke service ELF..."
    cargo build --manifest-path crates/catten-services/Cargo.toml \
        --target crates/catten-services/x86_64-unknown-none.json \
        --release -Z build-std=core,alloc --bin smoke
    cp crates/catten-services/target/x86_64-unknown-none/release/smoke \
        "$SERVICE_BUNDLE/smoke.elf"
    "${ROOT_DIR}/scripts/sign-service-elfs.sh" "$SERVICE_BUNDLE" >/dev/null
    export CATTEN_X86_64_SMOKE_ELF="$SERVICE_BUNDLE/smoke.elf"
    FEATURES="${FEATURES},x86_el0_smoke"
fi

echo ">>> Building Catten kernel (${ARCH}, ${PROFILE}, headless)..."
if [ "${CATTEN_SKIP_KERNEL_BUILD:-0}" = "1" ]; then
    if [ ! -f "$KERNEL" ]; then
        echo "error: CATTEN_SKIP_KERNEL_BUILD=1 but $KERNEL is missing" >&2
        exit 1
    fi
    echo ">>> Reusing previously built Catten kernel."
else
    cargo build --package catten --target "$TARGET_SPEC" \
        --no-default-features --features "$FEATURES" $RELEASE_FLAG
fi

catten_boot_report_kernel "$KERNEL"

# --- Build a disposable FAT32 EFI System Partition image with mtools. ---
# It is deliberately separate from the block device delegated to userspace:
# formatting or object-store writes can no longer corrupt the next boot.
catten_boot_create_uefi_image \
    "$IMAGE" \
    64 \
    "${ROOT_DIR}/limine-binary/${EFI_BOOT_FILE}" \
    "$KERNEL" \
    "$LIMINE_CONFIG" \
    CATOS

CURRENT_BUNDLE_HASH="$(catten_boot_bundle_sha256 "$SERVICE_BUNDLE")"
STORED_BUNDLE_HASH="$(test -f "$DATA_BUNDLE_HASH" && tr -d '[:space:]' < "$DATA_BUNDLE_HASH" || true)"
BLANK_LAYOUT="blank:${DATA_SIZE_MIB}MiB"
if [ "$BLANK_STORAGE" = "1" ] && [ "$REUSE_STORAGE" = "1" ] && [ -f "$DATA_IMAGE" ]; then
    echo ">>> Reusing empty/installed first-boot disk ${DATA_IMAGE} by explicit request."
elif [ "$BLANK_STORAGE" = "1" ] && { [ ! -f "$DATA_IMAGE" ] \
    || [ "$FRESH_STORAGE" = "1" ] || [ "$STORED_BUNDLE_HASH" != "$BLANK_LAYOUT" ]; }; then
    echo ">>> Producing empty ${DATA_SIZE_MIB} MiB first-boot disk ${DATA_IMAGE}..."
    dd if=/dev/zero of="$DATA_IMAGE" bs=1 count=0 seek=$((DATA_SIZE_MIB * 1048576)) status=none
    printf '%s\n' "$BLANK_LAYOUT" > "$DATA_BUNDLE_HASH"
elif [ "$BLANK_STORAGE" = "1" ]; then
    echo ">>> Reusing empty/installed first-boot disk ${DATA_IMAGE}."
elif [ "$REUSE_STORAGE" = "1" ] && [ -f "$DATA_IMAGE" ]; then
    echo ">>> Reusing block image ${DATA_IMAGE} by explicit request."
elif [ ! -f "$DATA_IMAGE" ] || [ "$FRESH_STORAGE" = "1" ] \
    || [ "$STORED_BUNDLE_HASH" != "$CURRENT_BUNDLE_HASH" ]; then
    echo ">>> Producing block image ${DATA_IMAGE} from the signed bundle..."
    python3 "${ROOT_DIR}/scripts/make-nvme-image.py" "$DATA_IMAGE" "$SERVICE_BUNDLE"
    printf '%s\n' "$CURRENT_BUNDLE_HASH" > "$DATA_BUNDLE_HASH"
else
    echo ">>> Reusing block image ${DATA_IMAGE} (signed bundle unchanged)."
fi

if [ "$BUILD_ONLY" = "1" ]; then
    echo ">>> Image build complete."
    echo ">>> Boot image: ${IMAGE}"
    echo ">>> Data image: ${DATA_IMAGE}"
    exit 0
fi

case "$IOMMU" in
    intel) IOMMU_DEVICE="intel-iommu" ;;
    amd)   IOMMU_DEVICE="amd-iommu,dma-remap=on" ;;
    *) echo "error: --iommu must be 'intel' or 'amd'" >&2; exit 1 ;;
esac

case "$BLOCK" in
    nvme)   BLOCK_DEVICE=("-device" "nvme,drive=data0,serial=cat0") ;;
    ahci)   BLOCK_DEVICE=("-device" "ide-hd,drive=data0,bus=ide.0") ;;
    virtio) BLOCK_DEVICE=("-device" "virtio-blk-pci-non-transitional,drive=data0,iommu_platform=on") ;;
    *) echo "error: --block must be 'nvme', 'ahci', or 'virtio'" >&2; exit 1 ;;
esac

CPU_OPTS=(-cpu max)
if [ "$USE_KVM" = "1" ]; then
    CPU_OPTS=(-accel kvm -cpu host,+invtsc)
fi

QEMU_OPTS=(
    -M q35
    "${CPU_OPTS[@]}"
    -smp "$SMP"
    -m 512M
    -drive "if=pflash,format=raw,unit=0,file=${FW},readonly=on"
    -drive "if=none,file=${IMAGE},format=raw,id=esp,readonly=on"
    -device "qemu-xhci,id=xhci"
    -device "usb-storage,bus=xhci.0,drive=esp,bootindex=1"
    -drive "if=none,file=${DATA_IMAGE},format=raw,id=data0"
    "${BLOCK_DEVICE[@]}"
    -device "$IOMMU_DEVICE"
    -display none
    -no-reboot
    -nic none
)

if [ "$NETWORK" = "1" ]; then
    case "$NET_BACKEND" in
        user)
            if [ "$HTTP_TEST" = "1" ]; then
                QEMU_OPTS+=(-netdev "user,id=net0,hostfwd=tcp::${HTTP_HOST_PORT}-:80")
            else
                QEMU_OPTS+=(-netdev "user,id=net0")
            fi
            ;;
        listen:*)
            NET_PORT="${NET_BACKEND#listen:}"
            # QEMU 11.1's stream backend can dereference a cleared channel
            # while virtio-net is transmitting on macOS.  The socket backend
            # implements the same point-to-point LAN without that host crash.
            QEMU_OPTS+=(-netdev "socket,id=net0,listen=0.0.0.0:${NET_PORT}")
            ;;
        connect:*)
            NET_PEER="${NET_BACKEND#connect:}"
            NET_HOST="${NET_PEER%%:*}"
            NET_PORT="${NET_PEER#*:}"
            QEMU_OPTS+=(-netdev "socket,id=net0,connect=${NET_HOST}:${NET_PORT}")
            ;;
    esac
    case "$NET_DEVICE" in
        virtio)
            QEMU_OPTS+=(
                -device "virtio-net-pci-non-transitional,netdev=net0,iommu_platform=on,mac=${NET_MAC}"
            )
            ;;
        e1000e)
            QEMU_OPTS+=(-device "e1000e,netdev=net0,mac=${NET_MAC}")
            ;;
    esac
fi

if [ -n "$GDB" ]; then
    QEMU_OPTS+=(-gdb "tcp::${GDB_PORT}")
fi

if [ -n "$TIMEOUT" ]; then
    LOG="/tmp/charlotte-x86${INSTANCE_SUFFIX}-serial.log"
    : >"$LOG"
    QEMU_OPTS+=(-serial "file:${LOG}")
    echo ">>> Booting under QEMU (${TIMEOUT}s timeout, serial to ${LOG})..."
    qemu-system-x86_64 "${QEMU_OPTS[@]}" $GDB &
    QPID=$!
    SELFTEST_COMPLETE=0
    SELFTEST_COMPLETE_TICK=-1
    CLUSTER_DRAIN_TICKS=0
    if [ "$NET_BACKEND" != "user" ]; then
        CLUSTER_DRAIN_TICKS=150
    fi
    HTTP_PROBED=0
    HTTP_PROBE_OK=0
    MAX_TICKS=$((TIMEOUT * 10))
    for ((tick = 0; tick < MAX_TICKS; tick++)); do
        sleep 0.1
        if ! kill -0 "$QPID" 2>/dev/null; then
            wait "$QPID" 2>/dev/null || true
            echo "error: QEMU exited before the ${TIMEOUT}s window elapsed" >&2
            if [ -f "$LOG" ]; then
                echo ">>> Serial log (${LOG}):"
                cat "$LOG"
            fi
            exit 1
        fi
        if [ "$HTTP_TEST" = "1" ] && [ "$HTTP_PROBED" = "0" ] \
            && grep -Fq "httpd is listening" "$LOG"; then
            HTTP_PROBED=1
            echo ">>> Probing guest HTTP keyhole at http://127.0.0.1:${HTTP_HOST_PORT}/metrics ..."
            for _ in 1 2 3 4 5 6 7 8; do
                HTTP_BODY="$(curl -fsS --max-time 5 http://127.0.0.1:${HTTP_HOST_PORT}/metrics 2>&1 || true)"
                if printf '%s' "$HTTP_BODY" | grep -Fq '"http":{"requests":'; then
                    HTTP_PROBE_OK=1
                    break
                fi
                sleep 2
            done
            echo ">>> Guest HTTP keyhole response:"
            echo "$HTTP_BODY"
        fi
        if grep -Fq "SELFTEST COMPLETE:" "$LOG"; then
            SELFTEST_COMPLETE=1
            if [ "$SELFTEST_COMPLETE_TICK" -lt 0 ]; then
                SELFTEST_COMPLETE_TICK=$tick
                echo ">>> Authoritative self-test result observed after $(((tick + 1) / 10))s."
                if [ "$CLUSTER_DRAIN_TICKS" -gt 0 ]; then
                    echo ">>> Keeping the socket-linked guest alive for a 15s peer drain window."
                fi
            fi
            if [ "$tick" -ge $((SELFTEST_COMPLETE_TICK + CLUSTER_DRAIN_TICKS)) ] \
                && { [ "$HTTP_TEST" != "1" ] || [ "$HTTP_PROBE_OK" = "1" ]; }; then
                break
            fi
        fi
    done
    kill "$QPID" 2>/dev/null || true
    wait "$QPID" 2>/dev/null || true
    echo ">>> Serial log (${LOG}):"
    cat "$LOG"
    if [ "$HTTP_TEST" = "1" ] && { [ "$HTTP_PROBED" = "0" ] || [ "$HTTP_PROBE_OK" = "0" ]; }; then
        echo "error: guest HTTP keyhole was not validated from the host" >&2
        exit 1
    fi
    if [ "$SELFTEST_COMPLETE" -ne 1 ]; then
        echo "error: authoritative self-test result was not produced within ${TIMEOUT}s" >&2
        exit 1
    fi
    catten_boot_validate_selftest_log "$LOG"
else
    QEMU_OPTS+=(-serial stdio)
    echo ">>> Booting under QEMU (serial on stdio; press Ctrl-A X to quit)..."
    exec qemu-system-x86_64 "${QEMU_OPTS[@]}" $GDB
fi
