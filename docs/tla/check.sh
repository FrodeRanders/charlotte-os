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
run_model CharlotteIpcTransaction CharlotteIpcTransaction_small.cfg \
    BeginVector TransferMove BeginBorrow AttachConnection Commit \
    FailAndRollback Deliver WaitTimeout Reply ObserveReply
run_expected_violation CharlotteIpcTransaction \
    CharlotteIpcTransaction_rollback_unsafe.cfg \
    RollbackRestoresAll UnsafeRollback
run_expected_violation CharlotteIpcTransaction \
    CharlotteIpcTransaction_timeout_unsafe.cfg \
    TimeoutRetainsLoan UnsafeTimeoutRelease
run_model CharlotteIpcPublication CharlotteIpcPublication_small.cfg \
    Begin PrepareRecord RejectStorage Reserve RejectReservation PrepareMemory \
    PrepareConnection Commit RejectCommit FinishPreparation AbortPreparation \
    Readiness PrepareReceive RejectReceive ReceiveWriteFailed CommitReceive \
    ObserveReply RepeatObservation Use ApplicationClose CancelHidden Fill \
    RetireNamespace DetachNamespace ReplaceNamespace ReleaseRetired Abandon
run_expected_violation CharlotteIpcPublication CharlotteIpcPublication_memory_unsafe.cfg \
    UseRequiresDelivery UnsafeUseMemory
run_expected_violation CharlotteIpcPublication CharlotteIpcPublication_connection_unsafe.cfg \
    UseRequiresDelivery UnsafeUseConnection
run_expected_violation CharlotteIpcPublication CharlotteIpcPublication_batch_unsafe.cfg \
    AtomicPublication UnsafePublishPrefix
run_expected_violation CharlotteIpcPublication CharlotteIpcPublication_serial_unsafe.cfg \
    SerialsConsumed UnsafeReuseSerial
run_expected_violation CharlotteIpcPublication CharlotteIpcPublication_cancel_unsafe.cfg \
    ReplacementIntact UnsafeCancelSuccessor
run_expected_violation CharlotteIpcPublication CharlotteIpcPublication_restore_unsafe.cfg \
    ReplacementIntact UnsafeRestoreSuccessor
run_expected_violation CharlotteIpcPublication CharlotteIpcPublication_refund_unsafe.cfg \
    RetainedNodesCharged UnsafeRefundRetained
run_expected_violation CharlotteIpcPublication CharlotteIpcPublication_readiness_unsafe.cfg \
    VisibleRequiresDelivery UnsafeReadinessDelivers
run_expected_violation CharlotteIpcPublication CharlotteIpcPublication_observe_unsafe.cfg \
    ObservedCloseStaysClosed UnsafeRepublishObserved
run_expected_violation CharlotteIpcPublication CharlotteIpcPublication_rollback_unsafe.cfg \
    RollbackRestoresCapturedSource UnsafeRollbackNeedsAdmission
run_model CharlotteEndpointObservers CharlotteEndpointObservers_small.cfg \
    ArmReadiness ArmCloseWatch Send Receive Close ObserveClose
run_expected_violation CharlotteEndpointObservers CharlotteEndpointObservers_unsafe.cfg \
    CloseSignalImpliesClosed UnsafeMessageWake
run_model CharlotteNamePublication CharlotteNamePublication_small.cfg \
    Begin RejectBeforeWatch PrepareWatch RejectPolicy PublishPolicy Install \
    Lookup QueueLookup ResolveDeferred FinishInstall CloseEndpoint CqHint \
    ObservePending ObserveUnrelated RetireDeath RetireExplicit RejectRetirement \
    CancelWatch CloseSource WatchError WaitRejected LifecycleAbort PublicationAbort
run_model CharlotteNamePublication CharlotteNamePublication_exhaustion.cfg \
    Begin PrepareWatch PublishPolicy Install FinishInstall ExhaustionRetire \
    CancelWatch CloseSource RejectExhausted
run_expected_violation CharlotteNamePublication CharlotteNamePublication_public_unsafe.cfg \
    PublicIssuance UnsafePublicLookup
run_expected_violation CharlotteNamePublication CharlotteNamePublication_deferred_unsafe.cfg \
    PublicIssuance UnsafeDeferredLookup
run_expected_violation CharlotteNamePublication CharlotteNamePublication_replacement_unsafe.cfg \
    ReplacementProtected UnsafePublicReplacement
run_expected_violation CharlotteNamePublication CharlotteNamePublication_watch_unsafe.cfg \
    BeforeWatchUnchanged UnsafeMutationBeforeWatch
run_expected_violation CharlotteNamePublication CharlotteNamePublication_hint_unsafe.cfg \
    DeathNeedsExactResult UnsafeHintRetires
