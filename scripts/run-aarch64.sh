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
#   scripts/run-aarch64.sh [debug|release] [--clean] [--display] [--gdb] [--gdb-port PORT] [--debug-snapshot] [--scheduler-trace] [--hvf] [--no-network] [--net-test|--relmsg-test|--disco-test|--dhcp-test|--s3-test|--deployment-ingress-test|--kafka-test|--kafka-coordinator-test|--kafka-fencing-test] [--net-listen PORT|--net-connect HOST:PORT] [--instance NAME] [--mac ADDRESS] [--live-upgrade-test|--shutdown-test] [--smp N] [--timeout S] [--fresh-storage|--reuse-storage]
#
#   debug|release  Build profile (default: debug)
#   --clean        Remove all cached AArch64 target artifacts before building
#   --display      Build with framebuffer console (flanterm), boot with ramfb
#   --sbsa-ref     Boot the QEMU sbsa-ref machine with the TF-A + edk2 firmware
#                  built by scripts/build-sbsa-firmware.sh (SBSA_FLASH0/1.fd)
#   --gdb          Start QEMU paused with a gdb stub
#   --gdb-port PORT  GDB stub port (default: 1234)
#   --debug-snapshot  Capture all-LP stacks/registers at timeout without enabling tracing
#   --scheduler-trace  Capture and decode the in-memory scheduler trace at timeout
#   --hvf          Use Apple Hypervisor.Framework acceleration (macOS only)
#   --no-network   Do not attach a NIC or launch network-backed services
#   --net-test     Verify the default virtio-net capability under TCG/KVM
#   --relmsg-test  Exchange reliable messages with a second socket-LAN guest
#   --disco-test   Run the cluster discovery test (implies --net-test)
#   --deploy-test  Run the cluster-deployment test (implies --dns-test):
#               deploys a signed artifact to the peer node, executes it across
#               the network, migrates it between nodes, and verifies
#               cooperative plus zero-grace forced retirement.
#   --dns-test     Run the distributed name service test (Raft over the
#               network; both guests must run it, implies --disco-test)
#   --tcpip-test  Run the TCP/IP test: smoltcp adapter over the frouter,
#               exchanging TCP data between two guests (both guests must
#               run it, implies --net-test)
#   --http-test   Run the HTTP keyhole test: a hardcoded HTTP server on the
#               guest's tcpip stack serving observable state, reached from
#               the host via SLIRP hostfwd (single guest, user network)
#   --dhcp-test   Verify that the default DHCP client acquires a lease
#   --s3-test     Start a TLS RustFS Docker fixture and verify S3
#                 PUT/HEAD/GET/DELETE from a CharlotteOS application
#   --deployment-ingress-test  Use that RustFS fixture to verify the complete
#                 signed upload/notify/pull/launch/readiness deployment path
#   --kafka-test  Start a TLS/mTLS/SCRAM Apache Kafka Docker fixture and verify
#                 idempotent produce, read-committed consume, transactions,
#                 and recovery after the active route leader is killed
#   --kafka-coordinator-test  Run the Kafka fixture while hard-stopping the
#                 transaction coordinator discovered by the guest
#   --kafka-fencing-test  Launch two connectors with the same transactional
#                 identity and require the first to report producer fencing
#   --net-listen PORT  Put the guest NIC on a QEMU socket LAN and listen
#   --net-connect HOST:PORT  Connect the guest NIC to a QEMU socket LAN
#   --instance NAME  Use separate boot/NVMe/log files for this VM
#   --mac ADDRESS  Set the guest NIC MAC address
#   --live-upgrade-test  Run the isolated EL0 service lifecycle/upgrade integration test
#   --shutdown-test  Run the isolated cooperative/forced domain-shutdown test
#   --smp N        Number of CPUs (default: 4)
#   --timeout S    Kill QEMU after S seconds, capturing serial output (default: run interactively)
#   --fresh-storage  Recreate this instance's NVMe store from the blessed bundle
#   --reuse-storage  Keep it even when the blessed bundle changed (explicitly stale)
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=lib/boot-common.sh
source "${SCRIPT_DIR}/lib/boot-common.sh"

ARCH="aarch64"
PROFILE="debug"
GDB=""
GDB_PORT="1234"
DISPLAY_MODE="0"
USE_HVF="0"
NETWORK="1"
NET_TEST="0"
RELMSG_TEST="0"
DISCO_TEST="0"
DNS_TEST="0"
DEPLOY_TEST="0"
TCPIP_TEST="0"
HTTP_TEST="0"
DHCP_TEST="0"
S3_TEST="0"
DEPLOYMENT_INGRESS_TEST="0"
KAFKA_TEST="0"
KAFKA_COORDINATOR_TEST="0"
KAFKA_FENCING_TEST="0"
HTTP_HOST_PORT="${CATTEN_HTTP_HOST_PORT:-8080}"
DEPLOY_HOST_PORT="${CATTEN_DEPLOY_HOST_PORT:-8081}"
LIVE_UPGRADE_TEST="0"
SHUTDOWN_TEST="0"
SMP="4"
TIMEOUT=""
CLEAN_BUILD="0"
SCHEDULER_TRACE="0"
DEBUG_SNAPSHOT="0"
SBSA_REF="0"
FRESH_STORAGE="0"
REUSE_STORAGE="0"
INSTANCE=""
NET_BACKEND="user"
NET_MAC="52:54:00:12:34:56"

