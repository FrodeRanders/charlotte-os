#!/usr/bin/env bash
#
# Build a bootable UEFI disk image for CharlotteOS / Catten and run it under
# QEMU. Works on macOS (including Apple Silicon) and Linux.
#
# Requirements: qemu, mtools.  On macOS install via Homebrew:
#   brew install qemu mtools
# For HVF acceleration on Apple Silicon, use --hvf.
# For display (flanterm framebuffer console), use --display.
#
# Usage:
#   scripts/run-aarch64.sh [debug|release] [--clean] [--display] [--gdb] [--debug-snapshot] [--scheduler-trace] [--hvf] [--net-test|--relmsg-test|--disco-test] [--net-listen PORT|--net-connect HOST:PORT] [--instance NAME] [--mac ADDRESS] [--live-upgrade-test] [--smp N] [--timeout S]
#
#   debug|release  Build profile (default: debug)
#   --clean        Remove all cached AArch64 target artifacts before building
#   --display      Build with framebuffer console (flanterm), boot with ramfb
#   --gdb          Start QEMU paused with gdb stub on tcp::1234
#   --debug-snapshot  Capture all-LP stacks/registers at timeout without enabling tracing
#   --scheduler-trace  Capture and decode the in-memory scheduler trace at timeout
#   --hvf          Use Apple Hypervisor.Framework acceleration (macOS only)
#   --net-test     Build and run the virtio-net test under TCG/KVM
#   --relmsg-test  Exchange reliable messages with a second socket-LAN guest
#   --disco-test   Run the cluster discovery test (implies --net-test)
#   --dns-test     Run the distributed name service test (Raft over the
#               network; both guests must run it, implies --disco-test)
#   --net-listen PORT  Put the guest NIC on a QEMU socket LAN and listen
#   --net-connect HOST:PORT  Connect the guest NIC to a QEMU socket LAN
#   --instance NAME  Use separate boot/NVMe/log files for this VM
#   --mac ADDRESS  Set the guest NIC MAC address
#   --live-upgrade-test  Run the isolated EL0 service lifecycle/upgrade integration test
#   --smp N        Number of CPUs (default: 4)
#   --timeout S    Kill QEMU after S seconds, capturing serial output (default: run interactively)
#
set -euo pipefail

ARCH="aarch64"
PROFILE="debug"
GDB=""
DISPLAY_MODE="0"
USE_HVF="0"
NET_TEST="0"
RELMSG_TEST="0"
DISCO_TEST="0"
DNS_TEST="0"
LIVE_UPGRADE_TEST="0"
SMP="4"
TIMEOUT=""
CLEAN_BUILD="0"
SCHEDULER_TRACE="0"
DEBUG_SNAPSHOT="0"
INSTANCE=""
NET_BACKEND="user"
NET_MAC="52:54:00:12:34:56"

while [ "$#" -gt 0 ]; do
    case "$1" in
        debug|release) PROFILE="$1"; shift ;;
        --clean)       CLEAN_BUILD="1"; shift ;;
        --display)     DISPLAY_MODE="1"; shift ;;
        --gdb)         GDB="-s -S"; shift ;;
        --debug-snapshot) DEBUG_SNAPSHOT="1"; shift ;;
        --scheduler-trace) SCHEDULER_TRACE="1"; shift ;;
        --hvf)         USE_HVF="1"; shift ;;
        --net-test)    NET_TEST="1"; shift ;;
        --relmsg-test) NET_TEST="1"; RELMSG_TEST="1"; shift ;;
        --disco-test)  NET_TEST="1"; DISCO_TEST="1"; shift ;; # implies --net-test
        --dns-test)    NET_TEST="1"; DISCO_TEST="1"; DNS_TEST="1"; shift ;; # implies --disco-test
        --net-listen)
            [ "$#" -ge 2 ] || { echo "Missing value for --net-listen" >&2; exit 1; }
            NET_BACKEND="listen:$2"; shift 2 ;;
        --net-connect)
            [ "$#" -ge 2 ] || { echo "Missing value for --net-connect" >&2; exit 1; }
            NET_BACKEND="connect:$2"; shift 2 ;;
        --instance)
            [ "$#" -ge 2 ] || { echo "Missing value for --instance" >&2; exit 1; }
            INSTANCE="$2"; shift 2 ;;
        --mac)
            [ "$#" -ge 2 ] || { echo "Missing value for --mac" >&2; exit 1; }
            NET_MAC="$2"; shift 2 ;;
        --live-upgrade-test) LIVE_UPGRADE_TEST="1"; shift ;;
        --smp)
            [ "$#" -ge 2 ] || { echo "Missing value for --smp" >&2; exit 1; }
            SMP="$2"; shift 2 ;;
        --timeout)
            [ "$#" -ge 2 ] || { echo "Missing value for --timeout" >&2; exit 1; }
            TIMEOUT="$2"; shift 2 ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [ -n "$INSTANCE" ] && [[ ! "$INSTANCE" =~ ^[A-Za-z0-9._-]+$ ]]; then
    echo "error: --instance may contain only letters, digits, '.', '_' and '-'" >&2
    exit 1
