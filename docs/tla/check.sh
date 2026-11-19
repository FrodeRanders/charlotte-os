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
run_model KnytosPrincipalHistory KnytosPrincipalHistory_small.cfg \
    Prepare Attach Decide Exit Confirm ReleaseRoot Cancel Abandon
run_expected_violation KnytosPrincipalHistory KnytosPrincipalHistory_ordinary_unsafe.cfg \
    OrdinaryCeiling UnsafeIgnoreOrdinary
run_expected_violation KnytosPrincipalHistory KnytosPrincipalHistory_exit_unsafe.cfg \
    Accounted UnsafeExitRefund
run_expected_violation KnytosPrincipalHistory KnytosPrincipalHistory_drop_unsafe.cfg \
    Accounted UnsafeDropRefund
run_expected_violation KnytosPrincipalHistory KnytosPrincipalHistory_class_unsafe.cfg \
    OriginalClass UnsafeReclassify
run_expected_violation KnytosPrincipalHistory KnytosPrincipalHistory_evict_unsafe.cfg \
    HistoryRetained UnsafeEvict
run_expected_violation KnytosPrincipalHistory KnytosPrincipalHistory_early_unsafe.cfg \
    RootProof UnsafeRootFree
run_model KnytosThreadMetadata KnytosThreadMetadata_small.cfg \
    Admit AllocateNode AllocateContext Publish DisposeContext FreeNode Refund Abandon Retire Reuse
run_expected_violation KnytosThreadMetadata KnytosThreadMetadata_generation_unsafe.cfg \
    PrincipalCeiling UnsafeGenerationOnly
run_expected_violation KnytosThreadMetadata KnytosThreadMetadata_early_unsafe.cfg \
    Accounted UnsafeEarlyRefund
run_expected_violation KnytosThreadMetadata KnytosThreadMetadata_drop_unsafe.cfg \
    Accounted UnsafeDropRefund
run_expected_violation KnytosThreadMetadata KnytosThreadMetadata_reuse_unsafe.cfg \
    OriginalSponsor UnsafeReplacementSponsor
run_model KnytosPrincipalBacking KnytosPrincipalBacking_small.cfg \
    Allocate Grow Move Retire Restart ConfirmRelease
run_expected_violation KnytosPrincipalBacking KnytosPrincipalBacking_generation_unsafe.cfg \
    PrincipalCeiling UnsafeGenerationOnly
run_expected_violation KnytosPrincipalBacking KnytosPrincipalBacking_refund_unsafe.cfg \
    Accounted UnsafeRetirementRefund
run_expected_violation KnytosPrincipalBacking KnytosPrincipalBacking_move_unsafe.cfg \
    Accounted UnsafeMoveSponsor
run_expected_violation KnytosPrincipalBacking KnytosPrincipalBacking_reuse_unsafe.cfg \
    Accounted UnsafeReuseSponsor
run_expected_violation KnytosPrincipalBacking KnytosPrincipalBacking_growth_unsafe.cfg \
    Accounted UnsafeGrowCharge
run_expected_violation KnytosPrincipalBacking KnytosPrincipalBacking_drop_unsafe.cfg \
    Accounted UnsafeDropRefund
run_model KnytosControllerRelay KnytosControllerRelay_small.cfg \
    ProducerReply CheckedPreflight RejectPreflight ObserveTerminal Expire ReleaseReceiver Park Wake Tick AbortBindingOrWait
run_expected_violation KnytosControllerRelay KnytosControllerRelay_unknown_unsafe.cfg \
    MutationRequiresCheckedQuery UnsafeUnknownAsMissing
run_expected_violation KnytosControllerRelay KnytosControllerRelay_budget_unsafe.cfg \
    OriginalBudget UnsafeRestart
run_expected_violation KnytosControllerRelay KnytosControllerRelay_serial_unsafe.cfg \
    ParkCoversAll UnsafeSerialWait
run_expected_violation KnytosControllerRelay KnytosControllerRelay_replay_unsafe.cfg \
    AcceptedOnce UnsafeReplay
run_model KnytosDeploymentListener KnytosDeploymentListener_small.cfg \
    Admit RejectQueue ProducerReply Observe RejectConfirmed Abort Park Wake Tick
run_expected_violation KnytosDeploymentListener KnytosDeploymentListener_replay_unsafe.cfg \
    AcceptedOnce UnsafeReplay
run_expected_violation KnytosDeploymentListener KnytosDeploymentListener_budget_unsafe.cfg \
    OriginalBudget UnsafeRestart