while [ "$#" -gt 0 ]; do
    case "$1" in
        debug|release) PROFILE="$1"; shift ;;
        --clean)       CLEAN_BUILD="1"; shift ;;
        --display)     DISPLAY_MODE="1"; shift ;;
        --sbsa-ref)    SBSA_REF="1"; shift ;;
        --gdb)         GDB="-S"; shift ;;
        --gdb-port)
            [ "$#" -ge 2 ] || { echo "Missing value for --gdb-port" >&2; exit 1; }
            GDB_PORT="$2"; shift 2 ;;
        --debug-snapshot) DEBUG_SNAPSHOT="1"; shift ;;
        --scheduler-trace) SCHEDULER_TRACE="1"; shift ;;
        --hvf)         USE_HVF="1"; shift ;;
        --no-network)  NETWORK="0"; shift ;;
        --net-test)    NET_TEST="1"; shift ;;
        --relmsg-test) NET_TEST="1"; RELMSG_TEST="1"; shift ;;
        --disco-test)  NET_TEST="1"; DISCO_TEST="1"; shift ;; # implies --net-test
        --dns-test)    NET_TEST="1"; DISCO_TEST="1"; DNS_TEST="1"; shift ;; # implies --disco-test
        --deploy-test)  NET_TEST="1"; DISCO_TEST="1"; DNS_TEST="1"; DEPLOY_TEST="1"; shift ;; # implies --dns-test
        --tcpip-test)  NET_TEST="1"; TCPIP_TEST="1"; shift ;; # implies --net-test
        --http-test)   NET_TEST="1"; HTTP_TEST="1"; shift ;; # implies --net-test
        --dhcp-test)   NET_TEST="1"; DHCP_TEST="1"; shift ;; # includes the driver verifier
        --s3-test)     NET_TEST="1"; DHCP_TEST="1"; S3_TEST="1"; shift ;;
        --deployment-ingress-test) NET_TEST="1"; DHCP_TEST="1"; S3_TEST="1"; DEPLOYMENT_INGRESS_TEST="1"; shift ;;
        --kafka-test)  NET_TEST="1"; DHCP_TEST="1"; KAFKA_TEST="1"; shift ;;
        --kafka-coordinator-test) NET_TEST="1"; DHCP_TEST="1"; KAFKA_TEST="1"; KAFKA_COORDINATOR_TEST="1"; shift ;;
        --kafka-fencing-test) NET_TEST="1"; DHCP_TEST="1"; KAFKA_TEST="1"; KAFKA_FENCING_TEST="1"; shift ;;
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
        --shutdown-test) SHUTDOWN_TEST="1"; shift ;;
        --smp)
            [ "$#" -ge 2 ] || { echo "Missing value for --smp" >&2; exit 1; }
            SMP="$2"; shift 2 ;;
        --timeout)
            [ "$#" -ge 2 ] || { echo "Missing value for --timeout" >&2; exit 1; }
            TIMEOUT="$2"; shift 2 ;;
        --fresh-storage) FRESH_STORAGE="1"; shift ;;
        --reuse-storage) REUSE_STORAGE="1"; shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

catten_boot_validate_port "--gdb-port" "$GDB_PORT"
catten_boot_validate_port "CATTEN_HTTP_HOST_PORT" "$HTTP_HOST_PORT"
catten_boot_validate_port "CATTEN_DEPLOY_HOST_PORT" "$DEPLOY_HOST_PORT"
catten_boot_validate_instance "$INSTANCE"
catten_boot_validate_positive_integer "--smp" "$SMP"
if [ -n "$TIMEOUT" ]; then
    catten_boot_validate_positive_integer "--timeout" "$TIMEOUT"
fi
if [ "$FRESH_STORAGE" = "1" ] && [ "$REUSE_STORAGE" = "1" ]; then
    echo "error: --fresh-storage and --reuse-storage are mutually exclusive" >&2
    exit 1
fi
if [ "$NET_BACKEND" != "user" ] && [ "$NETWORK" != "1" ]; then
    echo "error: socket networking is incompatible with --no-network" >&2
    exit 1
fi
if [ "$NETWORK" != "1" ] && { [ "$NET_TEST" = "1" ] || [ "$RELMSG_TEST" = "1" ] \
    || [ "$DISCO_TEST" = "1" ] || [ "$DNS_TEST" = "1" ] || [ "$DEPLOY_TEST" = "1" ] \
    || [ "$TCPIP_TEST" = "1" ] || [ "$HTTP_TEST" = "1" ] || [ "$DHCP_TEST" = "1" ] \
    || [ "$S3_TEST" = "1" ] || [ "$DEPLOYMENT_INGRESS_TEST" = "1" ] \
    || [ "$KAFKA_TEST" = "1" ]; }; then
    echo "error: network verification options are incompatible with --no-network" >&2
    exit 1
fi
if [ "$NET_BACKEND" != "user" ] && [ "${CODEX_SANDBOX_NETWORK_DISABLED:-0}" = "1" ] \
    && [ "${CATTEN_ALLOW_SANDBOX_NETWORK:-0}" != "1" ]; then
    echo "error: two-QEMU socket networking is unavailable in this sandbox" >&2
    echo "       CODEX_SANDBOX_NETWORK_DISABLED=1 prevents the listener/connect path." >&2
    echo "       Re-run outside the sandbox (or with network permission)." >&2
    echo "       Set CATTEN_ALLOW_SANDBOX_NETWORK=1 only if the sandbox is known to allow it." >&2
    exit 1
fi
if [ "$NET_BACKEND" != "user" ] && [ -n "${CODEX_SANDBOX:-}" ] \
    && [ "${CATTEN_ALLOW_SANDBOX_NETWORK:-0}" != "1" ]; then
    echo "warning: running a two-QEMU network test inside sandbox '${CODEX_SANDBOX}'" >&2
    echo "         If discovery receives no frames, retry outside the sandbox." >&2
fi
if [ "$RELMSG_TEST" = "1" ] && [ "$NET_BACKEND" = "user" ]; then
    echo "error: --relmsg-test requires --net-listen or --net-connect" >&2
    exit 1
fi
if [ "$TCPIP_TEST" = "1" ] && [ "$NET_BACKEND" = "user" ]; then
    echo "error: --tcpip-test requires --net-listen or --net-connect" >&2
    exit 1
fi
if [ "$SBSA_REF" = "1" ] && [ "$USE_HVF" = "1" ]; then
    echo "error: --sbsa-ref is incompatible with --hvf (the sbsa-ref machine needs TCG)" >&2
    exit 1
fi
if [ "$LIVE_UPGRADE_TEST" = "1" ] && [ "$USE_HVF" = "1" ]; then
    echo "error: --live-upgrade-test requires the protected-DMA object store and is incompatible with --hvf" >&2
    exit 1
fi
if [ "$NETWORK" = "1" ] && [ "$USE_HVF" = "1" ]; then
    echo "error: default networking is incompatible with --hvf (EL0 MMIO is unsupported); pass --no-network" >&2
    exit 1
fi
if [ "$SBSA_REF" = "1" ] && [ "$NETWORK" = "1" ]; then
    echo "error: --sbsa-ref does not yet support the default network device; pass --no-network" >&2
    exit 1