run_expected_violation CharlotteNamePublication CharlotteNamePublication_stale_unsafe.cfg \
    DeathNeedsExactResult UnsafeStaleHint
run_expected_violation CharlotteNamePublication CharlotteNamePublication_disposal_unsafe.cfg \
    WatchRetainsSource UnsafeCloseSourceFirst
run_expected_violation CharlotteNamePublication CharlotteNamePublication_quota_unsafe.cfg \
    LiveBound UnsafeNewAtCapacity
run_expected_violation CharlotteNamePublication CharlotteNamePublication_abort_unsafe.cfg \
    AbortRetainsOwners UnsafeAbortDisposes
run_expected_violation CharlotteNamePublication CharlotteNamePublication_retire_unsafe.cfg \
    RetirementAuthorized UnsafeUnauthorizedRetire
run_expected_violation CharlotteNamePublication CharlotteNamePublication_wrap_unsafe.cfg \
    GenerationNotReused UnsafeGenerationWrap
run_expected_violation CharlotteNamePublication CharlotteNamePublication_exhausted_unsafe.cfg \
    PolicyGenerationRepresentable UnsafePublishExhausted
run_expected_violation CharlotteNamePublication CharlotteNamePublication_serial_unsafe.cfg \
    GrantGenerationCoherent UnsafeLookupDuringPublish
run_model CharlotteBootTrust CharlotteBootTrust_fixture.cfg \
    ReadInstalled ReadFailure Verify RejectRecord AcquireRecipient PrepareSigned \
    RejectRecipient PrepareDevelopment Publish RejectPublication BorrowView DropPreparation Reboot
run_model CharlotteBootTrust CharlotteBootTrust_contract.cfg \
    ReadInstalled ReadEnrollment ReadFailure Verify RejectRecord CommitPolicy \
    CommitFailure CommitInterrupted CompetingCommit AcquireRecipient PrepareSigned \
    RejectRecipient Publish RejectPublication BorrowView DropPreparation Reboot
run_expected_violation CharlotteBootTrust CharlotteBootTrust_uninstalled_unsafe.cfg \
    InstalledBeforePublication UnsafeUninstalledPublish
run_expected_violation CharlotteBootTrust CharlotteBootTrust_stale_unsafe.cfg \
    NoForkedInstallation UnsafeStaleCommit
run_expected_violation CharlotteBootTrust CharlotteBootTrust_enrollment_unsafe.cfg \
    NoEnrollmentFallback UnsafeEnrollmentFallback
run_expected_violation CharlotteBootTrust CharlotteBootTrust_overwrite_unsafe.cfg \
    OnePublicationPerBoot UnsafeOverwrite
run_expected_violation CharlotteBootTrust CharlotteBootTrust_manifest_unsafe.cfg \
    SharedPolicy UnsafeIndependentManifest
run_expected_violation CharlotteBootTrust CharlotteBootTrust_recipient_unsafe.cfg \
    RecipientBound UnsafeWrongRecipient
run_expected_violation CharlotteBootTrust CharlotteBootTrust_authenticate_unsafe.cfg \
    VerifiedAuthenticated UnsafeAuthenticate
run_expected_violation CharlotteBootTrust CharlotteBootTrust_development_unsafe.cfg \
    ExplicitDevelopment UnsafeDevelopmentFallback
run_expected_violation CharlotteBootTrust CharlotteBootTrust_failure_unsafe.cfg \
    InstalledBeforePublication UnsafeFailedCommitPublishes
run_expected_violation CharlotteBootTrust CharlotteBootTrust_boundary_unsafe.cfg \
    ConditionalBoundary UnsafeUnsupportedProduction
run_model CharlotteCQ CharlotteCQ_mini.cfg \
    Complete Fail CancelOp DrainOne DrainAll ObserveResult CqWait CqWake TimerFire
run_expected_violation CharlotteCQ CharlotteCQ_buffer_unsafe.cfg \
    NonTerminalBufferRemainsLoaned UnsafeCancelReleasesBuffer
run_model CharlotteTimedWait CharlotteTimedWait_small.cfg \
    ArmWait PublishWork DeliverWake TimerFire Consume
run_expected_violation CharlotteTimedWait CharlotteTimedWait_unsafe.cfg \
    TimeoutObservedNoWork UnsafeTimerFire
run_model CharlotteCheckedWait CharlotteCheckedWait_small.cfg \
    BeginWait PrepareWatchdog RejectWatchdog PrepareSource RejectSource \
    Register PublishPark PublishTerminal ChangeEpisode Wake UnrelatedWake \
    Recheck ObserveTerminal AdvanceTime Abandon
run_expected_violation CharlotteCheckedWait CharlotteCheckedWait_rejection_unsafe.cfg \
    PendingRetainsOwnerAndLoan UnsafeRejectedRelease
