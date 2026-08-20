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
# virtio-net + cluster-discovery networking path.
#
# Usage:
#   scripts/run-x86_64.sh [debug|release] [--clean] [--gdb] [--gdb-port PORT]
#                         [--instance NAME] [--smp N] [--timeout S]
#                         [--iommu intel|amd] [--block nvme|ahci|virtio]
#                         [--net-test|--disco-test] [--mac ADDRESS]
#                         [--net-listen PORT|--net-connect HOST:PORT]
#                         [--fresh-storage|--reuse-storage]
#
#   debug|release  Build profile (default: debug)
#   --clean        Remove all cached x86_64 target artifacts before building
#   --gdb          Start QEMU paused with a gdb stub
#   --gdb-port PORT  GDB stub port (default: 1234)
#   --instance NAME  Use separate boot/log files for this VM
#   --smp N        Number of CPUs (default: 4)
#   --timeout S    Kill QEMU after S seconds, capturing serial output
#                  (default: run interactively)
#   --iommu intel|amd  DMA remapping unit (default: intel)
#   --block nvme|ahci|virtio  Block device transport (default: nvme)
#   --net-test     Build and run the virtio-net test
#   --disco-test   Run the cluster discovery test (implies --net-test)
#   --mac ADDRESS  Set the guest NIC MAC address
#   --net-listen PORT  Put the guest NIC on a QEMU socket LAN and listen
#   --net-connect HOST:PORT  Connect the guest NIC to a QEMU socket LAN
#   --fresh-storage  Recreate this instance's persistent block-device image
#   --reuse-storage  Keep it even when the signed service bundle changed
#   --el0-smoke    Build + sign the x86_64 `smoke` service ELF and run the
#                  EL0 Rust-ELF round-trip self-test
#
set -euo pipefail

ARCH="x86_64"
PROFILE="debug"
GDB=""
GDB_PORT="1234"
SMP="4"
TIMEOUT=""
CLEAN_BUILD="0"
INSTANCE=""
FRESH_STORAGE="0"
REUSE_STORAGE="0"
EL0_SMOKE="0"
IOMMU="intel"
BLOCK="nvme"
NET_TEST="0"
DISCO_TEST="0"
DNS_TEST="0"
TCPIP_TEST="0"
HTTP_TEST="0"
NET_BACKEND="user"
NET_MAC="52:54:00:12:34:56"

while [ "$#" -gt 0 ]; do
    case "$1" in
        debug|release) PROFILE="$1"; shift ;;
        --clean)       CLEAN_BUILD="1"; shift ;;
        --gdb)         GDB="-S"; shift ;;
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
        --net-test)    NET_TEST="1"; shift ;;
        --disco-test)  NET_TEST="1"; DISCO_TEST="1"; shift ;; # implies --net-test
        --dns-test)    NET_TEST="1"; DISCO_TEST="1"; DNS_TEST="1"; shift ;; # implies --disco-test
        --tcpip-test)  NET_TEST="1"; TCPIP_TEST="1"; shift ;; # implies --net-test
        --http-test)   NET_TEST="1"; HTTP_TEST="1"; shift ;; # implies --net-test
        --net-listen)
            [ "$#" -ge 2 ] || { echo "Missing value for --net-listen" >&2; exit 1; }
            NET_BACKEND="listen:$2"; shift 2 ;;
        --net-connect)
            [ "$#" -ge 2 ] || { echo "Missing value for --net-connect" >&2; exit 1; }
            NET_BACKEND="connect:$2"; shift 2 ;;
        --mac)
            [ "$#" -ge 2 ] || { echo "Missing value for --mac" >&2; exit 1; }
            NET_MAC="$2"; shift 2 ;;
        --fresh-storage) FRESH_STORAGE="1"; shift ;;
        --reuse-storage) REUSE_STORAGE="1"; shift ;;
        --el0-smoke)     EL0_SMOKE="1"; shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if ! [[ "$GDB_PORT" =~ ^[0-9]+$ ]] || [ "$GDB_PORT" -lt 1 ] || [ "$GDB_PORT" -gt 65535 ]; then
    echo "error: --gdb-port must be an integer from 1 through 65535" >&2
    exit 1