fi
if [ "$HTTP_TEST" = "1" ] && [ "$NET_BACKEND" != "user" ]; then
    echo "error: --http-test requires the default user network (hostfwd)" >&2
    exit 1
fi
if [ "$S3_TEST" = "1" ] && { [ "$NET_BACKEND" != "user" ] || [ -z "$TIMEOUT" ]; }; then
    echo "error: --s3-test requires the default user network and --timeout" >&2
    exit 1
fi
if [ "$KAFKA_TEST" = "1" ] && { [ "$NET_BACKEND" != "user" ] || [ -z "$TIMEOUT" ]; }; then
    echo "error: --kafka-test requires the default user network and --timeout" >&2
    exit 1
fi

cd "$ROOT_DIR"
catten_boot_init "$ROOT_DIR"
catten_boot_require_commands mformat mmd mcopy
LIMINE_CONFIG="$CATTEN_BOOT_LIMINE_CONFIG"

# A root Cargo clean can remove arbitrary generated files below `target/`, not
# only Rust artifacts. Do it before creating test certificates whose paths are
# consumed later by kernel `include_bytes!` expressions. This ordering matters
# for both --s3-test and --kafka-test.
if [ "${CATTEN_SKIP_EMBED_BUILD:-0}" != "1" ] && [ "$CLEAN_BUILD" = "1" ]; then
    echo ">>> Cleaning cached ${ARCH} kernel and dependency artifacts..."
    cargo clean --target "target_specs/${ARCH}-unknown-none-catten.json"
fi

RUSTFS_COMPOSE="${ROOT_DIR}/docker/rustfs-s3-test/compose.yaml"
RUSTFS_RUNNING="0"
DEPLOYMENT_WORKER_PID=""
KAFKA_COMPOSE="${ROOT_DIR}/docker/kafka-test/compose.yaml"
KAFKA_RUNNING="0"
cleanup_fixtures() {
    if [ -n "$DEPLOYMENT_WORKER_PID" ]; then
        kill "$DEPLOYMENT_WORKER_PID" >/dev/null 2>&1 || true
    fi
    if [ "$RUSTFS_RUNNING" = "1" ]; then
        docker compose -f "$RUSTFS_COMPOSE" down --volumes --remove-orphans >/dev/null 2>&1 || true
    fi
    if [ "$KAFKA_RUNNING" = "1" ]; then
        docker compose -f "$KAFKA_COMPOSE" down --volumes --remove-orphans >/dev/null 2>&1 || true
    fi
}
trap cleanup_fixtures EXIT

if [ "$S3_TEST" = "1" ]; then
    catten_boot_require_commands docker openssl
    RUSTFS_TEST_DIR="${ROOT_DIR}/target/rustfs-s3-test"
    export CATTEN_RUSTFS_CERT_DIR="${RUSTFS_TEST_DIR}/certs"
    export CATTEN_RUSTFS_PORT="19000"
    mkdir -p "$CATTEN_RUSTFS_CERT_DIR"
    openssl ecparam -name prime256v1 -genkey -noout \
        -out "$CATTEN_RUSTFS_CERT_DIR/ca.key"
    openssl req -x509 -new -sha256 -days 2 \
        -key "$CATTEN_RUSTFS_CERT_DIR/ca.key" \
        -subj "/CN=CharlotteOS RustFS test CA" \
        -addext "basicConstraints=critical,CA:TRUE" \
        -addext "keyUsage=critical,keyCertSign,cRLSign" \
        -out "$CATTEN_RUSTFS_CERT_DIR/ca.crt"
    openssl ecparam -name prime256v1 -genkey -noout \
        -out "$CATTEN_RUSTFS_CERT_DIR/rustfs_key.pem"
    openssl req -new -sha256 \
        -key "$CATTEN_RUSTFS_CERT_DIR/rustfs_key.pem" \
        -subj "/CN=rustfs.test" \
        -out "$CATTEN_RUSTFS_CERT_DIR/rustfs.csr"
    openssl x509 -req -sha256 -days 2 \
        -in "$CATTEN_RUSTFS_CERT_DIR/rustfs.csr" \
        -CA "$CATTEN_RUSTFS_CERT_DIR/ca.crt" \
        -CAkey "$CATTEN_RUSTFS_CERT_DIR/ca.key" \
        -CAcreateserial \
        -extfile "${ROOT_DIR}/docker/rustfs-s3-test/server-ext.cnf" \
        -out "$CATTEN_RUSTFS_CERT_DIR/rustfs_cert.pem"
    openssl x509 -in "$CATTEN_RUSTFS_CERT_DIR/ca.crt" -outform DER \
        -out "$CATTEN_RUSTFS_CERT_DIR/ca.der"
    chmod 0644 "$CATTEN_RUSTFS_CERT_DIR/ca.crt" \
        "$CATTEN_RUSTFS_CERT_DIR/ca.der" \
        "$CATTEN_RUSTFS_CERT_DIR/rustfs_cert.pem" \
        "$CATTEN_RUSTFS_CERT_DIR/rustfs_key.pem"
    export CATTEN_S3_TEST_CA_DER="$CATTEN_RUSTFS_CERT_DIR/ca.der"
    echo ">>> Starting ephemeral TLS RustFS fixture on host port 19000..."
    docker compose -f "$RUSTFS_COMPOSE" down --volumes --remove-orphans >/dev/null 2>&1 || true
    RUSTFS_RUNNING="1"
    docker compose -f "$RUSTFS_COMPOSE" up -d --wait rustfs
    docker compose -f "$RUSTFS_COMPOSE" run --rm init
fi