run_expected_violation KnytosDeploymentListener KnytosDeploymentListener_serial_unsafe.cfg \
    ParkCoversAll UnsafeSerialPark
run_model KnytosNamespaceDrain KnytosNamespaceDrain_small.cfg \
    Select SelectEmpty ClosePeer FinishClaim CancelSelected RejectPreparation RecheckMissing ReturnPending
run_expected_violation KnytosNamespaceDrain KnytosNamespaceDrain_stale_unsafe.cfg \
    NoStalePending UnsafeStalePending
run_expected_violation KnytosNamespaceDrain KnytosNamespaceDrain_skip_unsafe.cfg \
    CompleteOnlyDrained UnsafeSkipLive
run_model KnytosDeploymentClose KnytosDeploymentClose_blocking.cfg \
    RejectQueue Admit ProducerAck ProducerUnavailable ObserveAck Expire RejectWaitOrObservation Park Wake Tick RetainAdmission
run_expected_violation KnytosDeploymentClose KnytosDeploymentClose_fallback_unsafe.cfg \
    NoReentry UnsafeFallback
run_model KnytosDeploymentClose KnytosDeploymentClose_small.cfg \
    RejectQueue Admit ProducerAck ProducerUnavailable ObserveAck Expire RejectWaitOrObservation Park Wake Tick
run_expected_violation KnytosDeploymentClose KnytosDeploymentClose_replay_unsafe.cfg \
    AcceptedOnce UnsafeReplay
run_expected_violation KnytosDeploymentClose KnytosDeploymentClose_release_unsafe.cfg \
    OwnersUntilAck UnsafeRelease
run_model KnytosDeploymentOutput KnytosDeploymentOutput_small.cfg \
    ProducerReceipt ObservePrefix EndUncertain Tick Park Wake RejectWait
run_expected_violation KnytosDeploymentOutput KnytosDeploymentOutput_replay_unsafe.cfg \
    PrefixesOnce UnsafeReplay
run_expected_violation KnytosDeploymentOutput KnytosDeploymentOutput_budget_unsafe.cfg \
    OriginalBudget UnsafeRestartBudget
run_model KnytosDeploymentControl KnytosDeploymentControl_small.cfg \
    Bind Complete Observe Expire ReleaseReceiver Park Tick RejectAdmission
run_expected_violation KnytosDeploymentControl KnytosDeploymentControl_replay_unsafe.cfg \
    SubmittedOnce UnsafeReplay
run_expected_violation KnytosDeploymentControl KnytosDeploymentControl_serial_unsafe.cfg \
    ParkCoversAll UnsafeSerialWait
run_model KnytosDeploymentInput KnytosDeploymentInput_small.cfg \
    Advance ProducerReady Tick Park RejectWait
run_expected_violation KnytosDeploymentInput KnytosDeploymentInput_cursor_unsafe.cfg \
    HealthyTurnBound UnsafeKeepCursor
run_expected_violation KnytosDeploymentInput KnytosDeploymentInput_deadline_unsafe.cfg \
    OriginalBudget UnsafeRestartBudget
run_model KnytosIPC KnytosIPC_small.cfg \
    MemoryCreate ScalarCallMove ScalarCallBorrowRead ScalarCallBorrowWrite \
    ScalarCallCopy Receive ReplyReturnMemory EndpointClose \
    DomainTeardown
run_model KnytosIpcTransaction KnytosIpcTransaction_small.cfg \
    BeginVector TransferMove BeginBorrow AttachConnection Commit \
    FailAndRollback Deliver WaitTimeout Reply ObserveReply
run_expected_violation KnytosIpcTransaction \
    KnytosIpcTransaction_rollback_unsafe.cfg \
    RollbackRestoresAll UnsafeRollback
run_expected_violation KnytosIpcTransaction \
    KnytosIpcTransaction_timeout_unsafe.cfg \
    TimeoutRetainsLoan UnsafeTimeoutRelease
run_model KnytosIpcPublication KnytosIpcPublication_small.cfg \
    Begin PrepareRecord RejectStorage Reserve RejectReservation PrepareMemory \
    PrepareConnection Commit RejectCommit FinishPreparation AbortPreparation \
    Readiness PrepareReceive RejectReceive ReceiveWriteFailed CommitReceive \
    ObserveReply RepeatObservation Use ApplicationClose CancelHidden Fill \
    RetireNamespace DetachNamespace ReplaceNamespace ReleaseRetired Abandon
