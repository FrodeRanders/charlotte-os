#!/usr/bin/env bash
set -euo pipefail

if [[ $# -gt 2 ]]; then
    echo "usage: $0 [path-to-tla2tools.jar] [module]" >&2
    exit 2
fi

tools_jar="${1:-${TLA2TOOLS_JAR:-}}"
if [[ -z "${tools_jar}" || ! -f "${tools_jar}" ]]; then
    echo "error: pass tla2tools.jar as the first argument or set TLA2TOOLS_JAR" >&2
    exit 2
fi

selected_module="${2:-}"
selected_cases=0

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
model_dir="$(mktemp -d "${TMPDIR:-/tmp}/knytos-tla.XXXXXX")"
trap 'rm -rf "${model_dir}"' EXIT

run_model() {
    local module="$1"
    local config="$2"
    if [[ -n "${selected_module}" && "${module}" != "${selected_module}" ]]; then
        return 0
    fi
    selected_cases=$((selected_cases + 1))
    shift 2
    local run_id="${module}-${config%.cfg}"
    local log="${model_dir}/${run_id}.log"

    echo ">>> TLC ${module} (${config})"
    if ! java -XX:+UseParallelGC -cp "${tools_jar}" tlc2.TLC \
            "${module}" \
            -config "${config}" \
            -workers auto \
            -metadir "${model_dir}/${run_id}-states" \
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
    if [[ -n "${selected_module}" && "${module}" != "${selected_module}" ]]; then
        return 0
    fi
    selected_cases=$((selected_cases + 1))
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
run_model PrincipalHistory PrincipalHistory_small.cfg \
    Prepare Attach Decide Exit Confirm ReleaseRoot Cancel Abandon
run_expected_violation PrincipalHistory PrincipalHistory_ordinary_unsafe.cfg \
    OrdinaryCeiling UnsafeIgnoreOrdinary
run_expected_violation PrincipalHistory PrincipalHistory_exit_unsafe.cfg \
    Accounted UnsafeExitRefund
run_expected_violation PrincipalHistory PrincipalHistory_drop_unsafe.cfg \
    Accounted UnsafeDropRefund
run_expected_violation PrincipalHistory PrincipalHistory_class_unsafe.cfg \
    OriginalClass UnsafeReclassify
run_expected_violation PrincipalHistory PrincipalHistory_evict_unsafe.cfg \
    HistoryRetained UnsafeEvict
run_expected_violation PrincipalHistory PrincipalHistory_early_unsafe.cfg \
    RootProof UnsafeRootFree
run_model ThreadMetadata ThreadMetadata_small.cfg \
    Admit AllocateNode AllocateContext Publish DisposeContext FreeNode Refund Abandon Retire Reuse
run_expected_violation ThreadMetadata ThreadMetadata_generation_unsafe.cfg \
    PrincipalCeiling UnsafeGenerationOnly
run_expected_violation ThreadMetadata ThreadMetadata_early_unsafe.cfg \
    Accounted UnsafeEarlyRefund
run_expected_violation ThreadMetadata ThreadMetadata_drop_unsafe.cfg \
    Accounted UnsafeDropRefund
run_expected_violation ThreadMetadata ThreadMetadata_reuse_unsafe.cfg \
    OriginalSponsor UnsafeReplacementSponsor
run_model PrincipalBacking PrincipalBacking_small.cfg \
    Allocate Grow Move Retire Restart ConfirmRelease
run_expected_violation PrincipalBacking PrincipalBacking_generation_unsafe.cfg \
    PrincipalCeiling UnsafeGenerationOnly
run_expected_violation PrincipalBacking PrincipalBacking_refund_unsafe.cfg \
    Accounted UnsafeRetirementRefund
run_expected_violation PrincipalBacking PrincipalBacking_move_unsafe.cfg \
    Accounted UnsafeMoveSponsor
run_expected_violation PrincipalBacking PrincipalBacking_reuse_unsafe.cfg \
    Accounted UnsafeReuseSponsor
run_expected_violation PrincipalBacking PrincipalBacking_growth_unsafe.cfg \
    Accounted UnsafeGrowCharge
run_expected_violation PrincipalBacking PrincipalBacking_drop_unsafe.cfg \
    Accounted UnsafeDropRefund
run_model ControllerRelay ControllerRelay_small.cfg \
    ProducerReply CheckedPreflight RejectPreflight ObserveTerminal Expire ReleaseReceiver Park Wake Tick AbortBindingOrWait
run_expected_violation ControllerRelay ControllerRelay_unknown_unsafe.cfg \
    MutationRequiresCheckedQuery UnsafeUnknownAsMissing
run_expected_violation ControllerRelay ControllerRelay_budget_unsafe.cfg \
    OriginalBudget UnsafeRestart
run_expected_violation ControllerRelay ControllerRelay_serial_unsafe.cfg \
    ParkCoversAll UnsafeSerialWait
run_expected_violation ControllerRelay ControllerRelay_replay_unsafe.cfg \
    AcceptedOnce UnsafeReplay
run_model DeploymentListener DeploymentListener_small.cfg \
    Admit RejectQueue ProducerReply Observe RejectConfirmed Abort Park Wake Tick
run_expected_violation DeploymentListener DeploymentListener_replay_unsafe.cfg \
    AcceptedOnce UnsafeReplay
run_expected_violation DeploymentListener DeploymentListener_budget_unsafe.cfg \
    OriginalBudget UnsafeRestart
run_expected_violation DeploymentListener DeploymentListener_serial_unsafe.cfg \
    ParkCoversAll UnsafeSerialPark
run_model NamespaceDrain NamespaceDrain_small.cfg \
    Select SelectEmpty ClosePeer FinishClaim CancelSelected RejectPreparation RecheckMissing ReturnPending
run_expected_violation NamespaceDrain NamespaceDrain_stale_unsafe.cfg \
    NoStalePending UnsafeStalePending
run_expected_violation NamespaceDrain NamespaceDrain_skip_unsafe.cfg \
    CompleteOnlyDrained UnsafeSkipLive
run_model DeploymentClose DeploymentClose_blocking.cfg \
    RejectQueue Admit ProducerAck ProducerUnavailable ObserveAck Expire RejectWaitOrObservation Park Wake Tick RetainAdmission
run_expected_violation DeploymentClose DeploymentClose_fallback_unsafe.cfg \
    NoReentry UnsafeFallback
run_model DeploymentClose DeploymentClose_small.cfg \
    RejectQueue Admit ProducerAck ProducerUnavailable ObserveAck Expire RejectWaitOrObservation Park Wake Tick
run_expected_violation DeploymentClose DeploymentClose_replay_unsafe.cfg \
    AcceptedOnce UnsafeReplay
run_expected_violation DeploymentClose DeploymentClose_release_unsafe.cfg \
    OwnersUntilAck UnsafeRelease
run_model DeploymentOutput DeploymentOutput_small.cfg \
    ProducerReceipt ObservePrefix EndUncertain Tick Park Wake RejectWait
run_expected_violation DeploymentOutput DeploymentOutput_replay_unsafe.cfg \
    PrefixesOnce UnsafeReplay
run_expected_violation DeploymentOutput DeploymentOutput_budget_unsafe.cfg \
    OriginalBudget UnsafeRestartBudget
run_model DeploymentControl DeploymentControl_small.cfg \
    Bind Complete Observe Expire ReleaseReceiver Park Tick RejectAdmission
run_expected_violation DeploymentControl DeploymentControl_replay_unsafe.cfg \
    SubmittedOnce UnsafeReplay
run_expected_violation DeploymentControl DeploymentControl_serial_unsafe.cfg \
    ParkCoversAll UnsafeSerialWait
run_model DeploymentInput DeploymentInput_small.cfg \
    Advance ProducerReady Tick Park RejectWait
run_expected_violation DeploymentInput DeploymentInput_cursor_unsafe.cfg \
    HealthyTurnBound UnsafeKeepCursor
run_expected_violation DeploymentInput DeploymentInput_deadline_unsafe.cfg \
    OriginalBudget UnsafeRestartBudget
run_model Ipc Ipc_small.cfg \
    MemoryCreate ScalarCallMove ScalarCallBorrowRead ScalarCallBorrowWrite \
    ScalarCallCopy Receive ReplyReturnMemory EndpointClose \
    DomainTeardown
run_model IpcTransaction IpcTransaction_small.cfg \
    BeginVector TransferMove BeginBorrow AttachConnection Commit \
    FailAndRollback Deliver WaitTimeout Reply ObserveReply
run_expected_violation IpcTransaction \
    IpcTransaction_rollback_unsafe.cfg \
    RollbackRestoresAll UnsafeRollback
run_expected_violation IpcTransaction \
    IpcTransaction_timeout_unsafe.cfg \
    TimeoutRetainsLoan UnsafeTimeoutRelease
run_model IpcPublication IpcPublication_small.cfg \
    Begin PrepareRecord RejectStorage Reserve RejectReservation PrepareMemory \
    PrepareConnection Commit RejectCommit FinishPreparation AbortPreparation \
    Readiness PrepareReceive RejectReceive ReceiveWriteFailed CommitReceive \
    ObserveReply RepeatObservation Use ApplicationClose CancelHidden Fill \
    RetireNamespace DetachNamespace ReplaceNamespace ReleaseRetired Abandon
run_expected_violation IpcPublication IpcPublication_memory_unsafe.cfg \
    UseRequiresDelivery UnsafeUseMemory
run_expected_violation IpcPublication IpcPublication_connection_unsafe.cfg \
    UseRequiresDelivery UnsafeUseConnection
run_expected_violation IpcPublication IpcPublication_batch_unsafe.cfg \
    AtomicPublication UnsafePublishPrefix
run_expected_violation IpcPublication IpcPublication_serial_unsafe.cfg \
    SerialsConsumed UnsafeReuseSerial
run_expected_violation IpcPublication IpcPublication_cancel_unsafe.cfg \
    ReplacementIntact UnsafeCancelSuccessor
run_expected_violation IpcPublication IpcPublication_restore_unsafe.cfg \
    ReplacementIntact UnsafeRestoreSuccessor
run_expected_violation IpcPublication IpcPublication_refund_unsafe.cfg \
    RetainedNodesCharged UnsafeRefundRetained
run_expected_violation IpcPublication IpcPublication_readiness_unsafe.cfg \
    VisibleRequiresDelivery UnsafeReadinessDelivers
run_expected_violation IpcPublication IpcPublication_observe_unsafe.cfg \
    ObservedCloseStaysClosed UnsafeRepublishObserved
run_expected_violation IpcPublication IpcPublication_rollback_unsafe.cfg \
    RollbackRestoresCapturedSource UnsafeRollbackNeedsAdmission
run_model EndpointObservers EndpointObservers_small.cfg \
    ArmReadiness ArmCloseWatch Send Receive Close ObserveClose
run_expected_violation EndpointObservers EndpointObservers_unsafe.cfg \
    CloseSignalImpliesClosed UnsafeMessageWake
run_model NamePublication NamePublication_small.cfg \
    Begin RejectBeforeWatch PrepareWatch RejectPolicy PublishPolicy Install \
    Lookup QueueLookup ResolveDeferred FinishInstall CloseEndpoint CqHint \
    ObservePending ObserveUnrelated RetireDeath RetireExplicit RejectRetirement \
    CancelWatch CloseSource WatchError WaitRejected LifecycleAbort PublicationAbort
run_model NamePublication NamePublication_exhaustion.cfg \
    Begin PrepareWatch PublishPolicy Install FinishInstall ExhaustionRetire \
    CancelWatch CloseSource RejectExhausted
run_expected_violation NamePublication NamePublication_public_unsafe.cfg \
    PublicIssuance UnsafePublicLookup
run_expected_violation NamePublication NamePublication_deferred_unsafe.cfg \
    PublicIssuance UnsafeDeferredLookup
run_expected_violation NamePublication NamePublication_replacement_unsafe.cfg \
    ReplacementProtected UnsafePublicReplacement
run_expected_violation NamePublication NamePublication_watch_unsafe.cfg \
    BeforeWatchUnchanged UnsafeMutationBeforeWatch
run_expected_violation NamePublication NamePublication_hint_unsafe.cfg \
    DeathNeedsExactResult UnsafeHintRetires
run_expected_violation NamePublication NamePublication_stale_unsafe.cfg \
    DeathNeedsExactResult UnsafeStaleHint
run_expected_violation NamePublication NamePublication_disposal_unsafe.cfg \
    WatchRetainsSource UnsafeCloseSourceFirst
run_expected_violation NamePublication NamePublication_quota_unsafe.cfg \
    LiveBound UnsafeNewAtCapacity
run_expected_violation NamePublication NamePublication_abort_unsafe.cfg \
    AbortRetainsOwners UnsafeAbortDisposes
run_expected_violation NamePublication NamePublication_retire_unsafe.cfg \
    RetirementAuthorized UnsafeUnauthorizedRetire
run_expected_violation NamePublication NamePublication_wrap_unsafe.cfg \
    GenerationNotReused UnsafeGenerationWrap
run_expected_violation NamePublication NamePublication_exhausted_unsafe.cfg \
    PolicyGenerationRepresentable UnsafePublishExhausted
run_expected_violation NamePublication NamePublication_serial_unsafe.cfg \
    GrantGenerationCoherent UnsafeLookupDuringPublish
run_model BootTrust BootTrust_fixture.cfg \
    ReadInstalled ReadFailure Verify RejectRecord AcquireRecipient PrepareSigned \
    RejectRecipient PrepareDevelopment SelectAuthority LoadBootstrap LoadPlatform \
    Publish RejectPublication BorrowView DropPreparation Reboot
run_model BootTrust BootTrust_contract.cfg \
    ReadInstalled ReadEnrollment ReadFailure Verify RejectRecord CommitPolicy \
    CommitFailure CommitInterrupted CompetingCommit AcquireRecipient PrepareSigned \
    RejectRecipient SelectAuthority LoadBootstrap LoadPlatform \
    Publish RejectPublication BorrowView DropPreparation Reboot
run_expected_violation BootTrust BootTrust_uninstalled_unsafe.cfg \
    InstalledBeforePublication UnsafeUninstalledPublish
run_expected_violation BootTrust BootTrust_stale_unsafe.cfg \
    NoForkedInstallation UnsafeStaleCommit
run_expected_violation BootTrust BootTrust_enrollment_unsafe.cfg \
    NoEnrollmentFallback UnsafeEnrollmentFallback
run_expected_violation BootTrust BootTrust_overwrite_unsafe.cfg \
    OnePublicationPerBoot UnsafeOverwrite
run_expected_violation BootTrust BootTrust_manifest_unsafe.cfg \
    SharedPolicy UnsafeIndependentManifest
run_expected_violation BootTrust BootTrust_recipient_unsafe.cfg \
    RecipientBound UnsafeWrongRecipient
run_expected_violation BootTrust BootTrust_authenticate_unsafe.cfg \
    VerifiedAuthenticated UnsafeAuthenticate
run_expected_violation BootTrust BootTrust_development_unsafe.cfg \
    ExplicitDevelopment UnsafeDevelopmentFallback
run_expected_violation BootTrust BootTrust_failure_unsafe.cfg \
    InstalledBeforePublication UnsafeFailedCommitPublishes
run_expected_violation BootTrust BootTrust_boundary_unsafe.cfg \
    ConditionalBoundary UnsafeUnsupportedProduction
run_expected_violation BootTrust BootTrust_platform_unsafe.cfg \
    PlatformImagesUseSelectedAuthority UnsafePlatformFallback
run_expected_violation BootTrust BootTrust_selection_unsafe.cfg \
    SelectedAuthorityMatchesState UnsafeSelectionOverwrite
run_model Cq Cq_mini.cfg \
    Complete Fail CancelOp DrainOne DrainAll ObserveResult CqWait CqWake TimerFire
run_expected_violation Cq Cq_buffer_unsafe.cfg \
    NonTerminalBufferRemainsLoaned UnsafeCancelReleasesBuffer
run_model TimedWait TimedWait_small.cfg \
    ArmWait PublishWork DeliverWake TimerFire Consume
run_expected_violation TimedWait TimedWait_unsafe.cfg \
    TimeoutObservedNoWork UnsafeTimerFire
run_model CheckedWait CheckedWait_small.cfg \
    BeginWait PrepareWatchdog RejectWatchdog PrepareSource RejectSource \
    Register PublishPark PublishTerminal ChangeEpisode Wake UnrelatedWake \
    Recheck ObserveTerminal AdvanceTime Abandon
run_expected_violation CheckedWait CheckedWait_rejection_unsafe.cfg \
    PendingRetainsOwnerAndLoan UnsafeRejectedRelease
run_expected_violation CheckedWait CheckedWait_replay_unsafe.cfg \
    OneSubmission UnsafeReplay
run_expected_violation CheckedWait CheckedWait_deadline_unsafe.cfg \
    DeadlineUnchanged UnsafeRestartDeadline
run_expected_violation CheckedWait CheckedWait_result_unsafe.cfg \
    ResultNeedsTerminalProof UnsafeWakeAdoptsResult
run_expected_violation CheckedWait CheckedWait_park_unsafe.cfg \
    ParkedTerminalHasWake UnsafeParkWithoutRecheck
run_expected_violation CheckedWait CheckedWait_timeout_unsafe.cfg \
    TimeoutRespectsDeadline UnsafeAdmissionTimeout
run_model Scheduler Scheduler_small.cfg \
    Spawn Admit Dispatch Preempt Block Wake SwitchOff Migrate \
    RequestRemoteAbort RetireRemoteAbort AbortNotRunning SelfAbort Reap \
    BeginDomainAbort DestroyAddressSpace
run_expected_violation Scheduler Scheduler_unsafe.cfg \
    ReapOnlyOffCpu UnsafeRemoteAbort
run_expected_violation Scheduler Scheduler_domain_abort_unsafe.cfg \
    AbortingThreadsDoomed UnsafeSpawnDuringAbort
run_model StackGrowth StackGrowth_small.cfg \
    Fault Grow Kill Exit
run_expected_violation StackGrowth \
    StackGrowth_grow_unsafe.cfg \
    CommittedWithinBudget UnsafeGrowOverBudget
run_expected_violation StackGrowth \
    StackGrowth_leak_unsafe.cfg \
    DeadDomainsReleaseFrames UnsafeKillWithoutRelease
run_model ThreadJoin ThreadJoin_small.cfg \
    Spawn RejectExhaustedSpawn CaptureHandle Exit ObserveJoin Reap
run_expected_violation ThreadJoin ThreadJoin_unsafe.cfg \
    ObserverMatchesCapturedHandle UnsafeObserveJoin
run_expected_violation ThreadJoin ThreadJoin_wrap_unsafe.cfg \
    GenerationNeverReused UnsafeSpawnWrap
run_model AddressSpace AddressSpace_small.cfg \
    Allocate CaptureHandle CloseExact
run_expected_violation AddressSpace AddressSpace_unsafe.cfg \
    ReplacementSurvivesStaleHandle UnsafeStaleClose
run_model RootRetirement RootRetirement_small.cfg \
    Allocate Acquire FinishOperation AbandonOperation BeginClose ClosePending \
    FinishCleanup Detach BeginInvalidation InvalidationFailed RetainForRetry \
    ClaimRetry InvalidationSucceeded StartPhysical FinishPhysical Abandon
run_expected_violation RootRetirement RootRetirement_lease_unsafe.cfg \
    LeasesRetainRoot UnsafeDetachWithLease
run_expected_violation RootRetirement RootRetirement_release_unsafe.cfg \
    PhysicalRequiresInvalidation UnsafeReleaseBeforeInvalidation
run_expected_violation RootRetirement RootRetirement_physical_unsafe.cfg \
    PhysicalOnce UnsafeRetryPhysical
run_expected_violation RootRetirement RootRetirement_abandon_unsafe.cfg \
    TerminalAbandonment UnsafeAdoptAbandoned
run_expected_violation RootRetirement RootRetirement_refund_unsafe.cfg \
    BackingCharged UnsafeRefundPartial
run_model HardwareAsid HardwareAsid_small.cfg \
    Allocate CacheTranslation Retire Invalidate
run_expected_violation HardwareAsid HardwareAsid_unsafe.cfg \
    NoDirtyTagReuse UnsafeAllocate
run_model InterruptRoute InterruptRoute_small.cfg \
    Bind QueueWake Unbind DrainSafe
run_expected_violation InterruptRoute InterruptRoute_unsafe.cfg \
    NoStaleWakeDelivery DrainUnsafe
run_model ServiceLifecycle ServiceLifecycle_small.cfg \
    StageTrusted StageUntrusted RejectUntrustedLoad Load Start Prepare \
    PublishLocal Activate RejectStaleActivate Lookup ClearLookup \
    FencedUnregister RejectStaleUnregister CleanupLocal RequestStop Exit DomainAbort Reap Teardown
run_expected_violation ServiceLifecycle ServiceLifecycle_unsafe.cfg \
    ReplacementSurvivesStaleUnregister UnsafeStaleUnregister
run_model Capability Capability_small.cfg \
    Allocate Remove DelegateCopy BeginMove CommitMove RollbackMove CloseAddressSpace
run_model Authorization Authorization_small.cfg \
    PublishService ReplaceService UnpublishService SetPolicy IssueTicket \
    CancelTicket Redeem CloseCapability
run_expected_violation Authorization \
    Authorization_policy_unsafe.cfg \
    PolicyMutationAuthorized UnsafeSetPolicy
run_expected_violation Authorization \
    Authorization_principal_unsafe.cfg \
    MintBoundToPrincipal UnsafeRedeemOtherPrincipal
run_expected_violation Authorization \
    Authorization_policy_version_unsafe.cfg \
    MintUsesCurrentPolicy UnsafeRedeemStalePolicy
run_expected_violation Authorization \
    Authorization_generation_unsafe.cfg \
    MintTargetsCurrentBinding UnsafeRedeemStaleBinding
run_expected_violation Authorization \
    Authorization_rights_unsafe.cfg \
    NoRightsAmplification UnsafeAmplifyRights
run_model Dma Dma_small.cfg \
    CreateMemory CreateDomain CpuMap CpuUnmap BeginLoan EndLoan BeginMap \
    CommitMap QuarantineMap FailMap RevokeMap ReleasePin \
    BeginDestroy AcknowledgeDestroy QuarantineDestroy FinalizeDomain \
    CloseMemory ExitDriver ReclaimMemory
run_expected_violation Dma Dma_unsafe.cfg \
    ExclusiveDmaHasNoCpuAuthority UnsafeBeginExclusiveMap
run_model DmaOperation DmaOperation_small.cfg \
    BeginMap ClaimMap PrepareInterest RejectSourcePreparation RejectContext \
    LinkInterest ReadyDuringPreparation RejectWaiter Park ObserveClosedSource \
    RestoreUnit NotifySource CompetingClaim AbandonExternal CleanupUnstarted \
    FinishMap CompleteMapPublic RejectMapPublicCompletion BeginUnmap \
    RejectUnmapAdmission ClaimUnmap FinishUnmap CompleteUnmapPublic \
    RejectUnmapPublicCompletion CpuUnmapFailure CpuUnmapSuccess \
    FailedSharedCpuUnmap FreshCpuMap PrepareLocalDrop DropLocal ReleaseBacking RejectRootClose Abandon
run_expected_violation DmaOperation DmaOperation_cell_unsafe.cfg \
    ClaimNeedsCompleteUnit UnsafeClaimDetachedCell
run_expected_violation DmaOperation DmaOperation_readiness_unsafe.cfg \
    ReadinessIsNotCompletion UnsafeReadinessCompletes
run_expected_violation DmaOperation DmaOperation_preparation_unsafe.cfg \
    PreclaimRetainsPreparation UnsafeRejectDropsPreparation
run_expected_violation DmaOperation DmaOperation_map_unsafe.cfg \
    MapSubmittedOnce UnsafeReplayMap
run_expected_violation DmaOperation DmaOperation_rearm_unsafe.cfg \
    StartedFailureNotRearmed UnsafeRearmStartedUnmap
run_expected_violation DmaOperation DmaOperation_drop_unsafe.cfg \
    UncertainAttemptNotResubmitted UnsafeDropResubmits
run_expected_violation DmaOperation DmaOperation_confirmation_unsafe.cfg \
    ConfirmationSticky UnsafeForgetDmaConfirmation
run_expected_violation DmaOperation DmaOperation_access_unsafe.cfg \
    FailedSharedRestricted UnsafeFailedSharedAccess
run_expected_violation DmaOperation DmaOperation_cpu_unsafe.cfg \
    PossibleDmaRetainsBacking UnsafeCpuUnmapDischargesDma
run_expected_violation DmaOperation DmaOperation_abandon_unsafe.cfg \
    AbandonmentRetainsOwner UnsafeAbandonReleases
run_expected_violation DmaOperation DmaOperation_context_unsafe.cfg \
    WaitNeedsQualifiedContext UnsafeWaitMasked
run_expected_violation DmaOperation DmaOperation_public_unsafe.cfg \
    PublicRejectionRetainsAttempt UnsafeFinalBusyRearms
run_expected_violation DmaOperation DmaOperation_remap_unsafe.cfg \
    CpuOwnerNeedsNoDma UnsafeCoherentRemap
run_model Raft Raft_small.cfg \
    StartElection GrantVote BecomeLeader ObserveHigherTerm Crash Restart
run_model RaftLog RaftLog_small.cfg \
    Elect AppendLeader ReplicateOne CommitLeader PropagateCommit Crash Restart
run_model RaftResponse RaftResponse_small.cfg \
    CaptureOldReply ReplacePrefix Reelect Replicate Acknowledge Failure \
    RejectOld RejectInvalid Commit ObserveHigherTerm
run_expected_violation RaftResponse \
    RaftResponse_oldterm_unsafe.cfg \
    CommitHasCurrentPrefix UnsafeOldTermAck
run_expected_violation RaftResponse \
    RaftResponse_regress_unsafe.cfg \
    ConfirmedProgressRetained UnsafeRegressSuccess
run_expected_violation RaftResponse \
    RaftResponse_backoff_unsafe.cfg \
    BackoffRespectsConfirmedPrefix UnsafeBackoff
run_expected_violation RaftResponse \
    RaftResponse_invalid_unsafe.cfg \
    AcknowledgementsHaveProof UnsafeInvalidCredit
run_model RaftRead RaftRead_small.cfg \
    Elect AdmitRead CaptureReply ReceiveOldPrefix CommitQuorum DeliverReply Apply Read RejectRead \
    CancelRead ChangeLeader StepDown ChangeEpoch ReplaceDomain
run_expected_violation RaftRead RaftRead_commit_unsafe.cfg \
    ReadHasCurrentCommit UnsafeReadBeforeCommit
run_expected_violation RaftRead RaftRead_apply_unsafe.cfg \
    ReadHasAppliedCommit UnsafeReadBeforeApply
run_expected_violation RaftRead RaftRead_lease_unsafe.cfg \
    ReadHasFenceProof UnsafeReceiveLease
run_expected_violation RaftRead RaftRead_binding_unsafe.cfg \
    ReadBoundToAdmission UnsafeReadBinding
run_model RaftMembership RaftMembership_small.cfg \
    Elect ElectNextOnly SubmitJoint Replicate CommitJoint SubmitFinalize \
    RejectLaggingFinalize CommitFinalize Crash Restart
run_expected_violation RaftMembership \
    RaftMembership_beforecatchup_unsafe.cfg \
    ProposedMembersCaughtUp UnsafeFinalizeBeforeCatchUp
run_expected_violation RaftMembership \
    RaftMembership_learnerleader_unsafe.cfg \
    LeaderIsEligible UnsafeFinalizeLearnerLeader
run_expected_violation RaftMembership \
    RaftMembership_oldonly_unsafe.cfg \
    FinalizationHasBothQuorums UnsafeFinalizeOldOnly
run_model RaftJoin RaftJoin_small.cfg \
    Elect BeginJoining Crash Restart SubmitJoin CommitJoin ReplicateToJoiner SubmitJoint CommitJoint
run_expected_violation RaftJoin RaftJoin_unsafe.cfg \
    JoiningAcceptsOnlySelectedAnchor UnsafeReplicateToJoiner
run_expected_violation RaftJoin RaftJoin_restart_unsafe.cfg \
    RestartPreservesAdmission UnsafeRestartForgetsAdmission
run_model RaftSnapshot RaftSnapshot_small.cfg \
    AppendLog Commit BeginReceive ReceiveChunk PersistSnapshot ActivateSnapshot \
    DiscardStale Crash Restart
run_model RaftStream RaftStream_small.cfg \
    BeginReceive RestartOffsetZero ChangeTerm RecognizeLeader ReceiveChunk \
    CancelReceive RejectStale RejectLeader RejectOffset RejectBoundary
run_expected_violation RaftStream RaftStream_owner_unsafe.cfg \
    InstalledSingleOwner ReceiveChunk
run_expected_violation RaftStream RaftStream_offset_unsafe.cfg \
    InstalledInOrder ReceiveChunk
run_model SnapshotRestore SnapshotRestore_small.cfg \
    ReadHeader ParseSections RejectUnsupported RejectMalformed Validate Restore \
    Persist DefinitePersistenceFailure AmbiguousPersistenceFailure \
    ActivateStreamJoin ActivateStreamRetained \
    ActivateRecoveryJoin ActivateRecoveryRetained Crash BeginRecovery
run_expected_violation SnapshotRestore \
    SnapshotRestore_application_unsafe.cfg \
    RejectionPreservesApplication UnsafeRejectApplication
run_expected_violation SnapshotRestore \
    SnapshotRestore_durable_unsafe.cfg \
    RejectionPreservesDurable UnsafeRejectDurable
run_expected_violation SnapshotRestore \
    SnapshotRestore_progress_unsafe.cfg \
    RejectionPreservesProgress UnsafeRejectProgress
run_expected_violation SnapshotRestore \
    SnapshotRestore_membership_unsafe.cfg \
    RejectionPreservesMembership UnsafeRejectMembership
run_expected_violation SnapshotRestore \
    SnapshotRestore_join_unsafe.cfg \
    RejectionPreservesJoin UnsafeRejectJoin
run_expected_violation SnapshotRestore \
    SnapshotRestore_exposure_unsafe.cfg \
    RecoveryExposureRequiresRestore UnsafeExposeRecovery
run_expected_violation SnapshotRestore \
    SnapshotRestore_ordering_unsafe.cfg \
    PublicationRequiresRestore UnsafePublishBeforeRestore
run_model ClusterIngress ClusterIngress_small.cfg \
    PublishReady WithdrawReady ReplaceDeployment CommitDrain BeginJoint \
    FinalizeJoint LearnRoute ForgetRoute InstallSnapshot StartNewFlow \
    ExpireLease ExistingFlowPacket DropStaleNewFlow DropUnretainedFlow \
    DropWithdrawnFlow DropDepartedBackend EndFlow
run_expected_violation ClusterIngress \
    ClusterIngress_stale_snapshot_unsafe.cfg \
    NewFlowsRequireFreshLease UnsafeStartStaleFlow
run_expected_violation ClusterIngress \
    ClusterIngress_history_unsafe.cfg \
    FlowBackendStable UnsafeFallbackExistingPacket
run_expected_violation ClusterIngress \
    ClusterIngress_readiness_unsafe.cfg \
    CommittedPoliciesAreDerived UnsafeAdmitStaleReadiness
run_model PlacementControl PlacementControl_small.cfg \
    Observe SelectCandidate Tick PressureMove RemoveEligibility ForcedMove Admit
run_expected_violation PlacementControl \
    PlacementControl_dwell_unsafe.cfg \
    PressureRespectsStability UnsafeEarlyPressureMove
run_expected_violation PlacementControl \
    PlacementControl_reserve_unsafe.cfg \
    ReservationsPreserveReserve UnsafeAdmit
run_model RemoteCall RemoteCall_small.cfg \
    Start ReplaceTarget Execute RejectStale QueueReply DuplicateRequest \
    DeliverReply Timeout SettleTransport RetireUncertainSession Evict
run_model ReliableMessage ReliableMessage_small.cfg \
    AbandonSession RestartService AcceptCurrentSession
run_expected_violation ReliableMessage \
    ReliableMessage_identity_unsafe.cfg \
    SessionIdentityUnique RestartService
run_expected_violation ReliableMessage \
    ReliableMessage_regression_unsafe.cfg \
    ReceiveSessionMonotonic AcceptDelayedSession

if [[ "${selected_cases}" -eq 0 ]]; then
    echo "error: no configurations registered for ${selected_module}" >&2
    exit 1
fi