if [ "$KAFKA_TEST" = "1" ]; then
    catten_boot_require_commands docker keytool openssl
    KAFKA_TEST_DIR="${ROOT_DIR}/target/kafka-test"
    export CATTEN_KAFKA_CERT_DIR="${KAFKA_TEST_DIR}/certs"
    export CATTEN_KAFKA_PORT="19092"
    export CATTEN_KAFKA_PORT_2="19094"
    export CATTEN_KAFKA_PORT_3="19096"
    mkdir -p "$CATTEN_KAFKA_CERT_DIR"
    openssl ecparam -name prime256v1 -genkey -noout \
        -out "$CATTEN_KAFKA_CERT_DIR/ca.key"
    openssl req -x509 -new -sha256 -days 2 \
        -key "$CATTEN_KAFKA_CERT_DIR/ca.key" \
        -subj "/CN=CharlotteOS Kafka test CA" \
        -addext "basicConstraints=critical,CA:TRUE" \
        -addext "keyUsage=critical,keyCertSign,cRLSign" \
        -out "$CATTEN_KAFKA_CERT_DIR/ca.crt"
    openssl ecparam -name prime256v1 -genkey -noout \
        -out "$CATTEN_KAFKA_CERT_DIR/kafka.key"
    openssl req -new -sha256 \
        -key "$CATTEN_KAFKA_CERT_DIR/kafka.key" \
        -subj "/CN=kafka-1.test" \
        -out "$CATTEN_KAFKA_CERT_DIR/kafka.csr"
    openssl x509 -req -sha256 -days 2 \
        -in "$CATTEN_KAFKA_CERT_DIR/kafka.csr" \
        -CA "$CATTEN_KAFKA_CERT_DIR/ca.crt" \
        -CAkey "$CATTEN_KAFKA_CERT_DIR/ca.key" \
        -CAcreateserial \
        -extfile "${ROOT_DIR}/docker/kafka-test/server-ext.cnf" \
        -out "$CATTEN_KAFKA_CERT_DIR/kafka.crt"
    openssl pkcs12 -export \
        -name kafka \
        -inkey "$CATTEN_KAFKA_CERT_DIR/kafka.key" \
        -in "$CATTEN_KAFKA_CERT_DIR/kafka.crt" \
        -certfile "$CATTEN_KAFKA_CERT_DIR/ca.crt" \
        -out "$CATTEN_KAFKA_CERT_DIR/kafka.p12" \
        -passout pass:charlotte-kafka-test
    openssl x509 -in "$CATTEN_KAFKA_CERT_DIR/ca.crt" -outform DER \
        -out "$CATTEN_KAFKA_CERT_DIR/ca.der"
    openssl ecparam -name prime256v1 -genkey -noout \
        -out "$CATTEN_KAFKA_CERT_DIR/client.key"
    openssl req -new -sha256 \
        -key "$CATTEN_KAFKA_CERT_DIR/client.key" \
        -subj "/CN=charlotte" \
        -out "$CATTEN_KAFKA_CERT_DIR/client.csr"
    openssl x509 -req -sha256 -days 2 \
        -in "$CATTEN_KAFKA_CERT_DIR/client.csr" \
        -CA "$CATTEN_KAFKA_CERT_DIR/ca.crt" \
        -CAkey "$CATTEN_KAFKA_CERT_DIR/ca.key" \
        -CAserial "$CATTEN_KAFKA_CERT_DIR/ca.srl" \
        -extfile "${ROOT_DIR}/docker/kafka-test/client-ext.cnf" \
        -out "$CATTEN_KAFKA_CERT_DIR/client.crt"
    openssl x509 -in "$CATTEN_KAFKA_CERT_DIR/client.crt" -outform DER \
        -out "$CATTEN_KAFKA_CERT_DIR/client.der"
    openssl ec -in "$CATTEN_KAFKA_CERT_DIR/client.key" -outform DER \
        -out "$CATTEN_KAFKA_CERT_DIR/client-key.der"
    rm -f "$CATTEN_KAFKA_CERT_DIR/kafka-truststore.p12"
    keytool -importcert -noprompt \
        -alias charlotte-test-ca \
        -file "$CATTEN_KAFKA_CERT_DIR/ca.crt" \
        -keystore "$CATTEN_KAFKA_CERT_DIR/kafka-truststore.p12" \
        -storetype PKCS12 \
        -storepass charlotte-kafka-test
    printf '%s\n' 'charlotte-kafka-test' >"$CATTEN_KAFKA_CERT_DIR/key-password"
    printf '%s\n' 'charlotte-kafka-test' >"$CATTEN_KAFKA_CERT_DIR/keystore-password"
    printf '%s\n' 'charlotte-kafka-test' >"$CATTEN_KAFKA_CERT_DIR/truststore-password"
    printf '%s\n' \
        'KafkaServer {' \
        '  org.apache.kafka.common.security.scram.ScramLoginModule required' \
        '  username="charlotte"' \
        '  password="charlotte-kafka-test";' \
        '};' >"$CATTEN_KAFKA_CERT_DIR/kafka_server_jaas.conf"
    chmod 0600 "$CATTEN_KAFKA_CERT_DIR/client-key.der"
    chmod 0644 "$CATTEN_KAFKA_CERT_DIR/ca.der" \
        "$CATTEN_KAFKA_CERT_DIR/kafka.p12" \
        "$CATTEN_KAFKA_CERT_DIR/kafka-truststore.p12" \
        "$CATTEN_KAFKA_CERT_DIR/client.der" \
        "$CATTEN_KAFKA_CERT_DIR/key-password" \
        "$CATTEN_KAFKA_CERT_DIR/keystore-password" \
        "$CATTEN_KAFKA_CERT_DIR/truststore-password" \
        "$CATTEN_KAFKA_CERT_DIR/kafka_server_jaas.conf"
    export CATTEN_KAFKA_TEST_CA_DER="$CATTEN_KAFKA_CERT_DIR/ca.der"
    export CATTEN_KAFKA_TEST_CLIENT_CERT_DER="$CATTEN_KAFKA_CERT_DIR/client.der"
    export CATTEN_KAFKA_TEST_CLIENT_KEY_DER="$CATTEN_KAFKA_CERT_DIR/client-key.der"
    echo ">>> Starting ephemeral three-broker TLS/mTLS/SCRAM Apache Kafka fixture..."
    docker compose -f "$KAFKA_COMPOSE" down --volumes --remove-orphans >/dev/null 2>&1 || true
    KAFKA_RUNNING="1"
    docker compose -f "$KAFKA_COMPOSE" up -d --wait kafka1 kafka2 kafka3
    docker compose -f "$KAFKA_COMPOSE" exec -T kafka1 \
        /opt/kafka/bin/kafka-configs.sh \
        --bootstrap-server localhost:29092 \
        --alter \
        --add-config 'SCRAM-SHA-256=[iterations=4096,password=charlotte-kafka-test]' \
        --entity-type users \
        --entity-name charlotte
    KAFKA_EVENTS_REPLICAS="1:2:3"
    KAFKA_RESULTS_REPLICAS="2:3:1"
    docker compose -f "$KAFKA_COMPOSE" exec -T kafka1 \
        /opt/kafka/bin/kafka-topics.sh \
        --bootstrap-server localhost:29092 \
        --create --if-not-exists \
        --topic charlotte-events --replica-assignment "$KAFKA_EVENTS_REPLICAS"
    docker compose -f "$KAFKA_COMPOSE" exec -T kafka1 \
        /opt/kafka/bin/kafka-topics.sh \
        --bootstrap-server localhost:29092 \
        --create --if-not-exists \
        --topic charlotte-results --replica-assignment "$KAFKA_RESULTS_REPLICAS"