run_expected_violation CharlotteCheckedWait CharlotteCheckedWait_replay_unsafe.cfg \
    OneSubmission UnsafeReplay
run_expected_violation CharlotteCheckedWait CharlotteCheckedWait_deadline_unsafe.cfg \
    DeadlineUnchanged UnsafeRestartDeadline
run_expected_violation CharlotteCheckedWait CharlotteCheckedWait_result_unsafe.cfg \
    ResultNeedsTerminalProof UnsafeWakeAdoptsResult
run_expected_violation CharlotteCheckedWait CharlotteCheckedWait_park_unsafe.cfg \
    ParkedTerminalHasWake UnsafeParkWithoutRecheck
run_expected_violation CharlotteCheckedWait CharlotteCheckedWait_timeout_unsafe.cfg \
    TimeoutRespectsDeadline UnsafeAdmissionTimeout
run_model CharlotteScheduler CharlotteScheduler_small.cfg \
    Spawn Admit Dispatch Preempt Block Wake SwitchOff Migrate \
    RequestRemoteAbort RetireRemoteAbort AbortNotRunning SelfAbort Reap \
    BeginDomainAbort DestroyAddressSpace
run_expected_violation CharlotteScheduler CharlotteScheduler_unsafe.cfg \
    ReapOnlyOffCpu UnsafeRemoteAbort
run_expected_violation CharlotteScheduler CharlotteScheduler_domain_abort_unsafe.cfg \
    AbortingThreadsDoomed UnsafeSpawnDuringAbort
run_model CharlotteStackGrowth CharlotteStackGrowth_small.cfg \
    Fault Grow Kill Exit
run_expected_violation CharlotteStackGrowth \
    CharlotteStackGrowth_grow_unsafe.cfg \
    CommittedWithinBudget UnsafeGrowOverBudget
run_expected_violation CharlotteStackGrowth \
    CharlotteStackGrowth_leak_unsafe.cfg \
    DeadDomainsReleaseFrames UnsafeKillWithoutRelease
run_model CharlotteThreadJoin CharlotteThreadJoin_small.cfg \
    Spawn RejectExhaustedSpawn CaptureHandle Exit ObserveJoin Reap
run_expected_violation CharlotteThreadJoin CharlotteThreadJoin_unsafe.cfg \
    ObserverMatchesCapturedHandle UnsafeObserveJoin
run_expected_violation CharlotteThreadJoin CharlotteThreadJoin_wrap_unsafe.cfg \
    GenerationNeverReused UnsafeSpawnWrap
run_model CharlotteAddressSpace CharlotteAddressSpace_small.cfg \
    Allocate CaptureHandle CloseExact
run_expected_violation CharlotteAddressSpace CharlotteAddressSpace_unsafe.cfg \
    ReplacementSurvivesStaleHandle UnsafeStaleClose
run_model CharlotteRootRetirement CharlotteRootRetirement_small.cfg \
    Allocate Acquire FinishOperation AbandonOperation BeginClose ClosePending \
    FinishCleanup Detach BeginInvalidation InvalidationFailed RetainForRetry \
    ClaimRetry InvalidationSucceeded StartPhysical FinishPhysical Abandon
run_expected_violation CharlotteRootRetirement CharlotteRootRetirement_lease_unsafe.cfg \
    LeasesRetainRoot UnsafeDetachWithLease
run_expected_violation CharlotteRootRetirement CharlotteRootRetirement_release_unsafe.cfg \
    PhysicalRequiresInvalidation UnsafeReleaseBeforeInvalidation
run_expected_violation CharlotteRootRetirement CharlotteRootRetirement_physical_unsafe.cfg \
    PhysicalOnce UnsafeRetryPhysical
run_expected_violation CharlotteRootRetirement CharlotteRootRetirement_abandon_unsafe.cfg \
    TerminalAbandonment UnsafeAdoptAbandoned
