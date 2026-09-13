#!/usr/bin/env bash
# Three-member L2-ingress fixture with Raft-leader/VIP-owner failure.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUNNER="${ROOT_DIR}/scripts/run-aarch64.sh"
HUB="${CATTEN_INGRESS_HUB:-127.0.0.1:12042}"
SERVICE="${CATTEN_INGRESS_SERVICE:-10.0.0.42:80}"
TIMEOUT="${CATTEN_INGRESS_TIMEOUT:-180}"
PHASE_TIMEOUT="${CATTEN_INGRESS_PHASE_TIMEOUT:-90}"
SMP="${CATTEN_INGRESS_SMP:-4}"
WORK="/tmp/charlotte-ingress-fixture"
mkdir -p "$WORK"
for fixture_node in ingress-a ingress-b ingress-c; do
    : >"$WORK/${fixture_node}.qemu.pid"
    : >"$WORK/${fixture_node}.capacity-start"
    : >"/tmp/charlotte-${fixture_node}-serial.log"
done

pids=()
cleanup() {
    for pid_file in "$WORK"/*.qemu.pid; do
        [ -f "$pid_file" ] || continue
        qemu_pid="$(tr -d '[:space:]' <"$pid_file")"
        if [ -n "$qemu_pid" ]; then
            kill "$qemu_pid" 2>/dev/null || true
        fi
    done
    for pid in "${pids[@]}"; do
        kill "$pid" 2>/dev/null || true
    done
}
trap cleanup EXIT INT TERM

start_node() {
    node="$1"
    mac="$2"
    skip_build="$3"
    runner_log="$WORK/${node}.runner.log"
    : >"$runner_log"
    if [ "$skip_build" = "1" ]; then
        CATTEN_SKIP_EMBED_BUILD=1 CATTEN_SKIP_KERNEL_BUILD=1 \
            CATTEN_ALLOW_SANDBOX_NETWORK=1 CATTEN_QEMU_PID_FILE="$WORK/${node}.qemu.pid" \
            "$RUNNER" release --cluster-service "$SERVICE" --cluster-ingress-test \
            --net-connect "$HUB" --instance "$node" --mac "$mac" \
            --fresh-storage --smp "$SMP" --timeout "$TIMEOUT" >"$runner_log" 2>&1 &
    else
        CATTEN_ALLOW_SANDBOX_NETWORK=1 CATTEN_QEMU_PID_FILE="$WORK/${node}.qemu.pid" \
            "$RUNNER" release --cluster-service "$SERVICE" --cluster-ingress-test \
            --net-connect "$HUB" --instance "$node" --mac "$mac" \
            --fresh-storage --smp "$SMP" --timeout "$TIMEOUT" >"$runner_log" 2>&1 &
    fi
    last_pid=$!
    pids+=("$last_pid")
}

wait_for_text() {
    file="$1"
    pattern="$2"
    deadline=$((SECONDS + TIMEOUT))
    until [ -f "$file" ] && grep -Eq "$pattern" "$file"; do
        if [ "$SECONDS" -ge "$deadline" ]; then
            echo "error: timed out waiting for '$pattern' in $file" >&2
            return 1
        fi
        sleep 1
    done
}

wait_for_capacity_control() {
    required="$1"
    shift
    deadline=$((SECONDS + PHASE_TIMEOUT))
    while [ "$SECONDS" -lt "$deadline" ]; do
        for node in "$@"; do
            serial="/tmp/charlotte-${node}-serial.log"
            [ -f "$serial" ] || continue
            start_file="$WORK/${node}.capacity-start"
            if [ -s "$start_file" ]; then
                start="$(tr -d '[:space:]' <"$start_file")"
                count="$(tail -n "+$start" "$serial" | sed -n \
                    's/.*seeded capacity control for node \([0-9a-f][0-9a-f]*\).*/\1/p' \
                    | sort -u | wc -l | tr -d '[:space:]')"
            else
                count="$(sed -n \
                    's/.*seeded capacity control for node \([0-9a-f][0-9a-f]*\).*/\1/p' \
                    "$serial" | sort -u | wc -l | tr -d '[:space:]')"
            fi
            if [ "${count:-0}" -ge "$required" ]; then
                echo ">>> ${node} seeded fresh placement-control state for ${count} node(s)."
                return 0
            fi
        done
        sleep 1
    done
    echo "error: no candidate leader seeded capacity control for ${required} node(s)" >&2
    return 1
}