fi
if [ "$NET_BACKEND" != "user" ] && [ "$NET_TEST" != "1" ]; then
    echo "error: socket networking requires --net-test" >&2
    exit 1
fi
if [ "$RELMSG_TEST" = "1" ] && [ "$NET_BACKEND" = "user" ]; then
    echo "error: --relmsg-test requires --net-listen or --net-connect" >&2
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
KERNEL="./target/${TARGET_DIR}/${PROFILE}/catten"
EFI_BOOT_FILE="BOOTAA64.EFI"

# On macOS, firmware is under /opt/homebrew; on Linux it's under /usr/share.
if [ -f "/opt/homebrew/share/qemu/edk2-aarch64-code.fd" ]; then
    FIRMWARE="/opt/homebrew/share/qemu/edk2-aarch64-code.fd"
else
    FIRMWARE="/usr/share/AAVMF/AAVMF_CODE.fd"
fi

RELEASE_FLAG=""
if [ "$PROFILE" = "release" ]; then
    RELEASE_FLAG="--release"
fi

if [ "${CATTEN_SKIP_EMBED_BUILD:-0}" = "1" ]; then
    echo ">>> Reusing staged AArch64 EL0 bundle."
elif [ "$CLEAN_BUILD" = "1" ]; then
    echo ">>> Cleaning cached ${ARCH} kernel and dependency artifacts..."
    cargo clean --target "$TARGET_SPEC"
    echo ">>> Cleaning and rebuilding embedded EL0 service bundle..."
    "${ROOT_DIR}/scripts/build-catten-services.sh" --embed --clean
else
    echo ">>> Rebuilding embedded EL0 service bundle..."
    "${ROOT_DIR}/scripts/build-catten-services.sh" --embed
fi
if [ "${CATTEN_SKIP_EMBED_BUILD:-0}" != "1" ]; then
    "${ROOT_DIR}/scripts/build-catten-user.sh" --embed
fi
export CATTEN_AARCH64_SERVICE_BUNDLE="${ROOT_DIR}/target/embedded-services/aarch64-unknown-none"

# Feature selection.
FEATURES="acpi"
BUILD_EXTRA=""
if [ "$DISPLAY_MODE" = "1" ]; then
    SYSROOT="$(rustc --print sysroot)"
    HOST_TRIPLE="$(rustc -vV | awk '/^host:/ {print $2}')"
    LLVM_AR="${SYSROOT}/lib/rustlib/${HOST_TRIPLE}/bin/llvm-ar"
    if [ ! -x "$LLVM_AR" ]; then
        echo "error: llvm-ar not found at ${LLVM_AR}" >&2
        echo "       run: rustup component add llvm-tools" >&2
        exit 1
    fi
    export AR_aarch64_unknown_none_catten="$LLVM_AR"
    FEATURES="acpi,display,virtio_gpu"
    echo ">>> Building Catten kernel (${ARCH}, ${PROFILE}, display)..."
else
    echo ">>> Building Catten kernel (${ARCH}, ${PROFILE}, headless)..."
fi

if [ "$USE_HVF" = "1" ]; then
    FEATURES="${FEATURES},hvf_compat"
fi

if [ "$NET_TEST" = "1" ]; then
    if [ "$USE_HVF" = "1" ]; then
        echo "error: --net-test is incompatible with --hvf (EL0 MMIO is unsupported)" >&2
        exit 1
    fi
    FEATURES="${FEATURES},virtio_net_test"
fi
if [ "$RELMSG_TEST" = "1" ]; then
    FEATURES="${FEATURES},relmsg_net_test"
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

if [ "$LIVE_UPGRADE_TEST" = "1" ]; then
    FEATURES="${FEATURES},live_upgrade_test"
fi

if [ "$SCHEDULER_TRACE" = "1" ]; then
    if [ -z "$TIMEOUT" ]; then
        echo "error: --scheduler-trace requires --timeout" >&2
        exit 1
    fi
    if [ -n "$GDB" ]; then
        echo "error: --scheduler-trace cannot be combined with --gdb" >&2
        exit 1
    fi
    FEATURES="${FEATURES},scheduler_trace"