fi
if [ -n "$INSTANCE" ] && [[ ! "$INSTANCE" =~ ^[A-Za-z0-9._-]+$ ]]; then
    echo "error: --instance may contain only letters, digits, '.', '_' and '-'" >&2
    exit 1
fi
if ! [[ "$SMP" =~ ^[0-9]+$ ]] || [ "$SMP" -lt 1 ]; then
    echo "error: --smp must be a positive integer" >&2
    exit 1
fi
if [ -n "$TIMEOUT" ] && { ! [[ "$TIMEOUT" =~ ^[0-9]+$ ]] || [ "$TIMEOUT" -lt 1 ]; }; then
    echo "error: --timeout must be a positive integer" >&2
    exit 1
fi
if [ "$FRESH_STORAGE" = "1" ] && [ "$REUSE_STORAGE" = "1" ]; then
    echo "error: --fresh-storage and --reuse-storage are mutually exclusive" >&2
    exit 1
fi
if [ "$NET_BACKEND" != "user" ] && [ "$NET_TEST" != "1" ]; then
    echo "error: socket networking requires a network test option" >&2
    exit 1
fi

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

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

# Resolve the edk2 x86_64 firmware shipped with QEMU.
FW=""
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
for tool in mformat mmd mcopy; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "error: $tool not found; install mtools (brew install mtools)" >&2
        exit 1
    fi
done

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
if [ "$TCPIP_TEST" = "1" ]; then
    FEATURES="${FEATURES},tcpip_net_test"
fi
if [ "$HTTP_TEST" = "1" ]; then
    FEATURES="${FEATURES},http_net_test"
fi

# Build and sign the x86_64 service bundle. The bootstrap set is embedded at
# compile time and the same signed artifacts seed the persistent object-store
# image, so the bundle must exist before the kernel build.
echo ">>> Building and signing the x86_64 bootstrap service bundle..."
SERVICE_BUNDLE="${ROOT_DIR}/target/embedded-services/x86_64-unknown-none"
mkdir -p "$SERVICE_BUNDLE"
cargo build --manifest-path crates/catten-services/Cargo.toml \
    --target crates/catten-services/x86_64-unknown-none.json \
    --release -Z build-std=core,alloc \
    --bin ns --bin observe --bin nvme --bin objstore --bin nvme_client \
    --bin objstore_client --bin echo --bin raft --bin client --bin servicemgr \
    --bin ahci --bin virtio_blk --bin net --bin nclient --bin disco --bin frouter \
    --bin dns --bin agent --bin greet --bin relmsg --bin tcpip --bin tcpclient --bin httpd
for svc in ns observe nvme objstore nvme_client objstore_client echo raft client servicemgr ahci virtio_blk net nclient disco frouter dns agent greet relmsg tcpip tcpclient httpd; do
    cp "crates/catten-services/target/x86_64-unknown-none/release/$svc" "$SERVICE_BUNDLE/$svc.elf"
done
"${ROOT_DIR}/scripts/sign-service-elfs.sh" "$SERVICE_BUNDLE" >/dev/null
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
cargo build --package catten --target "$TARGET_SPEC" \
    --no-default-features --features "$FEATURES" $RELEASE_FLAG

if command -v sha256sum >/dev/null 2>&1; then
    KERNEL_SHA256="$(sha256sum "$KERNEL" | awk '{print $1}')"
else
    KERNEL_SHA256="$(shasum -a 256 "$KERNEL" | awk '{print $1}')"
fi
echo ">>> Kernel payload: ${KERNEL}"
echo ">>> Kernel SHA-256: ${KERNEL_SHA256}"