fi

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

DEPLOYMENT_DESCRIPTOR=""
DEPLOYMENT_RELEASE=""
CLUSTER_SIGN_BIN="${ROOT_DIR}/target/debug/cluster-sign"
if [ "$DEPLOYMENT_INGRESS_TEST" = "1" ]; then
    echo ">>> Preparing signed central-store deployment fixture..."
    (cd /tmp && cargo build --quiet --manifest-path "${ROOT_DIR}/tools/cluster-sign/Cargo.toml")
    DEPLOYMENT_TEST_DIR="${ROOT_DIR}/target/deployment-ingress-test"
    mkdir -p "$DEPLOYMENT_TEST_DIR"
    DEPLOYMENT_ELF="${CATTEN_AARCH64_SERVICE_BUNDLE}/greet.elf"
    DEPLOYMENT_DESCRIPTOR="${DEPLOYMENT_TEST_DIR}/greet.cdep"
    DEPLOYMENT_RELEASE="${DEPLOYMENT_TEST_DIR}/greet.crelease"
    DEPLOYMENT_OBJECT_KEY="deployments/greet-e2e.elf"
    DEPLOYMENT_DIGEST="$($CLUSTER_SIGN_BIN sha256 "$DEPLOYMENT_ELF")"
    if [ -n "${CLUSTER_SIGN_PRIVATE_KEY:-}" ]; then
        DEPLOYMENT_PRIVATE_KEY="$CLUSTER_SIGN_PRIVATE_KEY"
    else
        DEPLOYMENT_PRIVATE_KEY="$(grep -v '^#' "${ROOT_DIR}/tools/cluster-sign/dev-key.hex" | tr -d '[:space:]')"
    fi
    DEPLOYMENT_SEQUENCE="$(date +%s)"
    docker compose -f "$RUSTFS_COMPOSE" run --rm --no-deps \
        -v "${DEPLOYMENT_ELF}:/tmp/greet.elf:ro" \
        --entrypoint /bin/sh init -ec \
        'rc alias set local https://rustfs.test:9000 charlotte-test-access charlotte-test-secret-2026 && rc cp /tmp/greet.elf local/charlotte-test/deployments/greet-e2e.elf'
    "$CLUSTER_SIGN_BIN" deployment-sign \
        "$DEPLOYMENT_DESCRIPTOR" greet "$DEPLOYMENT_OBJECT_KEY" "$DEPLOYMENT_DIGEST" \
        0 "$DEPLOYMENT_SEQUENCE" 4 1 5000 "$DEPLOYMENT_PRIVATE_KEY" greet=publish
    "$CLUSTER_SIGN_BIN" release-sign \
        "$DEPLOYMENT_RELEASE" deployment-ingress-e2e "$DEPLOYMENT_SEQUENCE" \
        "$DEPLOYMENT_PRIVATE_KEY" "$DEPLOYMENT_DESCRIPTOR"
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
if [ "$DEPLOY_TEST" = "1" ]; then
    FEATURES="${FEATURES},deploy_net_test"
    FEATURES="${FEATURES},clusterctl_test"
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
if [ "$S3_TEST" = "1" ]; then
    FEATURES="${FEATURES},s3_test"
fi
if [ "$KAFKA_TEST" = "1" ]; then
    FEATURES="${FEATURES},kafka_test"
fi
if [ "$KAFKA_COORDINATOR_TEST" = "1" ]; then
    FEATURES="${FEATURES},kafka_coordinator_test"
fi
if [ "$KAFKA_FENCING_TEST" = "1" ]; then
    FEATURES="${FEATURES},kafka_fencing_test"
fi

if [ "$LIVE_UPGRADE_TEST" = "1" ]; then
    FEATURES="${FEATURES},live_upgrade_test"
fi
if [ "$SHUTDOWN_TEST" = "1" ]; then
    FEATURES="${FEATURES},shutdown_test"
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

catten_boot_report_kernel "$KERNEL"

# --- Build a FAT32 EFI System Partition image with mtools. ---
catten_boot_create_uefi_image \
    "$IMAGE" \
    128 \
    "${ROOT_DIR}/limine-binary/${EFI_BOOT_FILE}" \
    "$KERNEL" \
    "$LIMINE_CONFIG"

# --- NVMe persistent disk image (virt only; sbsa-ref boots the image itself) ---
if [ "$SBSA_REF" != "1" ]; then
    NVME_IMAGE="${IMAGE_DIR}/nvme-disk${INSTANCE_SUFFIX}.img"
    NVME_BUNDLE_HASH="${NVME_IMAGE}.bundle-sha256"
    CURRENT_BUNDLE_HASH="$(catten_boot_bundle_sha256 "$CATTEN_AARCH64_SERVICE_BUNDLE")"
    STORED_BUNDLE_HASH="$(test -f "$NVME_BUNDLE_HASH" && tr -d '[:space:]' < "$NVME_BUNDLE_HASH" || true)"
    if [ "$REUSE_STORAGE" = "1" ] && [ -f "$NVME_IMAGE" ]; then
        echo ">>> Reusing NVMe disk image ${NVME_IMAGE} by explicit request."
        if [ "$STORED_BUNDLE_HASH" != "$CURRENT_BUNDLE_HASH" ]; then
            echo "warning: its service bundle is stale; store-loaded services may not match this build" >&2
        fi
    elif [ ! -f "$NVME_IMAGE" ] || [ "$FRESH_STORAGE" = "1" ] \
        || [ "$STORED_BUNDLE_HASH" != "$CURRENT_BUNDLE_HASH" ]; then
        # The initial image is produced host-side: an object-store volume
        # pre-seeded with the signed service ELFs the kernel loads from the
        # store at boot (the embedded bundle covers only the bootstrap set).
        echo ">>> Producing initial NVMe disk image ${NVME_IMAGE} from the signed bundle..."
        python3 "${ROOT_DIR}/scripts/make-nvme-image.py" \
            "$NVME_IMAGE" "$CATTEN_AARCH64_SERVICE_BUNDLE"
        printf '%s\n' "$CURRENT_BUNDLE_HASH" > "$NVME_BUNDLE_HASH"
    else
        echo ">>> Reusing NVMe disk image ${NVME_IMAGE} (blessed bundle unchanged)."
    fi