python3 "$ROOT_DIR/scripts/qemu-stream-l2-hub.py" --listen "$HUB" --trace-tcp >"$WORK/hub.log" 2>&1 &
hub_pid="$!"
pids+=("$hub_pid")
sleep 1
if ! kill -0 "$hub_pid" 2>/dev/null; then
    echo "error: QEMU L2 hub failed to start" >&2
    cat "$WORK/hub.log" >&2
    exit 1
fi
wait_for_text "$WORK/hub.log" "QEMU L2 hub listening"
python3 "$ROOT_DIR/scripts/qemu-stream-l2-probe.py" --connect "$HUB" \
    --vip "${SERVICE%:*}" --timeout "$PHASE_TIMEOUT" >"$WORK/probe.log" 2>&1 &
probe_pid="$!"
pids+=("$probe_pid")
wait_for_text "$WORK/probe.log" "external L2 probe armed"

# Start the lowest durable identity first so it remains the deterministic
# admission anchor.  Admit one peer at a time: simultaneous fresh singleton
# elections can legitimately choose different intermediate clusters before
# discovery converges, while this fixture is intended to exercise ingress
# failover after a stable three-voter configuration has committed.
start_node ingress-a 52:54:00:12:34:03 0
first_runner="$last_pid"
wait_for_text "$WORK/ingress-a.runner.log" "Booting under QEMU"
if ! kill -0 "$first_runner" 2>/dev/null; then
    echo "error: first ingress guest failed during build" >&2
    cat "$WORK/ingress-a.runner.log" >&2
    exit 1
fi

start_node ingress-b 52:54:00:12:34:02 1
wait_for_text "/tmp/charlotte-ingress-a-serial.log" "VIP SNAPSHOT OWNER .*backends=2"
start_node ingress-c 52:54:00:12:34:01 1

for node in ingress-a ingress-b ingress-c; do
    wait_for_text "/tmp/charlotte-${node}-serial.log" \
        "MEMBERSHIP epoch=.*joint=false members=3"
done
wait_for_capacity_control 3 ingress-a ingress-b ingress-c
kill -USR1 "$probe_pid"
wait_for_text "$WORK/probe.log" "external FAILOVER WINDOW OPEN"

owner=""
for node in ingress-a ingress-b ingress-c; do
    if grep -Eq "VIP SNAPSHOT OWNER .*backends=3" "/tmp/charlotte-${node}-serial.log"; then
        owner="$node"
        break
    fi
done
if [ -z "$owner" ]; then
    echo "error: no three-member Raft leader acquired VIP advertisement" >&2
    exit 1
fi

echo ">>> Stopping VIP advertiser ${owner}; backend flows remain on surviving members."
for node in ingress-a ingress-b ingress-c; do
    lines="$(wc -l <"/tmp/charlotte-${node}-serial.log" | tr -d '[:space:]')"
    echo "$((lines + 1))" >"$WORK/${node}.capacity-start"
done
owner_qemu="$(tr -d '[:space:]' <"$WORK/${owner}.qemu.pid")"
kill "$owner_qemu"

survivors=()
for node in ingress-a ingress-b ingress-c; do
    if [ "$node" != "$owner" ]; then
        survivors+=("$node")
    fi
done
for node in "${survivors[@]}"; do
    wait_for_text "/tmp/charlotte-${node}-serial.log" "SELFTEST COMPLETE: .*failed=0 pending=0"
done
wait_for_capacity_control 2 "${survivors[@]}"
wait "$probe_pid"
wait_for_text "$WORK/probe.log" \
    "external [1-9][0-9]* flow\(s\) survived the failover window"
wait_for_text "$WORK/probe.log" "external reconnect succeeded on source port"

if ! grep -Eq "FIRST REMOTE VIP FRAME" /tmp/charlotte-ingress-*-serial.log; then
    echo "error: fixture did not exercise remote L2 forwarding" >&2
    exit 1
fi
if ! grep -Eq "VIP ADVERTISER ACQUIRED" "/tmp/charlotte-${survivors[0]}-serial.log" \
    && ! grep -Eq "VIP ADVERTISER ACQUIRED" "/tmp/charlotte-${survivors[1]}-serial.log"; then
    echo "error: no surviving member acquired VIP advertisement" >&2
    exit 1
fi

echo ">>> Distributed ingress verified: remote selection, leader failover, fresh capacity-control reconstruction, gratuitous ARP, surviving established flow, and backend-loss reconnect."