run_expected_violation KnytosIpcPublication KnytosIpcPublication_memory_unsafe.cfg \
    UseRequiresDelivery UnsafeUseMemory
run_expected_violation KnytosIpcPublication KnytosIpcPublication_connection_unsafe.cfg \
    UseRequiresDelivery UnsafeUseConnection
run_expected_violation KnytosIpcPublication KnytosIpcPublication_batch_unsafe.cfg \
    AtomicPublication UnsafePublishPrefix
run_expected_violation KnytosIpcPublication KnytosIpcPublication_serial_unsafe.cfg \
    SerialsConsumed UnsafeReuseSerial
run_expected_violation KnytosIpcPublication KnytosIpcPublication_cancel_unsafe.cfg \
    ReplacementIntact UnsafeCancelSuccessor
run_expected_violation KnytosIpcPublication KnytosIpcPublication_restore_unsafe.cfg \
    ReplacementIntact UnsafeRestoreSuccessor
run_expected_violation KnytosIpcPublication KnytosIpcPublication_refund_unsafe.cfg \
    RetainedNodesCharged UnsafeRefundRetained
run_expected_violation KnytosIpcPublication KnytosIpcPublication_readiness_unsafe.cfg \
    VisibleRequiresDelivery UnsafeReadinessDelivers
run_expected_violation KnytosIpcPublication KnytosIpcPublication_observe_unsafe.cfg \
    ObservedCloseStaysClosed UnsafeRepublishObserved
run_expected_violation KnytosIpcPublication KnytosIpcPublication_rollback_unsafe.cfg \
    RollbackRestoresCapturedSource UnsafeRollbackNeedsAdmission
run_model KnytosEndpointObservers KnytosEndpointObservers_small.cfg \
    ArmReadiness ArmCloseWatch Send Receive Close ObserveClose
run_expected_violation KnytosEndpointObservers KnytosEndpointObservers_unsafe.cfg \
    CloseSignalImpliesClosed UnsafeMessageWake
run_model KnytosNamePublication KnytosNamePublication_small.cfg \
    Begin RejectBeforeWatch PrepareWatch RejectPolicy PublishPolicy Install \
    Lookup QueueLookup ResolveDeferred FinishInstall CloseEndpoint CqHint \
    ObservePending ObserveUnrelated RetireDeath RetireExplicit RejectRetirement \
    CancelWatch CloseSource WatchError WaitRejected LifecycleAbort PublicationAbort
run_model KnytosNamePublication KnytosNamePublication_exhaustion.cfg \
    Begin PrepareWatch PublishPolicy Install FinishInstall ExhaustionRetire \
    CancelWatch CloseSource RejectExhausted
run_expected_violation KnytosNamePublication KnytosNamePublication_public_unsafe.cfg \
    PublicIssuance UnsafePublicLookup
run_expected_violation KnytosNamePublication KnytosNamePublication_deferred_unsafe.cfg \
    PublicIssuance UnsafeDeferredLookup
run_expected_violation KnytosNamePublication KnytosNamePublication_replacement_unsafe.cfg \
    ReplacementProtected UnsafePublicReplacement
run_expected_violation KnytosNamePublication KnytosNamePublication_watch_unsafe.cfg \
    BeforeWatchUnchanged UnsafeMutationBeforeWatch
run_expected_violation KnytosNamePublication KnytosNamePublication_hint_unsafe.cfg \
    DeathNeedsExactResult UnsafeHintRetires
run_expected_violation KnytosNamePublication KnytosNamePublication_stale_unsafe.cfg \
    DeathNeedsExactResult UnsafeStaleHint
run_expected_violation KnytosNamePublication KnytosNamePublication_disposal_unsafe.cfg \
    WatchRetainsSource UnsafeCloseSourceFirst
run_expected_violation KnytosNamePublication KnytosNamePublication_quota_unsafe.cfg \
    LiveBound UnsafeNewAtCapacity
run_expected_violation KnytosNamePublication KnytosNamePublication_abort_unsafe.cfg \
    AbortRetainsOwners UnsafeAbortDisposes
run_expected_violation KnytosNamePublication KnytosNamePublication_retire_unsafe.cfg \
    RetirementAuthorized UnsafeUnauthorizedRetire
run_expected_violation KnytosNamePublication KnytosNamePublication_wrap_unsafe.cfg \
    GenerationNotReused UnsafeGenerationWrap
run_expected_violation KnytosNamePublication KnytosNamePublication_exhausted_unsafe.cfg \
    PolicyGenerationRepresentable UnsafePublishExhausted