fi

if [ "$DEBUG_SNAPSHOT" = "1" ]; then
    if [ -z "$TIMEOUT" ]; then
        echo "error: --debug-snapshot requires --timeout" >&2
        exit 1
    fi
    if [ -n "$GDB" ]; then
        echo "error: --debug-snapshot cannot be combined with --gdb" >&2
        exit 1
    fi
fi

if [ "${CATTEN_SKIP_KERNEL_BUILD:-0}" = "1" ]; then
    if [ ! -f "$KERNEL" ]; then
        echo "error: CATTEN_SKIP_KERNEL_BUILD=1 but ${KERNEL} does not exist" >&2
        exit 1
    fi
    echo ">>> Reusing previously built Catten kernel."
else
    cargo build --package catten --target "$TARGET_SPEC" \
        --no-default-features --features "$FEATURES" $RELEASE_FLAG
fi

if command -v sha256sum >/dev/null 2>&1; then
    KERNEL_SHA256="$(sha256sum "$KERNEL" | awk '{print $1}')"
else
    KERNEL_SHA256="$(shasum -a 256 "$KERNEL" | awk '{print $1}')"
fi
echo ">>> Kernel payload: ${KERNEL}"
echo ">>> Kernel SHA-256: ${KERNEL_SHA256}"

# --- Build a FAT32 EFI System Partition image with mtools. ---
echo ">>> Creating boot image ${IMAGE}..."
mkdir -p "$IMAGE_DIR"
dd if=/dev/zero of="$IMAGE" bs=1048576 count=128 status=none
mformat -i "$IMAGE" -F ::
mmd -i "$IMAGE" ::/EFI
mmd -i "$IMAGE" ::/EFI/BOOT
mcopy -i "$IMAGE" "./limine-binary/${EFI_BOOT_FILE}" "::/EFI/BOOT/${EFI_BOOT_FILE}"
mcopy -i "$IMAGE" "$KERNEL" "::/catten"
mcopy -i "$IMAGE" "./limine.conf" "::/limine.conf"

# --- NVMe persistent disk image ---
NVME_IMAGE="${IMAGE_DIR}/nvme-disk${INSTANCE_SUFFIX}.img"
if [ ! -f "$NVME_IMAGE" ]; then
    echo ">>> Creating persistent NVMe disk image ${NVME_IMAGE} (16 MiB)..."
    dd if=/dev/zero of="$NVME_IMAGE" bs=1048576 count=16 status=none
else
    echo ">>> Reusing existing NVMe disk image ${NVME_IMAGE}"
fi

# --- QEMU options ---
MACHINE="virt,gic-version=3,msi=gicv2m"
if [ "$USE_HVF" != "1" ]; then
    MACHINE="${MACHINE},iommu=smmuv3,default-bus-bypass-iommu=off"
fi
QEMU_OPTS=(
    -M "$MACHINE"
    -m 512M
    -bios "$FIRMWARE"
    -drive "file=${IMAGE},format=raw,if=none,id=boot0"
    -device "virtio-blk-pci,drive=boot0,addr=3"
)

if [ "$USE_HVF" = "1" ]; then
    QEMU_OPTS+=(-accel hvf -cpu host)
else
    QEMU_OPTS+=(-cpu cortex-a710)
fi

QEMU_OPTS+=(-smp "$SMP")

if [ "${CATTEN_QEMU_VIRTIO_TRACE:-0}" = "1" ]; then
    QEMU_OPTS+=(
        -trace enable=virtio_pci_notify_write
    )
fi
if [ -n "${CATTEN_QEMU_TRACE_EVENTS:-}" ]; then
    QEMU_OPTS+=(
        -trace "events=${CATTEN_QEMU_TRACE_EVENTS},file=/tmp/charlotte${INSTANCE_SUFFIX}-qemu-trace.txt"
    )
fi
if [ "${CATTEN_QEMU_MONITOR:-0}" = "1" ]; then
    QEMU_OPTS+=(-monitor "unix:/tmp/charlotte${INSTANCE_SUFFIX}-monitor.sock,server=on,wait=off")
fi

QEMU_OPTS+=(
    -drive "if=none,file=${NVME_IMAGE},format=raw,id=nvme0"
    -device "nvme,drive=nvme0,serial=cat0,max_ioqpairs=4,addr=2"
)