fi

# --- sbsa-ref firmware check ---
# The sbsa-ref machine boots from the TF-A + edk2 firmware produced by
# scripts/build-sbsa-firmware.sh, not from QEMU's own UEFI (-bios). Locate
# SBSA_FLASH0/1.fd and fail with a pointer to the build script if absent.
if [ "$SBSA_REF" = "1" ]; then
    resolve_firmware() {
        local name="$1"
        local env_override="${2:-}"
        if [ -n "$env_override" ] && [ -f "$env_override" ]; then
            echo "$env_override"
            return 0
        fi
        for dir in "$ROOT_DIR/target/firmware" "$ROOT_DIR/target/firmware-src/out"; do
            if [ -f "$dir/$name" ]; then
                echo "$dir/$name"
                return 0
            fi
        done
        return 1
    }
    SBSA_FLASH0="$(resolve_firmware SBSA_FLASH0.fd "${CATTEN_SBSA_FLASH0:-}")" \
        || SBSA_FLASH0=""
    SBSA_FLASH1="$(resolve_firmware SBSA_FLASH1.fd "${CATTEN_SBSA_FLASH1:-}")" \
        || SBSA_FLASH1=""
    if [ -z "$SBSA_FLASH0" ] || [ -z "$SBSA_FLASH1" ]; then
        echo "error: sbsa-ref firmware is missing (looked for SBSA_FLASH0/1.fd" >&2
        echo "       under target/firmware/ and target/firmware-src/out/, or" >&2
        echo "       CATTEN_SBSA_FLASH0 / CATTEN_SBSA_FLASH1 if set)." >&2
        echo >&2
        echo "       Build it first:" >&2
        echo "         scripts/build-sbsa-firmware.sh" >&2
        echo >&2
        echo "       (That builds TF-A v2.11 + edk2 with the tracked patches in" >&2
        echo "        patches/ and writes SBSA_FLASH0.fd / SBSA_FLASH1.fd.)" >&2
        exit 1
    fi
    echo ">>> sbsa-ref firmware: $SBSA_FLASH0 / $SBSA_FLASH1"
fi

# --- QEMU options ---
if [ "$SBSA_REF" = "1" ]; then
    MACHINE="sbsa-ref"
    QEMU_OPTS=(
        -M "$MACHINE"
        -cpu neoverse-n1
        -m 512M
        -pflash "$SBSA_FLASH0"
        -pflash "$SBSA_FLASH1"
        -drive "if=none,file=${IMAGE},format=raw,id=nvme0"
        -device "nvme,drive=nvme0,serial=cat0"
    )
else
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
fi

if [ "$USE_HVF" = "1" ]; then
    QEMU_OPTS+=(-accel hvf -cpu host)
elif [ "$SBSA_REF" != "1" ]; then
    QEMU_OPTS+=(-cpu cortex-a710)
    # The named Cortex model has no RNDR instruction. Expose the host CSPRNG
    # through protected DMA to the ordinary VirtIO RNG service.
    QEMU_OPTS+=(
        -object "rng-random,filename=/dev/urandom,id=charlotte-rng"
        -device "virtio-rng-pci,rng=charlotte-rng,disable-legacy=on,iommu_platform=on,addr=4"
    )
fi

QEMU_OPTS+=(-smp "$SMP")

if [ -n "$GDB" ]; then
    QEMU_OPTS+=(-gdb "tcp::${GDB_PORT}")
fi

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

if [ "$SBSA_REF" != "1" ]; then
    # The sbsa-ref boot image already IS the NVMe drive (see the sbsa-ref
    # QEMU_OPTS above); the virt flow's separate persistent NVMe disk does not
    # apply there.
    QEMU_OPTS+=(
        -drive "if=none,file=${NVME_IMAGE},format=raw,id=nvme0"
        -device "nvme,drive=nvme0,serial=cat0,max_ioqpairs=4,addr=2"
    )
fi