run_expected_violation KnytosNamePublication KnytosNamePublication_serial_unsafe.cfg \
    GrantGenerationCoherent UnsafeLookupDuringPublish
run_model KnytosBootTrust KnytosBootTrust_fixture.cfg \
    ReadInstalled ReadFailure Verify RejectRecord AcquireRecipient PrepareSigned \
    RejectRecipient PrepareDevelopment SelectAuthority LoadBootstrap LoadPlatform \
    Publish RejectPublication BorrowView DropPreparation Reboot
run_model KnytosBootTrust KnytosBootTrust_contract.cfg \
    ReadInstalled ReadEnrollment ReadFailure Verify RejectRecord CommitPolicy \
    CommitFailure CommitInterrupted CompetingCommit AcquireRecipient PrepareSigned \
    RejectRecipient SelectAuthority LoadBootstrap LoadPlatform \
    Publish RejectPublication BorrowView DropPreparation Reboot
run_expected_violation KnytosBootTrust KnytosBootTrust_uninstalled_unsafe.cfg \
    InstalledBeforePublication UnsafeUninstalledPublish
run_expected_violation KnytosBootTrust KnytosBootTrust_stale_unsafe.cfg \
    NoForkedInstallation UnsafeStaleCommit
run_expected_violation KnytosBootTrust KnytosBootTrust_enrollment_unsafe.cfg \
    NoEnrollmentFallback UnsafeEnrollmentFallback
run_expected_violation KnytosBootTrust KnytosBootTrust_overwrite_unsafe.cfg \
    OnePublicationPerBoot UnsafeOverwrite
run_expected_violation KnytosBootTrust KnytosBootTrust_manifest_unsafe.cfg \
    SharedPolicy UnsafeIndependentManifest
run_expected_violation KnytosBootTrust KnytosBootTrust_recipient_unsafe.cfg \
    RecipientBound UnsafeWrongRecipient
run_expected_violation KnytosBootTrust KnytosBootTrust_authenticate_unsafe.cfg \
    VerifiedAuthenticated UnsafeAuthenticate
run_expected_violation KnytosBootTrust KnytosBootTrust_development_unsafe.cfg \
    ExplicitDevelopment UnsafeDevelopmentFallback
run_expected_violation KnytosBootTrust KnytosBootTrust_failure_unsafe.cfg \
    InstalledBeforePublication UnsafeFailedCommitPublishes
run_expected_violation KnytosBootTrust KnytosBootTrust_boundary_unsafe.cfg \
    ConditionalBoundary UnsafeUnsupportedProduction
run_expected_violation KnytosBootTrust KnytosBootTrust_platform_unsafe.cfg \
    PlatformImagesUseSelectedAuthority UnsafePlatformFallback
run_expected_violation KnytosBootTrust KnytosBootTrust_selection_unsafe.cfg \
    SelectedAuthorityMatchesState UnsafeSelectionOverwrite
run_model KnytosCQ KnytosCQ_mini.cfg \
    Complete Fail CancelOp DrainOne DrainAll ObserveResult CqWait CqWake TimerFire
run_expected_violation KnytosCQ KnytosCQ_buffer_unsafe.cfg \
    NonTerminalBufferRemainsLoaned UnsafeCancelReleasesBuffer
run_model KnytosTimedWait KnytosTimedWait_small.cfg \
    ArmWait PublishWork DeliverWake TimerFire Consume
run_expected_violation KnytosTimedWait KnytosTimedWait_unsafe.cfg \
    TimeoutObservedNoWork UnsafeTimerFire
run_model KnytosCheckedWait KnytosCheckedWait_small.cfg \
    BeginWait PrepareWatchdog RejectWatchdog PrepareSource RejectSource \
    Register PublishPark PublishTerminal ChangeEpisode Wake UnrelatedWake \
    Recheck ObserveTerminal AdvanceTime Abandon
run_expected_violation KnytosCheckedWait KnytosCheckedWait_rejection_unsafe.cfg \
    PendingRetainsOwnerAndLoan UnsafeRejectedRelease
run_expected_violation KnytosCheckedWait KnytosCheckedWait_replay_unsafe.cfg \
    OneSubmission UnsafeReplay
run_expected_violation KnytosCheckedWait KnytosCheckedWait_deadline_unsafe.cfg \
    DeadlineUnchanged UnsafeRestartDeadline
run_expected_violation KnytosCheckedWait KnytosCheckedWait_result_unsafe.cfg \
    ResultNeedsTerminalProof UnsafeWakeAdoptsResult
