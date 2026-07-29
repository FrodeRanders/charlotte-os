#!/usr/bin/env bash
set -euo pipefail

if [[ $# -gt 1 ]]; then
    echo "usage: $0 [path-to-tla2tools.jar]" >&2
    exit 2
fi

tools_jar="${1:-${TLA2TOOLS_JAR:-}}"
if [[ -z "${tools_jar}" || ! -f "${tools_jar}" ]]; then
    echo "error: pass tla2tools.jar as the first argument or set TLA2TOOLS_JAR" >&2
    exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
model_dir="$(mktemp -d "${TMPDIR:-/tmp}/charlotte-tla.XXXXXX")"
trap 'rm -rf "${model_dir}"' EXIT

run_model() {
    local module="$1"
    local config="$2"
    shift 2
    local log="${model_dir}/${module}.log"

    echo ">>> TLC ${module} (${config})"
    if ! java -XX:+UseParallelGC -cp "${tools_jar}" tlc2.TLC \
            "${module}" \
            -config "${config}" \
            -workers auto \
            -metadir "${model_dir}/${module}-states" \
            -coverage 1 >"${log}" 2>&1; then
        cat "${log}" >&2
        exit 1
    fi

    if grep -Eq '^Warning: (The variable|The EXCEPT|Successor state)' "${log}"; then
        echo "error: TLC reported a model-structure warning for ${module}" >&2
        exit 1
    fi

    local action
    for action in "$@"; do
        # TLC reports "new distinct successors:total successors". An action
        # can legitimately contribute zero new states when another action
        # reaches the same successor; a positive total still proves it was
        # enabled and exercised.
        if ! grep -Eq "^<${action} .*: [0-9]+:[1-9][0-9]*$" "${log}"; then
            echo "error: required action ${action} had no TLC coverage in ${module}" >&2
            exit 1
        fi
        grep -E "^<${action} " "${log}" | tail -n 1
    done
    grep -E 'states generated,|The depth of the complete state graph' "${log}"
}

cd "${script_dir}"
run_model CharlotteIPC CharlotteIPC_small.cfg \
    MemoryCreate ScalarCallMove ScalarCallBorrowRead ScalarCallBorrowWrite \
    ScalarCallCopy Receive ReplyReturnMemory EndpointClose \
    DomainTeardown
run_model CharlotteCQ CharlotteCQ_mini.cfg \
    Complete Fail CancelOp DrainOne DrainAll ObserveResult CqWait CqWake TimerFire
run_model CharlotteScheduler CharlotteScheduler_small.cfg \
    Spawn Admit Dispatch Preempt Block Wake Migrate Abort Reap
run_model CharlotteServiceLifecycle CharlotteServiceLifecycle_small.cfg \
    Load Start Publish RequestStop Exit Reap Teardown
run_model CharlotteCapability CharlotteCapability_small.cfg \
    Allocate Remove DelegateCopy BeginMove CommitMove RollbackMove CloseAddressSpace
run_model CharlotteDMA CharlotteDMA_small.cfg \
    CreateMemory CreateDomain BeginMap CommitMap QuarantineMap FailMap RevokeMap ReleasePin \
    BeginDestroy AcknowledgeDestroy QuarantineDestroy FinalizeDomain \
    CloseMemory ExitDriver ReclaimMemory
run_model CharlotteRaft CharlotteRaft_small.cfg \
    StartElection GrantVote BecomeLeader ObserveHigherTerm Crash Restart
run_model CharlotteRaftLog CharlotteRaftLog_small.cfg \
    Elect AppendLeader ReplicateOne CommitLeader PropagateCommit Crash Restart
run_model CharlotteRaftSnapshot CharlotteRaftSnapshot_small.cfg \
    AppendLog Commit BeginReceive ReceiveChunk PersistSnapshot ActivateSnapshot \
    DiscardStale Crash Restart