run_expected_violation CharlotteRootRetirement CharlotteRootRetirement_refund_unsafe.cfg \
    BackingCharged UnsafeRefundPartial
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
run_model CharlotteDmaOperation CharlotteDmaOperation_small.cfg \
    BeginMap ClaimMap PrepareInterest RejectSourcePreparation RejectContext \
    LinkInterest ReadyDuringPreparation RejectWaiter Park ObserveClosedSource \
    RestoreUnit NotifySource CompetingClaim AbandonExternal CleanupUnstarted \
    FinishMap CompleteMapPublic RejectMapPublicCompletion BeginUnmap \
    RejectUnmapAdmission ClaimUnmap FinishUnmap CompleteUnmapPublic \
    RejectUnmapPublicCompletion CpuUnmapFailure CpuUnmapSuccess \
    FailedSharedCpuUnmap FreshCpuMap PrepareLocalDrop DropLocal ReleaseBacking RejectRootClose Abandon
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_cell_unsafe.cfg \
    ClaimNeedsCompleteUnit UnsafeClaimDetachedCell
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_readiness_unsafe.cfg \
    ReadinessIsNotCompletion UnsafeReadinessCompletes
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_preparation_unsafe.cfg \
    PreclaimRetainsPreparation UnsafeRejectDropsPreparation
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_map_unsafe.cfg \
    MapSubmittedOnce UnsafeReplayMap
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_rearm_unsafe.cfg \
    StartedFailureNotRearmed UnsafeRearmStartedUnmap
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_drop_unsafe.cfg \
    UncertainAttemptNotResubmitted UnsafeDropResubmits
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_confirmation_unsafe.cfg \
    ConfirmationSticky UnsafeForgetDmaConfirmation
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_access_unsafe.cfg \
    FailedSharedRestricted UnsafeFailedSharedAccess
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_cpu_unsafe.cfg \
    PossibleDmaRetainsBacking UnsafeCpuUnmapDischargesDma
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_abandon_unsafe.cfg \
    AbandonmentRetainsOwner UnsafeAbandonReleases
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_context_unsafe.cfg \
    WaitNeedsQualifiedContext UnsafeWaitMasked
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_public_unsafe.cfg \
    PublicRejectionRetainsAttempt UnsafeFinalBusyRearms
run_expected_violation CharlotteDmaOperation CharlotteDmaOperation_remap_unsafe.cfg \
    CpuOwnerNeedsNoDma UnsafeCoherentRemap
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
run_model CharlotteSnapshotRestore CharlotteSnapshotRestore_small.cfg \
    ReadHeader ParseSections RejectUnsupported RejectMalformed Validate Restore \
    Persist DefinitePersistenceFailure AmbiguousPersistenceFailure \
    ActivateStream ActivateRecovery Crash BeginRecovery
run_expected_violation CharlotteSnapshotRestore \
    CharlotteSnapshotRestore_application_unsafe.cfg \
    RejectionPreservesApplication UnsafeRejectApplication
run_expected_violation CharlotteSnapshotRestore \
    CharlotteSnapshotRestore_durable_unsafe.cfg \
    RejectionPreservesDurable UnsafeRejectDurable
run_expected_violation CharlotteSnapshotRestore \
    CharlotteSnapshotRestore_progress_unsafe.cfg \
    RejectionPreservesProgress UnsafeRejectProgress
run_expected_violation CharlotteSnapshotRestore \
    CharlotteSnapshotRestore_membership_unsafe.cfg \
    RejectionPreservesMembership UnsafeRejectMembership
run_expected_violation CharlotteSnapshotRestore \
    CharlotteSnapshotRestore_join_unsafe.cfg \
    RejectionPreservesJoin UnsafeRejectJoin
run_expected_violation CharlotteSnapshotRestore \
    CharlotteSnapshotRestore_exposure_unsafe.cfg \
    RecoveryExposureRequiresRestore UnsafeExposeRecovery
run_expected_violation CharlotteSnapshotRestore \
    CharlotteSnapshotRestore_ordering_unsafe.cfg \
    PublicationRequiresRestore UnsafePublishBeforeRestore
run_model CharlotteClusterIngress CharlotteClusterIngress_small.cfg \
    PublishReady WithdrawReady ReplaceDeployment CommitDrain BeginJoint \
    FinalizeJoint LearnRoute ForgetRoute InstallSnapshot StartNewFlow \
    ExpireLease ExistingFlowPacket DropStaleNewFlow DropUnretainedFlow \
    DropWithdrawnFlow DropDepartedBackend EndFlow
run_expected_violation CharlotteClusterIngress \
    CharlotteClusterIngress_stale_snapshot_unsafe.cfg \
    NewFlowsRequireFreshLease UnsafeStartStaleFlow
run_expected_violation CharlotteClusterIngress \
    CharlotteClusterIngress_history_unsafe.cfg \
    FlowBackendStable UnsafeFallbackExistingPacket
run_expected_violation CharlotteClusterIngress \
    CharlotteClusterIngress_readiness_unsafe.cfg \
    CommittedPoliciesAreDerived UnsafeAdmitStaleReadiness
run_model CharlottePlacementControl CharlottePlacementControl_small.cfg \
    Observe SelectCandidate Tick PressureMove RemoveEligibility ForcedMove Admit
run_expected_violation CharlottePlacementControl \
    CharlottePlacementControl_dwell_unsafe.cfg \
    PressureRespectsStability UnsafeEarlyPressureMove
run_expected_violation CharlottePlacementControl \
    CharlottePlacementControl_reserve_unsafe.cfg \
    ReservationsPreserveReserve UnsafeAdmit
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