run_expected_violation KnytosCheckedWait KnytosCheckedWait_park_unsafe.cfg \
    ParkedTerminalHasWake UnsafeParkWithoutRecheck
run_expected_violation KnytosCheckedWait KnytosCheckedWait_timeout_unsafe.cfg \
    TimeoutRespectsDeadline UnsafeAdmissionTimeout
run_model KnytosScheduler KnytosScheduler_small.cfg \
    Spawn Admit Dispatch Preempt Block Wake SwitchOff Migrate \
    RequestRemoteAbort RetireRemoteAbort AbortNotRunning SelfAbort Reap \
    BeginDomainAbort DestroyAddressSpace
run_expected_violation KnytosScheduler KnytosScheduler_unsafe.cfg \
    ReapOnlyOffCpu UnsafeRemoteAbort
run_expected_violation KnytosScheduler KnytosScheduler_domain_abort_unsafe.cfg \
    AbortingThreadsDoomed UnsafeSpawnDuringAbort
run_model KnytosStackGrowth KnytosStackGrowth_small.cfg \
    Fault Grow Kill Exit
run_expected_violation KnytosStackGrowth \
    KnytosStackGrowth_grow_unsafe.cfg \
    CommittedWithinBudget UnsafeGrowOverBudget
run_expected_violation KnytosStackGrowth \
    KnytosStackGrowth_leak_unsafe.cfg \
    DeadDomainsReleaseFrames UnsafeKillWithoutRelease
run_model KnytosThreadJoin KnytosThreadJoin_small.cfg \
    Spawn RejectExhaustedSpawn CaptureHandle Exit ObserveJoin Reap
run_expected_violation KnytosThreadJoin KnytosThreadJoin_unsafe.cfg \
    ObserverMatchesCapturedHandle UnsafeObserveJoin
run_expected_violation KnytosThreadJoin KnytosThreadJoin_wrap_unsafe.cfg \
    GenerationNeverReused UnsafeSpawnWrap
run_model KnytosAddressSpace KnytosAddressSpace_small.cfg \
    Allocate CaptureHandle CloseExact
run_expected_violation KnytosAddressSpace KnytosAddressSpace_unsafe.cfg \
    ReplacementSurvivesStaleHandle UnsafeStaleClose
run_model KnytosRootRetirement KnytosRootRetirement_small.cfg \
    Allocate Acquire FinishOperation AbandonOperation BeginClose ClosePending \
    FinishCleanup Detach BeginInvalidation InvalidationFailed RetainForRetry \
    ClaimRetry InvalidationSucceeded StartPhysical FinishPhysical Abandon
run_expected_violation KnytosRootRetirement KnytosRootRetirement_lease_unsafe.cfg \
    LeasesRetainRoot UnsafeDetachWithLease
run_expected_violation KnytosRootRetirement KnytosRootRetirement_release_unsafe.cfg \
    PhysicalRequiresInvalidation UnsafeReleaseBeforeInvalidation
run_expected_violation KnytosRootRetirement KnytosRootRetirement_physical_unsafe.cfg \
    PhysicalOnce UnsafeRetryPhysical
run_expected_violation KnytosRootRetirement KnytosRootRetirement_abandon_unsafe.cfg \
    TerminalAbandonment UnsafeAdoptAbandoned
run_expected_violation KnytosRootRetirement KnytosRootRetirement_refund_unsafe.cfg \
    BackingCharged UnsafeRefundPartial
run_model KnytosHardwareAsid KnytosHardwareAsid_small.cfg \
    Allocate CacheTranslation Retire Invalidate
run_expected_violation KnytosHardwareAsid KnytosHardwareAsid_unsafe.cfg \
    NoDirtyTagReuse UnsafeAllocate
run_model KnytosInterruptRoute KnytosInterruptRoute_small.cfg \
    Bind QueueWake Unbind DrainSafe
run_expected_violation KnytosInterruptRoute KnytosInterruptRoute_unsafe.cfg \
    NoStaleWakeDelivery DrainUnsafe
run_model KnytosServiceLifecycle KnytosServiceLifecycle_small.cfg \
    StageTrusted StageUntrusted RejectUntrustedLoad Load Start Prepare \
    PublishLocal Activate RejectStaleActivate Lookup ClearLookup \
    FencedUnregister RejectStaleUnregister CleanupLocal RequestStop Exit DomainAbort Reap Teardown
