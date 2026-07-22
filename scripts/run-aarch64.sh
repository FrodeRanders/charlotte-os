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
#   scripts/run-aarch64.sh [debug|release] [--clean] [--display] [--gdb] [--debug-snapshot] [--scheduler-trace] [--hvf] [--net-test] [--smp N] [--timeout S]
#
#   debug|release  Build profile (default: debug)
#   --clean        Remove all cached AArch64 target artifacts before building
#   --display      Build with framebuffer console (flanterm), boot with ramfb
#   --gdb          Start QEMU paused with gdb stub on tcp::1234
#   --debug-snapshot  Capture all-LP stacks/registers at timeout without enabling tracing
#   --scheduler-trace  Capture and decode the in-memory scheduler trace at timeout
#   --hvf          Use Apple Hypervisor.Framework acceleration (macOS only)
#   --net-test     Build the KVM-only virtio-net test (requires separately configured matching PCI hardware)
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
SMP="4"
TIMEOUT=""
CLEAN_BUILD="0"
SCHEDULER_TRACE="0"
DEBUG_SNAPSHOT="0"

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
        --smp)
            [ "$#" -ge 2 ] || { echo "Missing value for --smp" >&2; exit 1; }
            SMP="$2"; shift 2 ;;
        --timeout)
            [ "$#" -ge 2 ] || { echo "Missing value for --timeout" >&2; exit 1; }
            TIMEOUT="$2"; shift 2 ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

TARGET_SPEC="target_specs/${ARCH}-unknown-none-catten.json"
TARGET_DIR="${ARCH}-unknown-none-catten"
IMAGE_DIR="./os-images"
IMAGE="${IMAGE_DIR}/charlotte-${ARCH}-${PROFILE}.img"
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

if [ "$CLEAN_BUILD" = "1" ]; then
    echo ">>> Cleaning cached ${ARCH} kernel and dependency artifacts..."
    cargo clean --target "$TARGET_SPEC"
    echo ">>> Cleaning and rebuilding embedded EL0 service bundle..."
    "${ROOT_DIR}/scripts/build-catten-services.sh" --embed --clean
else
    echo ">>> Rebuilding embedded EL0 service bundle..."
    "${ROOT_DIR}/scripts/build-catten-services.sh" --embed
fi

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

cargo build --package catten --target "$TARGET_SPEC" \
    --no-default-features --features "$FEATURES" $RELEASE_FLAG

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
dd if=/dev/zero of="$IMAGE" bs=1m count=128 status=none
mformat -i "$IMAGE" -F ::
mmd -i "$IMAGE" ::/EFI
mmd -i "$IMAGE" ::/EFI/BOOT
mcopy -i "$IMAGE" "./limine-binary/${EFI_BOOT_FILE}" "::/EFI/BOOT/${EFI_BOOT_FILE}"
mcopy -i "$IMAGE" "$KERNEL" "::/catten"
mcopy -i "$IMAGE" "./limine.conf" "::/limine.conf"

# --- QEMU options ---
QEMU_OPTS=(
    -M virt,gic-version=3
    -m 512M
    -bios "$FIRMWARE"
    -drive file="$IMAGE",format=raw,if=virtio
)

if [ "$USE_HVF" = "1" ]; then
    QEMU_OPTS+=(-accel hvf -cpu host)
else
    QEMU_OPTS+=(-cpu cortex-a710)
fi

QEMU_OPTS+=(-smp "$SMP")

if [ "$DISPLAY_MODE" = "1" ]; then
    QEMU_OPTS+=(-device ramfb)
else
    QEMU_OPTS+=(-display none)
fi

if [ -n "$TIMEOUT" ]; then
    LOG="/tmp/charlotte-serial.log"
    QEMU_OPTS+=(-serial "file:${LOG}")
    echo ">>> Booting under QEMU (${TIMEOUT}s timeout, serial to ${LOG})..."
    if [ "$SCHEDULER_TRACE" = "1" ] || [ "$DEBUG_SNAPSHOT" = "1" ]; then
        QEMU_OPTS+=(-gdb tcp::1234)
    fi
    qemu-system-aarch64 "${QEMU_OPTS[@]}" $GDB &
    QPID=$!
    sleep "$TIMEOUT"
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
    REQUIRED_MARKERS=(
        "[EL0] SUCCESS:"
        "[raft] SUCCESS:"
        "[EL0 IPC] SUCCESS:"
        "[EL0 IPC block] SUCCESS:"
        "[EL0 IPC cross-AS] SUCCESS:"
        "[EL0 IPC memory] SUCCESS:"
        "[EL0 IPC memory cancel] SUCCESS:"
        "[EL0 IPC memory copy] SUCCESS:"
        "[EL0 xLP] SUCCESS:"
        "[PP] SUCCESS:"
        "[sitas] SUCCESS:"
        "[service] SUCCESS:"
        "[cq wait] SUCCESS:"
        "[device] SUCCESS:"
        "[uart] SUCCESS:"
        "[scheduler lifecycle] SUCCESS:"
    )
    missing=0
    for marker in "${REQUIRED_MARKERS[@]}"; do
        if ! grep -Fq "$marker" "$LOG"; then
            echo "error: deferred self-test marker missing: ${marker}" >&2
            missing=1
        fi
    done
    if [ "$missing" -ne 0 ]; then
        exit 1
    fi
    echo ">>> All required deferred self-test markers observed."
else
    QEMU_OPTS+=(-serial stdio)
    if [ "$DISPLAY_MODE" = "1" ]; then
        echo ">>> Booting under QEMU (framebuffer window + serial; Ctrl-A X to quit)..."
    else
        echo ">>> Booting under QEMU (serial on stdio; press Ctrl-A X to quit)..."
    fi
    exec qemu-system-aarch64 "${QEMU_OPTS[@]}" $GDB
fi
