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

run_expected_violation() {
    local module="$1"
    local config="$2"
    local invariant="$3"
    local action="$4"
    local run_id="${module}-${config%.cfg}"
    local log="${model_dir}/${run_id}-negative.log"

    echo ">>> TLC ${module} (${config}, expected counterexample)"
    if java -XX:+UseParallelGC -cp "${tools_jar}" tlc2.TLC \
            "${module}" \
            -config "${config}" \
            -workers auto \
            -metadir "${model_dir}/${run_id}-negative-states" \
            -coverage 1 >"${log}" 2>&1; then
        echo "error: negative model unexpectedly satisfied ${invariant}" >&2
        cat "${log}" >&2
        exit 1
    fi
    if ! grep -Fq "Invariant ${invariant} is violated" "${log}"; then
        echo "error: negative model failed for a reason other than ${invariant}" >&2
        cat "${log}" >&2
        exit 1
    fi
    if ! grep -Eq "^<${action} .*: [0-9]+:[1-9][0-9]*$" "${log}"; then
        echo "error: expected counterexample did not exercise ${action}" >&2
        cat "${log}" >&2
        exit 1
    fi
    grep -F "Invariant ${invariant} is violated" "${log}"
    grep -E "^<${action} " "${log}" | tail -n 1
}

cd "${script_dir}"
run_model CharlotteIPC CharlotteIPC_small.cfg \
    MemoryCreate ScalarCallMove ScalarCallBorrowRead ScalarCallBorrowWrite \
    ScalarCallCopy Receive ReplyReturnMemory EndpointClose \
    DomainTeardown
run_model CharlotteEndpointObservers CharlotteEndpointObservers_small.cfg \
    ArmReadiness ArmCloseWatch Send Receive Close ObserveClose
run_expected_violation CharlotteEndpointObservers CharlotteEndpointObservers_unsafe.cfg \
    CloseSignalImpliesClosed UnsafeMessageWake
run_model CharlotteCQ CharlotteCQ_mini.cfg \
    Complete Fail CancelOp DrainOne DrainAll ObserveResult CqWait CqWake TimerFire
run_expected_violation CharlotteCQ CharlotteCQ_buffer_unsafe.cfg \
    NonTerminalBufferRemainsLoaned UnsafeCancelReleasesBuffer
run_model CharlotteTimedWait CharlotteTimedWait_small.cfg \
    ArmWait PublishWork DeliverWake TimerFire Consume
run_expected_violation CharlotteTimedWait CharlotteTimedWait_unsafe.cfg \
    TimeoutObservedNoWork UnsafeTimerFire
run_model CharlotteScheduler CharlotteScheduler_small.cfg \
    Spawn Admit Dispatch Preempt Block Wake SwitchOff Migrate \
    RequestRemoteAbort RetireRemoteAbort AbortNotRunning SelfAbort Reap \
    BeginDomainAbort DestroyAddressSpace
run_expected_violation CharlotteScheduler CharlotteScheduler_unsafe.cfg \
    ReapOnlyOffCpu UnsafeRemoteAbort
run_expected_violation CharlotteScheduler CharlotteScheduler_domain_abort_unsafe.cfg \
    AbortingThreadsDoomed UnsafeSpawnDuringAbort
run_model CharlotteThreadJoin CharlotteThreadJoin_small.cfg \
    Spawn CaptureHandle Exit ObserveJoin Reap
run_expected_violation CharlotteThreadJoin CharlotteThreadJoin_unsafe.cfg \
    ObserverMatchesCapturedHandle UnsafeObserveJoin
run_model CharlotteAddressSpace CharlotteAddressSpace_small.cfg \
    Allocate CaptureHandle CloseExact
run_expected_violation CharlotteAddressSpace CharlotteAddressSpace_unsafe.cfg \
    ReplacementSurvivesStaleHandle UnsafeStaleClose
run_model CharlotteHardwareAsid CharlotteHardwareAsid_small.cfg \
    Allocate CacheTranslation Retire Invalidate
run_expected_violation CharlotteHardwareAsid CharlotteHardwareAsid_unsafe.cfg \
    NoDirtyTagReuse UnsafeAllocate
run_model CharlotteInterruptRoute CharlotteInterruptRoute_small.cfg \
    Bind QueueWake Unbind DrainSafe
run_expected_violation CharlotteInterruptRoute CharlotteInterruptRoute_unsafe.cfg \
    NoStaleWakeDelivery DrainUnsafe