run_expected_violation KnytosServiceLifecycle KnytosServiceLifecycle_unsafe.cfg \
    ReplacementSurvivesStaleUnregister UnsafeStaleUnregister
run_model KnytosCapability KnytosCapability_small.cfg \
    Allocate Remove DelegateCopy BeginMove CommitMove RollbackMove CloseAddressSpace
run_model KnytosAuthorization KnytosAuthorization_small.cfg \
    PublishService ReplaceService UnpublishService SetPolicy IssueTicket \
    CancelTicket Redeem CloseCapability
run_expected_violation KnytosAuthorization \
    KnytosAuthorization_policy_unsafe.cfg \
    PolicyMutationAuthorized UnsafeSetPolicy
run_expected_violation KnytosAuthorization \
    KnytosAuthorization_principal_unsafe.cfg \
    MintBoundToPrincipal UnsafeRedeemOtherPrincipal
run_expected_violation KnytosAuthorization \
    KnytosAuthorization_policy_version_unsafe.cfg \
    MintUsesCurrentPolicy UnsafeRedeemStalePolicy
run_expected_violation KnytosAuthorization \
    KnytosAuthorization_generation_unsafe.cfg \
    MintTargetsCurrentBinding UnsafeRedeemStaleBinding
run_expected_violation KnytosAuthorization \
    KnytosAuthorization_rights_unsafe.cfg \
    NoRightsAmplification UnsafeAmplifyRights
run_model KnytosDMA KnytosDMA_small.cfg \
    CreateMemory CreateDomain CpuMap CpuUnmap BeginLoan EndLoan BeginMap \
    CommitMap QuarantineMap FailMap RevokeMap ReleasePin \
    BeginDestroy AcknowledgeDestroy QuarantineDestroy FinalizeDomain \
    CloseMemory ExitDriver ReclaimMemory
run_expected_violation KnytosDMA KnytosDMA_unsafe.cfg \
    ExclusiveDmaHasNoCpuAuthority UnsafeBeginExclusiveMap
run_model KnytosDmaOperation KnytosDmaOperation_small.cfg \
    BeginMap ClaimMap PrepareInterest RejectSourcePreparation RejectContext \
    LinkInterest ReadyDuringPreparation RejectWaiter Park ObserveClosedSource \
    RestoreUnit NotifySource CompetingClaim AbandonExternal CleanupUnstarted \
    FinishMap CompleteMapPublic RejectMapPublicCompletion BeginUnmap \
    RejectUnmapAdmission ClaimUnmap FinishUnmap CompleteUnmapPublic \
    RejectUnmapPublicCompletion CpuUnmapFailure CpuUnmapSuccess \
    FailedSharedCpuUnmap FreshCpuMap PrepareLocalDrop DropLocal ReleaseBacking RejectRootClose Abandon
run_expected_violation KnytosDmaOperation KnytosDmaOperation_cell_unsafe.cfg \
    ClaimNeedsCompleteUnit UnsafeClaimDetachedCell
run_expected_violation KnytosDmaOperation KnytosDmaOperation_readiness_unsafe.cfg \
    ReadinessIsNotCompletion UnsafeReadinessCompletes
run_expected_violation KnytosDmaOperation KnytosDmaOperation_preparation_unsafe.cfg \
    PreclaimRetainsPreparation UnsafeRejectDropsPreparation
run_expected_violation KnytosDmaOperation KnytosDmaOperation_map_unsafe.cfg \
    MapSubmittedOnce UnsafeReplayMap
run_expected_violation KnytosDmaOperation KnytosDmaOperation_rearm_unsafe.cfg \
    StartedFailureNotRearmed UnsafeRearmStartedUnmap
run_expected_violation KnytosDmaOperation KnytosDmaOperation_drop_unsafe.cfg \
    UncertainAttemptNotResubmitted UnsafeDropResubmits
run_expected_violation KnytosDmaOperation KnytosDmaOperation_confirmation_unsafe.cfg \
    ConfirmationSticky UnsafeForgetDmaConfirmation
run_expected_violation KnytosDmaOperation KnytosDmaOperation_access_unsafe.cfg \
    FailedSharedRestricted UnsafeFailedSharedAccess
run_expected_violation KnytosDmaOperation KnytosDmaOperation_cpu_unsafe.cfg \
    PossibleDmaRetainsBacking UnsafeCpuUnmapDischargesDma
run_expected_violation KnytosDmaOperation KnytosDmaOperation_abandon_unsafe.cfg \
    AbandonmentRetainsOwner UnsafeAbandonReleases