# --- Build a disposable FAT32 EFI System Partition image with mtools. ---
# It is deliberately separate from the block device delegated to userspace:
# formatting or object-store writes can no longer corrupt the next boot.
echo ">>> Creating boot image ${IMAGE}..."
mkdir -p "$IMAGE_DIR"
dd if=/dev/zero of="$IMAGE" bs=1048576 count=64 status=none
mformat -i "$IMAGE" -F -v CATOS ::
mmd -i "$IMAGE" ::/EFI
mmd -i "$IMAGE" ::/EFI/BOOT
mcopy -i "$IMAGE" "./limine-binary/${EFI_BOOT_FILE}" "::/EFI/BOOT/${EFI_BOOT_FILE}"
mcopy -i "$IMAGE" "$KERNEL" "::/catten"
mcopy -i "$IMAGE" "./limine.conf" "::/limine.conf"

BUNDLE_DIGESTS=""
for service_elf in "$SERVICE_BUNDLE"/*.elf; do
    if command -v sha256sum >/dev/null 2>&1; then
        service_digest="$(sha256sum "$service_elf" | awk '{print $1}')"
    else
        service_digest="$(shasum -a 256 "$service_elf" | awk '{print $1}')"
    fi
    BUNDLE_DIGESTS="${BUNDLE_DIGESTS}$(basename "$service_elf"):${service_digest}
"
done
if command -v sha256sum >/dev/null 2>&1; then
    CURRENT_BUNDLE_HASH="$(printf '%s' "$BUNDLE_DIGESTS" | sha256sum | awk '{print $1}')"
else
    CURRENT_BUNDLE_HASH="$(printf '%s' "$BUNDLE_DIGESTS" | shasum -a 256 | awk '{print $1}')"
fi
STORED_BUNDLE_HASH="$(test -f "$DATA_BUNDLE_HASH" && tr -d '[:space:]' < "$DATA_BUNDLE_HASH" || true)"
if [ "$REUSE_STORAGE" = "1" ] && [ -f "$DATA_IMAGE" ]; then
    echo ">>> Reusing block image ${DATA_IMAGE} by explicit request."
elif [ ! -f "$DATA_IMAGE" ] || [ "$FRESH_STORAGE" = "1" ] \
    || [ "$STORED_BUNDLE_HASH" != "$CURRENT_BUNDLE_HASH" ]; then
    echo ">>> Producing block image ${DATA_IMAGE} from the signed bundle..."
    python3 "${ROOT_DIR}/scripts/make-nvme-image.py" "$DATA_IMAGE" "$SERVICE_BUNDLE"
    printf '%s\n' "$CURRENT_BUNDLE_HASH" > "$DATA_BUNDLE_HASH"
else
    echo ">>> Reusing block image ${DATA_IMAGE} (signed bundle unchanged)."
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

QEMU_OPTS=(
    -M q35
    -cpu max
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
)

if [ "$NET_TEST" = "1" ]; then
    case "$NET_BACKEND" in
        user)
            QEMU_OPTS+=(-netdev "user,id=net0")
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
    QEMU_OPTS+=(
        -device "virtio-net-pci-non-transitional,netdev=net0,iommu_platform=on,mac=${NET_MAC}"
    )
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
    done
    kill "$QPID" 2>/dev/null || true
    wait "$QPID" 2>/dev/null || true
    echo ">>> Serial log (${LOG}):"
    cat "$LOG"
    if grep -Fq "Kernel panic:" "$LOG"; then
        echo "error: kernel panic observed during the test window" >&2
        exit 1
    fi
    if grep -Eq 'SELFTEST COMPLETE: passed=[0-9]+ failed=0 pending=0' "$LOG"; then
        echo ">>> All registered deferred self-tests passed."
    else
        echo "error: no successful authoritative self-test result was produced" >&2
        exit 1
    fi
else
    QEMU_OPTS+=(-serial stdio)
    echo ">>> Booting under QEMU (serial on stdio; press Ctrl-A X to quit)..."
    exec qemu-system-x86_64 "${QEMU_OPTS[@]}" $GDB
fi