if [ "$NET_TEST" = "1" ]; then
    QEMU_OPTS+=(-nic none)
    case "$NET_BACKEND" in
        user)
            QEMU_OPTS+=(-netdev user,id=charlotte-net)
            ;;
        listen:*)
            NET_PORT="${NET_BACKEND#listen:}"
            QEMU_OPTS+=(-netdev "stream,id=charlotte-net,server=on,addr.type=inet,addr.host=0.0.0.0,addr.port=${NET_PORT}")
            ;;
        connect:*)
            NET_PEER="${NET_BACKEND#connect:}"
            NET_HOST="${NET_PEER%%:*}"
            NET_PORT="${NET_PEER#*:}"
            QEMU_OPTS+=(-netdev "stream,id=charlotte-net,server=off,addr.type=inet,addr.host=${NET_HOST},addr.port=${NET_PORT}")
            ;;
    esac
    QEMU_OPTS+=(
        -device "virtio-net-pci,netdev=charlotte-net,disable-legacy=on,iommu_platform=on,mac=${NET_MAC},addr=1"
    )
fi

if [ "$DISPLAY_MODE" = "1" ]; then
    QEMU_OPTS+=(-device ramfb)
else
    QEMU_OPTS+=(-display none)
fi

if [ -n "$TIMEOUT" ]; then
    LOG="/tmp/charlotte${INSTANCE_SUFFIX}-serial.log"
    : >"$LOG"
    QEMU_OPTS+=(-serial "file:${LOG}")
    echo ">>> Booting under QEMU (${TIMEOUT}s timeout, serial to ${LOG})..."
    if [ "$SCHEDULER_TRACE" = "1" ] || [ "$DEBUG_SNAPSHOT" = "1" ]; then
        QEMU_OPTS+=(-gdb tcp::1234)
    fi
    qemu-system-aarch64 "${QEMU_OPTS[@]}" $GDB &
    QPID=$!
    SELFTEST_COMPLETE=0
    MAX_TICKS=$((TIMEOUT * 10))
    for ((tick = 0; tick < MAX_TICKS; tick++)); do
        sleep 0.1
        if ! kill -0 "$QPID" 2>/dev/null; then
            wait "$QPID" 2>/dev/null || true
            echo "error: QEMU exited before the ${TIMEOUT}s test window elapsed" >&2
            if [ -f "$LOG" ]; then
                echo ">>> Serial log (${LOG}):"
                cat "$LOG"
            fi
            exit 1
        fi
        if grep -Fq "SELFTEST COMPLETE:" "$LOG"; then
            SELFTEST_COMPLETE=1
            if [ "$SCHEDULER_TRACE" = "0" ] && [ "$DEBUG_SNAPSHOT" = "0" ]; then
                echo ">>> Authoritative self-test result observed after $(((tick + 1) / 10))s."
                break
            fi
        fi
    done
    if [ "$SCHEDULER_TRACE" = "1" ]; then
        TRACE_RAW="/tmp/charlotte-scheduler-trace.bin"
        TRACE_TEXT="/tmp/charlotte-scheduler-trace.log"
        read -r TRACE_ADDR TRACE_SIZE < <(nm -S "$KERNEL" | awk '$4 == "DEBUG_TRACE" { print "0x" $1, "0x" $2; exit }')
        if [ -n "${TRACE_ADDR:-}" ] && command -v lldb >/dev/null 2>&1; then
            TRACE_COUNT=$((TRACE_SIZE))
            lldb --batch \
                -o "settings set interpreter.stop-command-source-on-error false" \
                -o "gdb-remote 1234" \
                -o "thread backtrace all" \
                -o "thread select 1" \
                -o "register read esr_el1 far_el1 elr_el1 spsr_el1 sp cpsr" \
                -o "register read cntv_ctl_el0 cntv_cval_el0" \
                -o "thread select 2" \
                -o "register read esr_el1 far_el1 elr_el1 spsr_el1 sp cpsr" \
                -o "register read cntv_ctl_el0 cntv_cval_el0" \
                -o "thread select 3" \
                -o "register read esr_el1 far_el1 elr_el1 spsr_el1 sp cpsr" \
                -o "register read cntv_ctl_el0 cntv_cval_el0" \
                -o "thread select 4" \
                -o "register read esr_el1 far_el1 elr_el1 spsr_el1 sp cpsr" \
                -o "register read cntv_ctl_el0 cntv_cval_el0" \
                -o "memory read --force --binary --size 1 --count ${TRACE_COUNT} --outfile ${TRACE_RAW} ${TRACE_ADDR}" \
                -o "process detach" "$KERNEL" >/tmp/charlotte-trace-lldb.log 2>&1 || true
            if [ -s "$TRACE_RAW" ]; then
                python3 scripts/decode-scheduler-trace.py "$TRACE_RAW" >"$TRACE_TEXT"
                echo ">>> Scheduler trace captured in ${TRACE_TEXT}"
            else
                echo "warning: scheduler trace capture failed; see /tmp/charlotte-trace-lldb.log" >&2
            fi
        else
            echo "warning: DEBUG_TRACE symbol or lldb unavailable; scheduler trace not captured" >&2
        fi
    elif [ "$DEBUG_SNAPSHOT" = "1" ]; then
        if command -v lldb >/dev/null 2>&1; then
            TIMER_DIAG_ADDR="$(nm "$KERNEL" | awk '$3 == "TIMER_DIAGNOSTICS" && !found { print "0x" $1; found=1 }')"
            WAKER_DIAG_ADDR="$(nm "$KERNEL" | awk '$3 == "WAKER_DIAGNOSTICS" && !found { print "0x" $1; found=1 }')"
            LIFECYCLE_PROGRESS_ADDR="$(nm "$KERNEL" | awk '$3 == "SCHEDULER_LIFECYCLE_PROGRESS" && !found { print "0x" $1; found=1 }')"
            SCHEDULER_LP_DIAG_ADDR="$(nm "$KERNEL" | awk '$3 == "SCHEDULER_LP_DIAGNOSTICS" && !found { print "0x" $1; found=1 }')"
            lldb --batch \
                -o "settings set interpreter.stop-command-source-on-error false" \
                -o "gdb-remote 1234" \
                -o "thread backtrace all" \
                -o "thread select 1" \
                -o "register read esr_el1 far_el1 elr_el1 spsr_el1 sp cpsr" \
                -o "register read cntv_ctl_el0 cntv_cval_el0" \
                -o "thread select 2" \
                -o "register read esr_el1 far_el1 elr_el1 spsr_el1 sp cpsr" \
                -o "register read cntv_ctl_el0 cntv_cval_el0" \
                -o "thread select 3" \
                -o "register read esr_el1 far_el1 elr_el1 spsr_el1 sp cpsr" \
                -o "register read cntv_ctl_el0 cntv_cval_el0" \
                -o "thread select 4" \
                -o "register read esr_el1 far_el1 elr_el1 spsr_el1 sp cpsr" \
                -o "register read cntv_ctl_el0 cntv_cval_el0" \
                -o "memory read --force --format x --size 8 --count 32 ${TIMER_DIAG_ADDR}" \
                -o "memory read --force --format x --size 8 --count 3 ${WAKER_DIAG_ADDR}" \
                -o "memory read --force --format x --size 8 --count 1 ${LIFECYCLE_PROGRESS_ADDR}" \
                -o "memory read --force --format x --size 8 --count 24 ${SCHEDULER_LP_DIAG_ADDR}" \
                -o "process detach" "$KERNEL" >/tmp/charlotte-debug-snapshot-lldb.log 2>&1 || true
            echo ">>> Debug snapshot captured in /tmp/charlotte-debug-snapshot-lldb.log"
        else
            echo "warning: lldb unavailable; debug snapshot not captured" >&2
        fi
    fi
    kill "$QPID" 2>/dev/null || true
    wait "$QPID" 2>/dev/null || true
    echo ">>> Serial log (${LOG}):"
    cat "$LOG"
    if [ "$SELFTEST_COMPLETE" -ne 1 ]; then
        echo "error: authoritative self-test result was not produced within ${TIMEOUT}s" >&2
        exit 1
    fi
    if ! grep -Eq \
        'SELFTEST COMPLETE: passed=[0-9]+ failed=0 pending=0 passed_bitmap=0x[0-9a-f]+ failed_bitmap=0x0 pending_bitmap=0x0' \
        "$LOG"
    then
        echo "error: malformed or unsuccessful authoritative self-test result" >&2
        grep -E 'SELFTEST (FAILED|PENDING):' "$LOG" >&2 || true
        exit 1
    fi
    echo ">>> All registered deferred self-tests passed."
else
    QEMU_OPTS+=(-serial stdio)
    if [ "$DISPLAY_MODE" = "1" ]; then
        echo ">>> Booting under QEMU (framebuffer window + serial; Ctrl-A X to quit)..."
    else
        echo ">>> Booting under QEMU (serial on stdio; press Ctrl-A X to quit)..."
    fi
    exec qemu-system-aarch64 "${QEMU_OPTS[@]}" $GDB
fi