run_expected_violation KnytosDmaOperation KnytosDmaOperation_context_unsafe.cfg \
    WaitNeedsQualifiedContext UnsafeWaitMasked
run_expected_violation KnytosDmaOperation KnytosDmaOperation_public_unsafe.cfg \
    PublicRejectionRetainsAttempt UnsafeFinalBusyRearms
run_expected_violation KnytosDmaOperation KnytosDmaOperation_remap_unsafe.cfg \
    CpuOwnerNeedsNoDma UnsafeCoherentRemap
run_model KnytosRaft KnytosRaft_small.cfg \
    StartElection GrantVote BecomeLeader ObserveHigherTerm Crash Restart
run_model KnytosRaftLog KnytosRaftLog_small.cfg \
    Elect AppendLeader ReplicateOne CommitLeader PropagateCommit Crash Restart
run_model KnytosRaftResponse KnytosRaftResponse_small.cfg \
    CaptureOldReply ReplacePrefix Reelect Replicate Acknowledge Failure \
    RejectOld RejectInvalid Commit ObserveHigherTerm
run_expected_violation KnytosRaftResponse \
    KnytosRaftResponse_oldterm_unsafe.cfg \
    CommitHasCurrentPrefix UnsafeOldTermAck
run_expected_violation KnytosRaftResponse \
    KnytosRaftResponse_regress_unsafe.cfg \
    ConfirmedProgressRetained UnsafeRegressSuccess
run_expected_violation KnytosRaftResponse \
    KnytosRaftResponse_backoff_unsafe.cfg \
    BackoffRespectsConfirmedPrefix UnsafeBackoff
run_expected_violation KnytosRaftResponse \
    KnytosRaftResponse_invalid_unsafe.cfg \
    AcknowledgementsHaveProof UnsafeInvalidCredit
run_model KnytosRaftRead KnytosRaftRead_small.cfg \
    Elect AdmitRead CaptureReply ReceiveOldPrefix CommitQuorum DeliverReply Apply Read RejectRead \
    CancelRead ChangeLeader StepDown ChangeEpoch ReplaceDomain
run_expected_violation KnytosRaftRead KnytosRaftRead_commit_unsafe.cfg \
    ReadHasCurrentCommit UnsafeReadBeforeCommit
run_expected_violation KnytosRaftRead KnytosRaftRead_apply_unsafe.cfg \
    ReadHasAppliedCommit UnsafeReadBeforeApply
run_expected_violation KnytosRaftRead KnytosRaftRead_lease_unsafe.cfg \
    ReadHasFenceProof UnsafeReceiveLease
run_expected_violation KnytosRaftRead KnytosRaftRead_binding_unsafe.cfg \
    ReadBoundToAdmission UnsafeReadBinding
run_model KnytosRaftMembership KnytosRaftMembership_small.cfg \
    Elect ElectNextOnly SubmitJoint Replicate CommitJoint SubmitFinalize \
    RejectLaggingFinalize CommitFinalize Crash Restart
run_expected_violation KnytosRaftMembership \
    KnytosRaftMembership_beforecatchup_unsafe.cfg \
    ProposedMembersCaughtUp UnsafeFinalizeBeforeCatchUp
run_expected_violation KnytosRaftMembership \
    KnytosRaftMembership_learnerleader_unsafe.cfg \
    LeaderIsEligible UnsafeFinalizeLearnerLeader
run_expected_violation KnytosRaftMembership \
    KnytosRaftMembership_oldonly_unsafe.cfg \
    FinalizationHasBothQuorums UnsafeFinalizeOldOnly
run_model KnytosRaftJoin KnytosRaftJoin_small.cfg \
    Elect BeginJoining Crash Restart SubmitJoin CommitJoin ReplicateToJoiner SubmitJoint CommitJoint
run_expected_violation KnytosRaftJoin KnytosRaftJoin_unsafe.cfg \
    JoiningAcceptsOnlySelectedAnchor UnsafeReplicateToJoiner
run_expected_violation KnytosRaftJoin KnytosRaftJoin_restart_unsafe.cfg \
    RestartPreservesAdmission UnsafeRestartForgetsAdmission
run_model KnytosRaftSnapshot KnytosRaftSnapshot_small.cfg \
    AppendLog Commit BeginReceive ReceiveChunk PersistSnapshot ActivateSnapshot \
    DiscardStale Crash Restart
