#!/usr/bin/env bash
# Shared signed-image inventory. Source before selecting a bundle.
# Operational services never depend on the optional regression applications.
catten_service_bundle() {
    local arch="$1" mode="$2" root="$3"
    case "$mode" in
        minimal|test) ;;
        *) echo "error: bundle mode must be minimal or test" >&2; return 1 ;;
    esac
    local default_heartbeats=1
    [ "$mode" != "minimal" ] || default_heartbeats=0
    SERVICE_HEARTBEATS="${CATTEN_SERVICE_HEARTBEATS-$default_heartbeats}"
    case "$SERVICE_HEARTBEATS" in
        0|1) ;;
        *) echo "error: CATTEN_SERVICE_HEARTBEATS must be 0 or 1" >&2; return 1 ;;
    esac
    SERVICE_FEATURE_ARGS=(--no-default-features)
    if [ "$SERVICE_HEARTBEATS" = "1" ]; then
        SERVICE_FEATURE_ARGS+=(--features service-heartbeats)
    fi
    SERVICES=(ns observe otlp grantctl nvme objstore net disco frouter dns agent
              relmsg tcpip httpd time s3 kafka rng fs clusterctl deployd)
    case "$arch" in
        aarch64) SERVICES+=(uart kafka_step) ;;
        x86_64) SERVICES+=(ahci virtio_blk e1000e) ;;
        *) echo "error: unsupported service-bundle architecture: $arch" >&2; return 1 ;;
    esac
    if [ "$mode" = "test" ]; then
        # Standalone in-memory Raft and the echo-upgrade manager are development
        # fixture servers. The operational DNS service owns its own durable Raft.
        SERVICES+=(raft servicemgr echo client nvme_client objstore_client nclient rclient tcpclient greet
                   shutdown_probe security_probe s3_smoke kafka_smoke)
        if [ "$arch" = "aarch64" ]; then
            SERVICES+=(cclient kafka_fence_smoke kafka_step_proc kafka_step_input)
        fi
    fi
    BUNDLE="$root/target/embedded-services/$arch-unknown-none"
    [ "$mode" != "minimal" ] || BUNDLE="$BUNDLE-minimal"
}

catten_validate_service_bundle() {
    local mode="$1" arch="$2" service elf name
    if [ ! -f "$BUNDLE/service-heartbeats" ] \
        || [ "$(cat "$BUNDLE/service-heartbeats")" != "$SERVICE_HEARTBEATS" ]; then
        echo "error: $mode bundle heartbeat setting is missing or stale; rebuild the service bundle" >&2
        return 1
    fi
    for service in "${SERVICES[@]}"; do
        if [ ! -f "$BUNDLE/$service.elf" ]; then
            echo "error: $mode bundle is missing $service.elf: $BUNDLE" >&2
            return 1
        fi
    done
    # Refuse stale probe files in a minimal bundle even with --skip builds.
    for elf in "$BUNDLE"/*.elf; do
        [ -f "$elf" ] || continue
        name="${elf##*/}"
        name="${name%.elf}"
        for service in "${SERVICES[@]}"; do
            [ "$name" != "$service" ] || break
        done
        [ "$name" != "$service" ] || continue
        if [ "$mode" = "test" ] && { [ "$name" = "smoke" ] \
            || { [ "$arch" = "aarch64" ] && [ "$name" = "sitas-user" ]; }; }; then
            continue
        fi
        echo "error: unexpected $name.elf in $mode bundle: $BUNDLE" >&2
        return 1
    done
}