QEMU_OPTS+=(-nic none)
if [ "$NETWORK" = "1" ]; then
    case "$NET_BACKEND" in
        user)
            if [ "$HTTP_TEST" = "1" ]; then
                # Host-side keyhole: forward the configurable host port to
                # guest port 80 so parallel/local runs need not contend for a
                # hard-coded listener.
                QEMU_OPTS+=(-netdev "user,id=charlotte-net,hostfwd=tcp::${HTTP_HOST_PORT}-:80,hostfwd=tcp::${DEPLOY_HOST_PORT}-:7444")
            else
                QEMU_OPTS+=(-netdev "user,id=charlotte-net,hostfwd=tcp::${DEPLOY_HOST_PORT}-:7444")
            fi
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
    if [ "${CATTEN_QEMU_NET_DUMP:-0}" = "1" ]; then
        QEMU_OPTS+=(
            -object "filter-dump,id=charlotte-net-dump,netdev=charlotte-net,file=/tmp/charlotte${INSTANCE_SUFFIX}-net.pcap"
        )
    fi
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
        QEMU_OPTS+=(-gdb "tcp::${GDB_PORT}")
    fi
    qemu-system-aarch64 "${QEMU_OPTS[@]}" $GDB &
    QPID=$!
    DEPLOYMENT_RESULT_FILE=""
    DEPLOYMENT_WORKER_LOG=""
    if [ "$DEPLOYMENT_INGRESS_TEST" = "1" ]; then
        DEPLOYMENT_RESULT_FILE="${ROOT_DIR}/target/deployment-ingress-test/result"
        DEPLOYMENT_WORKER_LOG="${ROOT_DIR}/target/deployment-ingress-test/worker.log"
        rm -f "$DEPLOYMENT_RESULT_FILE" "$DEPLOYMENT_WORKER_LOG"
        (
            deadline=$((SECONDS + TIMEOUT - 5))
            until "$CLUSTER_SIGN_BIN" release-apply \
                "$DEPLOYMENT_RELEASE" "127.0.0.1:${DEPLOY_HOST_PORT}" \
                "$((deadline - SECONDS))"; do
                if [ "$SECONDS" -ge "$deadline" ]; then
                    echo "deployment ingress did not accept and realize the release before timeout"
                    exit 1
                fi
                sleep 1
            done
            printf '%s\n' ready >"$DEPLOYMENT_RESULT_FILE"
        ) >"$DEPLOYMENT_WORKER_LOG" 2>&1 &
        DEPLOYMENT_WORKER_PID=$!
    fi
    SELFTEST_COMPLETE=0
    SELFTEST_COMPLETE_TICK=-1
    POWER_OFF_OBSERVED=0
    # A socket-linked peer may still be applying the final Raft entry or
    # consuming the causally ordered deployment barrier when this guest
    # finishes. Under TCG, a runnable verifier can take several host seconds
    # to observe state already published by its service. Keep a successful
    # guest serving long enough for that bounded tail before tearing down the
    # virtual LAN endpoint.
    CLUSTER_DRAIN_TICKS=0
    if [ "$NET_BACKEND" != "user" ]; then
        CLUSTER_DRAIN_TICKS=150
    fi
    HTTP_PROBED=0
    HTTP_PROBE_OK=0
    KAFKA_FAULT_INJECTED=0
    KAFKA_FAULT_RESTARTED=0
    KAFKA_FAULT_TICK=-1
    if [ "$KAFKA_COORDINATOR_TEST" = "1" ]; then
        KAFKA_FAULT_MARKER="[kafka-test] COORDINATOR FAULT WINDOW OPEN"
        KAFKA_FAULT_DESCRIPTION=""
        KAFKA_FAULT_SERVICE=""
    else
        KAFKA_FAULT_MARKER="[kafka-test] FAULT WINDOW OPEN"
        KAFKA_FAULT_DESCRIPTION="route leader kafka2"
        KAFKA_FAULT_SERVICE="kafka2"
    fi
    MAX_TICKS=$((TIMEOUT * 10))
    for ((tick = 0; tick < MAX_TICKS; tick++)); do
        sleep 0.1
        if ! kill -0 "$QPID" 2>/dev/null; then
            wait "$QPID" 2>/dev/null || true
            if [ "$SHUTDOWN_TEST" = "1" ] \
                && grep -Fq "SELFTEST COMPLETE:" "$LOG" \
                && grep -Fq "[shutdown] POWER-OFF REQUESTED via PSCI" "$LOG"; then
                SELFTEST_COMPLETE=1
                POWER_OFF_OBSERVED=1
                echo ">>> Guest completed verified shutdown and powered off through PSCI."
                break
            fi
            echo "error: QEMU exited before the ${TIMEOUT}s test window elapsed" >&2
            if [ -f "$LOG" ]; then
                echo ">>> Serial log (${LOG}):"
                cat "$LOG"
            fi
            exit 1
        fi
        # Probe the guest HTTP keyhole once the httpd reports listening,
        # retrying a few times for the SLIRP/smoltcp path to settle.
        if [ "$HTTP_TEST" = "1" ] && [ "$HTTP_PROBED" = "0" ] \
            && grep -Fq "httpd is listening" "$LOG"; then
            HTTP_PROBED=1
            echo ">>> Probing guest HTTP keyhole at http://127.0.0.1:${HTTP_HOST_PORT}/metrics ..."
            for _ in 1 2 3 4 5 6 7 8; do
                HTTP_BODY="$(curl -fsS --max-time 5 "http://127.0.0.1:${HTTP_HOST_PORT}/metrics" 2>&1 || true)"
                if printf '%s' "$HTTP_BODY" | grep -Fq '"http":{"requests":'; then
                    HTTP_PROBE_OK=1
                    break
                fi
                sleep 2
            done
            echo ">>> Guest HTTP keyhole response:"
            echo "$HTTP_BODY"
            if [ "$HTTP_PROBE_OK" = "1" ]; then
                echo ">>> HTTP keyhole validated from the host."
            else
                echo "error: guest HTTP keyhole did not return the expected JSON state page" >&2
            fi
        fi
        if [ "$KAFKA_TEST" = "1" ] && [ "$KAFKA_FENCING_TEST" = "0" ] \
            && [ "$KAFKA_FAULT_INJECTED" = "0" ] \
            && grep -Fq "$KAFKA_FAULT_MARKER" "$LOG"; then
            if [ "$KAFKA_COORDINATOR_TEST" = "1" ]; then
                KAFKA_COORDINATOR_LINE="$(grep -F "[kafka] coordinators group=" "$LOG" | tail -n 1)"
                KAFKA_TRANSACTION_COORDINATOR="$(printf '%s\n' "$KAFKA_COORDINATOR_LINE" \
                    | sed -E 's/.* transaction=([0-9]+).*/\1/')"
                case "$KAFKA_TRANSACTION_COORDINATOR" in
                    1|2|3) ;;
                    *)
                        echo "error: could not discover Kafka transaction coordinator from guest diagnostics" >&2
                        exit 1
                        ;;
                esac
                KAFKA_FAULT_SERVICE="kafka${KAFKA_TRANSACTION_COORDINATOR}"
                KAFKA_FAULT_DESCRIPTION="transaction coordinator ${KAFKA_FAULT_SERVICE}"
            fi
            echo ">>> Killing Kafka ${KAFKA_FAULT_DESCRIPTION} during the in-guest fault window..."
            docker compose -f "$KAFKA_COMPOSE" kill "$KAFKA_FAULT_SERVICE"
            KAFKA_FAULT_INJECTED=1
            KAFKA_FAULT_TICK=$tick
        fi
        if [ "$KAFKA_FAULT_INJECTED" = "1" ] && [ "$KAFKA_FAULT_RESTARTED" = "0" ] \
            && [ "$tick" -ge $((KAFKA_FAULT_TICK + 450)) ]; then
            echo ">>> Restarting Kafka broker ${KAFKA_FAULT_SERVICE} after the fault window..."
            docker compose -f "$KAFKA_COMPOSE" up -d --wait "$KAFKA_FAULT_SERVICE"
            KAFKA_FAULT_RESTARTED=1
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
            if [ "$SHUTDOWN_TEST" != "1" ] \
                && [ "$SCHEDULER_TRACE" = "0" ] && [ "$DEBUG_SNAPSHOT" = "0" ] \
                && { [ "$KAFKA_TEST" = "0" ] || [ "$KAFKA_FENCING_TEST" = "1" ] \
                    || [ "$KAFKA_FAULT_RESTARTED" = "1" ]; } \
                && { [ "$DEPLOYMENT_INGRESS_TEST" = "0" ] \
                    || [ -f "$DEPLOYMENT_RESULT_FILE" ]; } \
                && [ "$tick" -ge $((SELFTEST_COMPLETE_TICK + CLUSTER_DRAIN_TICKS)) ]; then
                break
            fi
        fi
    done
    if [ "$SCHEDULER_TRACE" = "1" ]; then
        TRACE_RAW="/tmp/charlotte${INSTANCE_SUFFIX}-scheduler-trace.bin"
        TRACE_TEXT="/tmp/charlotte${INSTANCE_SUFFIX}-scheduler-trace.log"
        TRACE_LLDB="/tmp/charlotte${INSTANCE_SUFFIX}-trace-lldb.log"
        read -r TRACE_ADDR TRACE_SIZE < <(nm -S "$KERNEL" | awk '$4 == "DEBUG_TRACE" { print "0x" $1, "0x" $2; exit }')
        if [ -n "${TRACE_ADDR:-}" ] && command -v lldb >/dev/null 2>&1; then
            TRACE_COUNT=$((TRACE_SIZE))
            lldb --batch \
                -o "settings set interpreter.stop-command-source-on-error false" \
                -o "gdb-remote ${GDB_PORT}" \
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
                -o "process detach" "$KERNEL" >"$TRACE_LLDB" 2>&1 || true
            if [ -s "$TRACE_RAW" ]; then
                python3 scripts/decode-scheduler-trace.py "$TRACE_RAW" >"$TRACE_TEXT"
                echo ">>> Scheduler trace captured in ${TRACE_TEXT}"
            else
                echo "warning: scheduler trace capture failed; see ${TRACE_LLDB}" >&2
            fi
        else
            echo "warning: DEBUG_TRACE symbol or lldb unavailable; scheduler trace not captured" >&2
        fi
    fi
    if [ "$DEBUG_SNAPSHOT" = "1" ]; then
        if command -v lldb >/dev/null 2>&1; then
            SNAPSHOT_LLDB="/tmp/charlotte${INSTANCE_SUFFIX}-debug-snapshot-lldb.log"
            TIMER_DIAG_ADDR="$(nm "$KERNEL" | awk '$3 == "TIMER_DIAGNOSTICS" && !found { print "0x" $1; found=1 }')"
            WAKER_DIAG_ADDR="$(nm "$KERNEL" | awk '$3 == "WAKER_DIAGNOSTICS" && !found { print "0x" $1; found=1 }')"
            LIFECYCLE_PROGRESS_ADDR="$(nm "$KERNEL" | awk '$3 == "SCHEDULER_LIFECYCLE_PROGRESS" && !found { print "0x" $1; found=1 }')"
            SCHEDULER_LP_DIAG_ADDR="$(nm "$KERNEL" | awk '$3 == "SCHEDULER_LP_DIAGNOSTICS" && !found { print "0x" $1; found=1 }')"
            lldb --batch \
                -o "settings set interpreter.stop-command-source-on-error false" \
                -o "gdb-remote ${GDB_PORT}" \
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
                -o "process detach" "$KERNEL" >"$SNAPSHOT_LLDB" 2>&1 || true
            echo ">>> Debug snapshot captured in ${SNAPSHOT_LLDB}"
        else
            echo "warning: lldb unavailable; debug snapshot not captured" >&2
        fi
    fi
    if kill -0 "$QPID" 2>/dev/null; then
        kill "$QPID" 2>/dev/null || true
        wait "$QPID" 2>/dev/null || true
    fi
    echo ">>> Serial log (${LOG}):"
    cat "$LOG"
    if [ "$HTTP_TEST" = "1" ] && [ "$HTTP_PROBED" = "1" ] && [ "$HTTP_PROBE_OK" = "0" ]; then
        echo "error: guest HTTP keyhole was not validated from the host" >&2
        exit 1
    fi
    if [ "$DEPLOYMENT_INGRESS_TEST" = "1" ]; then
        wait "$DEPLOYMENT_WORKER_PID" 2>/dev/null || true
        DEPLOYMENT_WORKER_PID=""
        echo ">>> Signed deployment fixture output:"
        cat "$DEPLOYMENT_WORKER_LOG"
        if [ ! -f "$DEPLOYMENT_RESULT_FILE" ]; then
            echo "error: signed RustFS deployment did not become ready" >&2
            exit 1
        fi
        echo ">>> Signed RustFS upload/atomic-release/pull/launch/readiness path validated."
    fi
    if [ "$SELFTEST_COMPLETE" -ne 1 ]; then
        echo "error: authoritative self-test result was not produced within ${TIMEOUT}s" >&2
        exit 1
    fi
    if [ "$SHUTDOWN_TEST" = "1" ] && [ "$POWER_OFF_OBSERVED" -ne 1 ]; then
        echo "error: shutdown test passed without a PSCI system-off transition" >&2
        exit 1
    fi
    if [ "$KAFKA_TEST" = "1" ] && [ "$KAFKA_FENCING_TEST" = "0" ] \
        && { [ "$KAFKA_FAULT_INJECTED" != "1" ] || [ "$KAFKA_FAULT_RESTARTED" != "1" ]; }; then
        echo "error: Kafka broker fault injection did not complete" >&2
        exit 1
    fi
    catten_boot_validate_selftest_log "$LOG"
else
    QEMU_OPTS+=(-serial stdio)
    if [ "$DISPLAY_MODE" = "1" ]; then
        echo ">>> Booting under QEMU (framebuffer window + serial; Ctrl-A X to quit)..."
    else
        echo ">>> Booting under QEMU (serial on stdio; press Ctrl-A X to quit)..."
    fi
    exec qemu-system-aarch64 "${QEMU_OPTS[@]}" $GDB
fi