run_model KnytosRaftStream KnytosRaftStream_small.cfg \
    BeginReceive RestartOffsetZero ChangeTerm RecognizeLeader ReceiveChunk \
    CancelReceive RejectStale RejectLeader RejectOffset RejectBoundary
run_expected_violation KnytosRaftStream KnytosRaftStream_owner_unsafe.cfg \
    InstalledSingleOwner ReceiveChunk
run_expected_violation KnytosRaftStream KnytosRaftStream_offset_unsafe.cfg \
    InstalledInOrder ReceiveChunk
run_model KnytosSnapshotRestore KnytosSnapshotRestore_small.cfg \
    ReadHeader ParseSections RejectUnsupported RejectMalformed Validate Restore \
    Persist DefinitePersistenceFailure AmbiguousPersistenceFailure \
    ActivateStreamJoin ActivateStreamRetained \
    ActivateRecoveryJoin ActivateRecoveryRetained Crash BeginRecovery
run_expected_violation KnytosSnapshotRestore \
    KnytosSnapshotRestore_application_unsafe.cfg \
    RejectionPreservesApplication UnsafeRejectApplication
run_expected_violation KnytosSnapshotRestore \
    KnytosSnapshotRestore_durable_unsafe.cfg \
    RejectionPreservesDurable UnsafeRejectDurable
run_expected_violation KnytosSnapshotRestore \
    KnytosSnapshotRestore_progress_unsafe.cfg \
    RejectionPreservesProgress UnsafeRejectProgress
run_expected_violation KnytosSnapshotRestore \
    KnytosSnapshotRestore_membership_unsafe.cfg \
    RejectionPreservesMembership UnsafeRejectMembership
run_expected_violation KnytosSnapshotRestore \
    KnytosSnapshotRestore_join_unsafe.cfg \
    RejectionPreservesJoin UnsafeRejectJoin
run_expected_violation KnytosSnapshotRestore \
    KnytosSnapshotRestore_exposure_unsafe.cfg \
    RecoveryExposureRequiresRestore UnsafeExposeRecovery
run_expected_violation KnytosSnapshotRestore \
    KnytosSnapshotRestore_ordering_unsafe.cfg \
    PublicationRequiresRestore UnsafePublishBeforeRestore
run_model KnytosClusterIngress KnytosClusterIngress_small.cfg \
    PublishReady WithdrawReady ReplaceDeployment CommitDrain BeginJoint \
    FinalizeJoint LearnRoute ForgetRoute InstallSnapshot StartNewFlow \
    ExpireLease ExistingFlowPacket DropStaleNewFlow DropUnretainedFlow \
    DropWithdrawnFlow DropDepartedBackend EndFlow
run_expected_violation KnytosClusterIngress \
    KnytosClusterIngress_stale_snapshot_unsafe.cfg \
    NewFlowsRequireFreshLease UnsafeStartStaleFlow
run_expected_violation KnytosClusterIngress \
    KnytosClusterIngress_history_unsafe.cfg \
    FlowBackendStable UnsafeFallbackExistingPacket
run_expected_violation KnytosClusterIngress \
    KnytosClusterIngress_readiness_unsafe.cfg \
    CommittedPoliciesAreDerived UnsafeAdmitStaleReadiness
run_model KnytosPlacementControl KnytosPlacementControl_small.cfg \
    Observe SelectCandidate Tick PressureMove RemoveEligibility ForcedMove Admit
run_expected_violation KnytosPlacementControl \
    KnytosPlacementControl_dwell_unsafe.cfg \
    PressureRespectsStability UnsafeEarlyPressureMove
run_expected_violation KnytosPlacementControl \
    KnytosPlacementControl_reserve_unsafe.cfg \
    ReservationsPreserveReserve UnsafeAdmit
run_model KnytosRemoteCall KnytosRemoteCall_small.cfg \
    Start ReplaceTarget Execute RejectStale QueueReply DuplicateRequest \
    DeliverReply Timeout SettleTransport RetireUncertainSession Evict
run_model KnytosReliableMessage KnytosReliableMessage_small.cfg \
    AbandonSession RestartService AcceptCurrentSession
run_expected_violation KnytosReliableMessage \
    KnytosReliableMessage_identity_unsafe.cfg \
    SessionIdentityUnique RestartService
run_expected_violation KnytosReliableMessage \
    KnytosReliableMessage_regression_unsafe.cfg \
    ReceiveSessionMonotonic AcceptDelayedSession

if [[ "${selected_cases}" -eq 0 ]]; then
    echo "error: no configurations registered for ${selected_module}" >&2
    exit 1
fi