run_model CharlotteServiceLifecycle CharlotteServiceLifecycle_small.cfg \
    StageTrusted StageUntrusted RejectUntrustedLoad Load Start Prepare \
    PublishLocal Activate RejectStaleActivate Lookup ClearLookup \
    FencedUnregister RejectStaleUnregister CleanupLocal RequestStop Exit DomainAbort Reap Teardown
run_expected_violation CharlotteServiceLifecycle CharlotteServiceLifecycle_unsafe.cfg \
    ReplacementSurvivesStaleUnregister UnsafeStaleUnregister
run_model CharlotteCapability CharlotteCapability_small.cfg \
    Allocate Remove DelegateCopy BeginMove CommitMove RollbackMove CloseAddressSpace
run_model CharlotteAuthorization CharlotteAuthorization_small.cfg \
    PublishService ReplaceService UnpublishService SetPolicy IssueTicket \
    CancelTicket Redeem CloseCapability
run_expected_violation CharlotteAuthorization \
    CharlotteAuthorization_policy_unsafe.cfg \
    PolicyMutationAuthorized UnsafeSetPolicy
run_expected_violation CharlotteAuthorization \
    CharlotteAuthorization_principal_unsafe.cfg \
    MintBoundToPrincipal UnsafeRedeemOtherPrincipal
run_expected_violation CharlotteAuthorization \
    CharlotteAuthorization_policy_version_unsafe.cfg \
    MintUsesCurrentPolicy UnsafeRedeemStalePolicy
run_expected_violation CharlotteAuthorization \
    CharlotteAuthorization_generation_unsafe.cfg \
    MintTargetsCurrentBinding UnsafeRedeemStaleBinding
run_expected_violation CharlotteAuthorization \
    CharlotteAuthorization_rights_unsafe.cfg \
    NoRightsAmplification UnsafeAmplifyRights
run_model CharlotteDMA CharlotteDMA_small.cfg \
    CreateMemory CreateDomain CpuMap CpuUnmap BeginLoan EndLoan BeginMap \
    CommitMap QuarantineMap FailMap RevokeMap ReleasePin \
    BeginDestroy AcknowledgeDestroy QuarantineDestroy FinalizeDomain \
    CloseMemory ExitDriver ReclaimMemory
run_expected_violation CharlotteDMA CharlotteDMA_unsafe.cfg \
    ExclusiveDmaHasNoCpuAuthority UnsafeBeginExclusiveMap
run_model CharlotteRaft CharlotteRaft_small.cfg \
    StartElection GrantVote BecomeLeader ObserveHigherTerm Crash Restart
run_model CharlotteRaftLog CharlotteRaftLog_small.cfg \
    Elect AppendLeader ReplicateOne CommitLeader PropagateCommit Crash Restart
run_model CharlotteRaftMembership CharlotteRaftMembership_small.cfg \
    Elect SubmitJoint Replicate CommitJoint SubmitFinalize CommitFinalize Crash Restart
run_model CharlotteRaftJoin CharlotteRaftJoin_small.cfg \
    Elect BeginJoining Crash Restart SubmitJoin CommitJoin ReplicateToJoiner SubmitJoint CommitJoint
run_expected_violation CharlotteRaftJoin CharlotteRaftJoin_unsafe.cfg \
    JoiningAcceptsOnlySelectedAnchor UnsafeReplicateToJoiner
run_expected_violation CharlotteRaftJoin CharlotteRaftJoin_restart_unsafe.cfg \
    RestartPreservesAdmission UnsafeRestartForgetsAdmission
run_model CharlotteRaftSnapshot CharlotteRaftSnapshot_small.cfg \
    AppendLog Commit BeginReceive ReceiveChunk PersistSnapshot ActivateSnapshot \
    DiscardStale Crash Restart
run_model CharlotteRemoteCall CharlotteRemoteCall_small.cfg \
    Start ReplaceTarget Execute RejectStale QueueReply DuplicateRequest \
    DeliverReply Timeout SettleTransport RetireUncertainSession Evict
run_model CharlotteReliableMessage CharlotteReliableMessage_small.cfg \
    AbandonSession RestartService AcceptCurrentSession
run_expected_violation CharlotteReliableMessage \
    CharlotteReliableMessage_identity_unsafe.cfg \
    SessionIdentityUnique RestartService
run_expected_violation CharlotteReliableMessage \
    CharlotteReliableMessage_regression_unsafe.cfg \
    ReceiveSessionMonotonic AcceptDelayedSession
