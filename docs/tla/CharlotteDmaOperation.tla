------------------------- MODULE CharlotteDmaOperation -------------------------
\* One map followed by owned unmap/CPU teardown. Availability, backend
\* acknowledgement and public completion are distinct proof boundaries.

EXTENDS Naturals

CONSTANTS MaxEpoch, MaxUnmapAttempts
ASSUME MaxEpoch >= 3
ASSUME MaxUnmapAttempts >= 2

Contexts == {"Scheduled", "Masked", "Nonscheduled", "Rejecting"}
Modes == {"Exclusive", "Shared"}
Phases == {"Idle", "MapAdmission", "SourcePrepared", "Watching", "Waiting",
    "MapRejecting", "MapRunning", "MapReturned", "Mapped", "MapFailed", "MapFailedRetained",
    "UnmapAdmission", "UnmapRunning", "UnmapReturned", "UnmapFailed",
    "UnmapFailedRetained", "DmaDone", "CpuUnmapFailed", "CpuOwned",
    "DropPending", "Dropped", "Abandoned"}
UnitOwners == {"None", "External", "ExternalAbandoned", "Map", "Unmap"}
PinOwners == {"None", "Preparation", "Maintenance", "Live", "Quarantine"}
UnmapResults == {"None", "AdmissionBusy", "Complete", "CompletionBusy", "CompletionOther"}

VARIABLE s
vars == <<s>>

Init == \E layout \in {"Ready", "Busy", "DetachedCell"} :
    s = [phase |-> "Idle", mode |-> "Exclusive", context |-> "Scheduled",
        unit |-> IF layout = "Ready" THEN "None" ELSE "External",
        engine |-> (layout # "Busy"), cell |-> (layout = "Ready"), epoch |-> 1,
        sourceStorage |-> FALSE, interest |-> FALSE, registered |-> FALSE,
        interestEpoch |-> 0, notice |-> FALSE, sourceClosed |-> FALSE,
        root |-> FALSE, claim |-> FALSE, rootGeneration |-> 1,
        capturedRoot |-> 1, pinRoot |-> 1, sponsor |-> 1,
        pin |-> "None", backing |-> TRUE, charge |-> TRUE,
        cpuMapped |-> FALSE, cpuAccess |-> "Owned", closeRequested |-> FALSE,
        mapSubmissions |-> 0, mapWork |-> 0, mapClaimProof |-> FALSE,
        mapResult |-> "None", mapUncertain |-> FALSE, failedMapReturned |-> FALSE,
        unmapState |-> "Active", unmapSubmissions |-> 0, unmapWork |-> 0,
        unmapResult |-> "None", uncertainAt |-> 0, confirmedAt |-> 0,
        backendAck |-> FALSE, dmaConfirmed |-> FALSE, hardwarePossible |-> FALSE,
        startedFailure |-> FALSE, dropRequested |-> FALSE,
        cpuFailure |-> FALSE, rootCloseRejected |-> FALSE,
        abandoned |-> FALSE, abandonedPin |-> "None", abandonedUnit |-> "None",
        abandonedStorage |-> FALSE]

Available == s.unit = "None" /\ s.engine /\ s.cell
Qualified == s.context = "Scheduled"
Preclaim == s.phase \in {"MapAdmission", "SourcePrepared", "Watching", "Waiting", "MapRejecting"}

BeginMap(mode, context) ==
    /\ s.phase = "Idle" /\ mode \in Modes /\ context \in Contexts
    /\ s' = [s EXCEPT !.phase = "MapAdmission", !.mode = mode, !.context = context,
        !.root = TRUE, !.claim = TRUE, !.pin = "Preparation", !.mapSubmissions = 1,
        !.cpuMapped = (mode = "Shared"), !.cpuAccess = "Restricted"]

ClaimMap ==
    /\ s.phase = "MapAdmission" /\ Available
    /\ s' = [s EXCEPT !.phase = "MapRunning", !.unit = "Map",
        !.engine = FALSE, !.cell = FALSE, !.pin = "Maintenance",
        !.mapWork = @ + 1, !.mapClaimProof = TRUE,
        !.hardwarePossible = TRUE,
        !.sourceStorage = FALSE, !.interest = FALSE, !.registered = FALSE]

PrepareInterest ==
    /\ s.phase = "MapAdmission" /\ ~Available /\ Qualified
    /\ s' = [s EXCEPT !.phase = "SourcePrepared", !.sourceStorage = TRUE]

RejectSourcePreparation ==
    /\ s.phase = "MapAdmission" /\ ~Available /\ Qualified
    /\ s' = [s EXCEPT !.phase = "MapRejecting", !.mapResult = "Error"]

RejectContext ==
    /\ s.phase = "MapAdmission" /\ ~Available /\ ~Qualified
    /\ s' = [s EXCEPT !.phase = "MapRejecting", !.mapResult = "Error"]

LinkInterest ==
    /\ s.phase = "SourcePrepared" /\ ~Available
    /\ s' = [s EXCEPT !.phase = "Watching", !.interest = TRUE,
        !.interestEpoch = s.epoch, !.sourceClosed = FALSE]

ReadyDuringPreparation ==
    /\ s.phase = "SourcePrepared" /\ Available
    /\ s' = [s EXCEPT !.phase = "MapAdmission", !.sourceStorage = FALSE]

RejectWaiter ==
    /\ s.phase = "Watching"
    /\ s' = [s EXCEPT !.phase = "MapRejecting", !.mapResult = "Error"]

Park ==
    /\ s.phase = "Watching" /\ Qualified /\ ~s.sourceClosed
    /\ s' = [s EXCEPT !.phase = "Waiting", !.registered = TRUE]

ObserveClosedSource ==
    /\ s.phase \in {"Watching", "Waiting"} /\ s.sourceClosed
    /\ s' = [s EXCEPT !.phase = "MapAdmission", !.registered = FALSE,
        !.interest = FALSE, !.sourceStorage = FALSE]

\* An external holder restores the actual engine and domain cell together.
\* Deferred notification is separate, so another claim may win before wake.
RestoreUnit ==
    /\ s.unit = "External" /\ s.epoch < MaxEpoch
    /\ s' = [s EXCEPT !.unit = "None", !.engine = TRUE, !.cell = TRUE,
        !.epoch = @ + 1, !.notice = (s.notice \/ s.interest)]

NotifySource ==
    /\ s.notice
    /\ s' = [s EXCEPT !.notice = FALSE,
        !.sourceClosed = (s.sourceClosed \/ (s.interest /\ s.interestEpoch < s.epoch))]

CompetingClaim ==
    /\ Available /\ s.epoch < MaxEpoch
    /\ s' = [s EXCEPT !.unit = "External", !.engine = FALSE, !.cell = FALSE]

AbandonExternal ==
    /\ s.unit = "External"
    /\ s' = [s EXCEPT !.unit = "ExternalAbandoned"]

CleanupUnstarted ==
    /\ s.phase = "MapRejecting" /\ s.pin = "Preparation"
    /\ s' = [s EXCEPT !.phase = "MapReturned", !.pin = "None",
        !.sourceStorage = FALSE, !.interest = FALSE, !.registered = FALSE]

FinishMap(result) ==
    /\ s.phase = "MapRunning" /\ s.unit = "Map"
    /\ result \in {"Success", "Rollback", "Uncertain"}
    /\ s' = [s EXCEPT !.phase = "MapReturned", !.unit = "None", !.engine = TRUE, !.cell = TRUE,
        !.pin = CASE result = "Success" -> "Live"
                    [] result = "Rollback" -> "None" [] OTHER -> "Quarantine",
        !.hardwarePossible = (result # "Rollback"),
        !.mapUncertain = (result = "Uncertain"),
        !.mapResult = IF result = "Success" THEN "Complete" ELSE "Error"]

CompleteMapPublic ==
    /\ s.phase = "MapReturned" /\ s.unit # "Map" /\ s.mapResult # "None"
    /\ s.pin # "Preparation"
    /\ s' = [s EXCEPT !.root = FALSE, !.claim = FALSE,
        !.phase = IF s.mapResult = "Complete" THEN "Mapped" ELSE "MapFailed",
        !.failedMapReturned = (s.mapResult = "Error"),
        !.cpuAccess = IF s.mapResult = "Complete" /\ s.mode = "Shared"
                        THEN "Volatile" ELSE "Restricted"]

RejectMapPublicCompletion ==
    /\ s.phase = "MapReturned" /\ s.unit # "Map" /\ s.mapResult # "None"
    /\ s' = [s EXCEPT !.phase = "MapFailedRetained", !.failedMapReturned = TRUE,
        !.cpuAccess = "Restricted"]

\* Store Retained before kernel entry. Only typed pre-claim busy can rearm it.
BeginUnmap(fromDrop) ==
    /\ s.phase = "Mapped" /\ s.unmapState = "Active"
    /\ s.unmapSubmissions < MaxUnmapAttempts /\ fromDrop \in BOOLEAN
    /\ s' = [s EXCEPT !.phase = "UnmapAdmission", !.unmapState = "Retained",
        !.unmapSubmissions = @ + 1, !.uncertainAt = s.unmapSubmissions + 1,
        !.unmapResult = "None", !.dropRequested = fromDrop,
        !.root = TRUE, !.claim = TRUE]

RejectUnmapAdmission ==
    /\ s.phase = "UnmapAdmission" /\ ~Available
    /\ s' = [s EXCEPT !.phase = "UnmapReturned", !.unmapResult = "AdmissionBusy"]

ClaimUnmap ==
    /\ s.phase = "UnmapAdmission" /\ Available
    /\ s' = [s EXCEPT !.phase = "UnmapRunning", !.unit = "Unmap",
        !.engine = FALSE, !.cell = FALSE, !.pin = "Maintenance", !.unmapWork = @ + 1]

FinishUnmap(result) ==
    /\ s.phase = "UnmapRunning" /\ s.unit = "Unmap"
    /\ result \in {"Complete", "CompletionBusy", "CompletionOther"}
    /\ s' = [s EXCEPT !.phase = "UnmapReturned", !.unmapResult = result,
        !.unit = "None", !.engine = TRUE, !.cell = TRUE,
        !.pin = IF result = "Complete" THEN "None" ELSE "Quarantine",
        !.backendAck = (result = "Complete"), !.hardwarePossible = (result # "Complete"),
        !.startedFailure = (result # "Complete")]

CompleteUnmapPublic ==
    /\ s.phase = "UnmapReturned"
    /\ s' = [s EXCEPT !.root = FALSE, !.claim = FALSE,
        !.unmapState = CASE s.unmapResult = "AdmissionBusy" -> "Active"
                          [] s.unmapResult = "Complete" -> "Complete" [] OTHER -> "Retained",
        !.uncertainAt = IF s.unmapResult = "AdmissionBusy" THEN 0 ELSE @,
        !.dmaConfirmed = (s.unmapResult = "Complete"),
        !.confirmedAt = IF s.unmapResult = "Complete" THEN s.unmapSubmissions ELSE @,
        !.phase = CASE s.dropRequested -> "DropPending"
                     [] s.unmapResult = "AdmissionBusy" -> "Mapped"
                     [] s.unmapResult = "Complete" -> "DmaDone" [] OTHER -> "UnmapFailed"]

\* Even a busy-looking final public completion failure is after backend entry.
RejectUnmapPublicCompletion ==
    /\ s.phase = "UnmapReturned"
    /\ s' = [s EXCEPT !.phase = "UnmapFailedRetained", !.unmapState = "Retained"]

CpuUnmapFailure ==
    /\ s.phase = "DmaDone" /\ s.mode = "Shared"
    /\ s' = [s EXCEPT !.phase = "CpuUnmapFailed", !.cpuFailure = TRUE]

CpuUnmapSuccess ==
    /\ s.phase \in {"DmaDone", "CpuUnmapFailed"}
    /\ s' = [s EXCEPT !.phase = "CpuOwned", !.cpuMapped = FALSE, !.cpuAccess = "Owned"]

FailedSharedCpuUnmap ==
    /\ s.phase \in {"MapFailed", "MapFailedRetained"} /\ s.mode = "Shared" /\ s.cpuMapped
    /\ s' = [s EXCEPT !.cpuMapped = FALSE]

\* Consuming the restricted wrapper can return an unmapped OwnedMemory. A
\* fresh CPU mapping still needs the kernel's all-DMA-pin admission check.
FreshCpuMap ==
    /\ s.phase \in {"MapFailed", "MapFailedRetained"} /\ s.mode = "Shared"
    /\ ~s.cpuMapped /\ s.pin = "None" /\ ~s.hardwarePossible
    /\ s' = [s EXCEPT !.phase = "CpuOwned", !.cpuMapped = TRUE, !.cpuAccess = "Rust"]

PrepareLocalDrop ==
    /\ s.phase \in {"MapFailed", "MapFailedRetained", "UnmapFailed",
                     "UnmapFailedRetained", "CpuUnmapFailed", "CpuOwned", "DmaDone"}
    /\ s' = [s EXCEPT !.phase = "DropPending"]

\* Best-effort CPU teardown may fail; uncertain kernel DMA pins remain owned.
DropLocal(cpuUnmapped) ==
    /\ s.phase = "DropPending" /\ cpuUnmapped \in BOOLEAN
    /\ s' = [s EXCEPT !.phase = "Dropped", !.closeRequested = TRUE,
        !.cpuAccess = "None", !.cpuMapped = (s.cpuMapped /\ ~cpuUnmapped)]

ReleaseBacking ==
    /\ s.closeRequested /\ s.backing /\ ~s.cpuMapped
    /\ s.pin = "None" /\ ~s.hardwarePossible
    /\ s' = [s EXCEPT !.backing = FALSE, !.charge = FALSE]

RejectRootClose ==
    /\ s.root /\ ~s.rootCloseRejected
    /\ s' = [s EXCEPT !.rootCloseRejected = TRUE]

Abandon ==
    /\ s.root /\ (Preclaim \/ s.phase \in {"MapRunning", "MapReturned", "UnmapAdmission", "UnmapRunning", "UnmapReturned"})
    /\ s' = [s EXCEPT !.phase = "Abandoned", !.abandoned = TRUE,
        !.abandonedPin = s.pin, !.abandonedUnit = s.unit,
        !.abandonedStorage = s.sourceStorage]

Next ==
    \/ \E mode \in Modes, context \in Contexts : BeginMap(mode, context)
    \/ ClaimMap \/ PrepareInterest \/ RejectSourcePreparation \/ RejectContext
    \/ LinkInterest \/ ReadyDuringPreparation \/ RejectWaiter \/ Park
    \/ ObserveClosedSource \/ RestoreUnit \/ NotifySource \/ CompetingClaim
    \/ AbandonExternal \/ CleanupUnstarted \/ CompleteMapPublic \/ RejectMapPublicCompletion
    \/ \E result \in {"Success", "Rollback", "Uncertain"} : FinishMap(result)
    \/ \E fromDrop \in BOOLEAN : BeginUnmap(fromDrop)
    \/ RejectUnmapAdmission \/ ClaimUnmap
    \/ \E result \in {"Complete", "CompletionBusy", "CompletionOther"} : FinishUnmap(result)
    \/ CompleteUnmapPublic \/ RejectUnmapPublicCompletion \/ CpuUnmapFailure
    \/ CpuUnmapSuccess \/ FailedSharedCpuUnmap \/ FreshCpuMap \/ PrepareLocalDrop
    \/ \E cpuUnmapped \in BOOLEAN : DropLocal(cpuUnmapped)
    \/ ReleaseBacking \/ RejectRootClose \/ Abandon

Spec == Init /\ [][Next]_vars

UnsafeClaimDetachedCell ==
    /\ s.phase = "MapAdmission" /\ s.engine /\ ~s.cell
    /\ s' = [s EXCEPT !.mapWork = 1, !.mapClaimProof = FALSE]

UnsafeReadinessCompletes ==
    /\ s.phase \in {"Watching", "Waiting"} /\ s.sourceClosed
    /\ s' = [s EXCEPT !.dmaConfirmed = TRUE]

UnsafeRejectDropsPreparation ==
    /\ s.phase = "MapRejecting"
    /\ s' = [s EXCEPT !.root = FALSE, !.claim = FALSE, !.pin = "None"]

UnsafeReplayMap ==
    /\ s.failedMapReturned
    /\ s' = [s EXCEPT !.mapSubmissions = 2]

UnsafeRearmStartedUnmap ==
    /\ s.phase = "UnmapFailed" /\ s.startedFailure
    /\ s' = [s EXCEPT !.phase = "Mapped", !.unmapState = "Active"]

UnsafeDropResubmits ==
    /\ s.phase \in {"UnmapFailed", "UnmapFailedRetained"} /\ s.unmapState = "Retained"
    /\ s' = [s EXCEPT !.unmapSubmissions = @ + 1]

UnsafeForgetDmaConfirmation ==
    /\ s.phase = "CpuUnmapFailed" /\ s.dmaConfirmed
    /\ s' = [s EXCEPT !.unmapState = "Active"]

UnsafeFailedSharedAccess ==
    /\ s.failedMapReturned /\ s.mode = "Shared" /\ s.cpuMapped
    /\ s' = [s EXCEPT !.cpuAccess = "Volatile"]

UnsafeCpuUnmapDischargesDma ==
    /\ s.failedMapReturned /\ s.mapUncertain /\ s.cpuMapped
    /\ s' = [s EXCEPT !.cpuMapped = FALSE, !.pin = "None", !.backing = FALSE, !.charge = FALSE]

UnsafeAbandonReleases ==
    /\ s.abandoned /\ s.abandonedUnit \in {"Map", "Unmap"}
    /\ s' = [s EXCEPT !.root = FALSE, !.claim = FALSE, !.pin = "None",
        !.unit = "None", !.engine = TRUE, !.cell = TRUE]

UnsafeWaitMasked ==
    /\ s.phase = "MapAdmission" /\ s.context = "Masked" /\ ~Available
    /\ s' = [s EXCEPT !.phase = "Waiting"]

UnsafeFinalBusyRearms ==
    /\ s.phase = "UnmapFailedRetained"
    /\ s' = [s EXCEPT !.unmapState = "Active"]

UnsafeCoherentRemap ==
    /\ s.phase \in {"MapFailed", "MapFailedRetained"} /\ s.mode = "Shared"
    /\ s.mapUncertain /\ ~s.cpuMapped /\ s.pin = "Quarantine"
    /\ s' = [s EXCEPT !.phase = "CpuOwned", !.cpuMapped = TRUE, !.cpuAccess = "Rust"]

UnsafeCellSpec == Init /\ [][Next \/ UnsafeClaimDetachedCell]_vars
UnsafeReadinessSpec == Init /\ [][Next \/ UnsafeReadinessCompletes]_vars
UnsafePreparationSpec == Init /\ [][Next \/ UnsafeRejectDropsPreparation]_vars
UnsafeMapSpec == Init /\ [][Next \/ UnsafeReplayMap]_vars
UnsafeRearmSpec == Init /\ [][Next \/ UnsafeRearmStartedUnmap]_vars
UnsafeDropSpec == Init /\ [][Next \/ UnsafeDropResubmits]_vars
UnsafeConfirmationSpec == Init /\ [][Next \/ UnsafeForgetDmaConfirmation]_vars
UnsafeAccessSpec == Init /\ [][Next \/ UnsafeFailedSharedAccess]_vars
UnsafeCpuSpec == Init /\ [][Next \/ UnsafeCpuUnmapDischargesDma]_vars
UnsafeAbandonSpec == Init /\ [][Next \/ UnsafeAbandonReleases]_vars
UnsafeContextSpec == Init /\ [][Next \/ UnsafeWaitMasked]_vars
UnsafePublicSpec == Init /\ [][Next \/ UnsafeFinalBusyRearms]_vars
UnsafeRemapSpec == Init /\ [][Next \/ UnsafeCoherentRemap]_vars

TypeOK ==
    /\ s.phase \in Phases /\ s.mode \in Modes /\ s.context \in Contexts
    /\ s.unit \in UnitOwners /\ s.engine \in BOOLEAN /\ s.cell \in BOOLEAN
    /\ s.epoch \in 1..MaxEpoch /\ s.interestEpoch \in 0..MaxEpoch
    /\ s.sourceStorage \in BOOLEAN /\ s.interest \in BOOLEAN /\ s.registered \in BOOLEAN
    /\ s.notice \in BOOLEAN /\ s.sourceClosed \in BOOLEAN
    /\ s.root \in BOOLEAN /\ s.claim \in BOOLEAN
    /\ s.rootGeneration = 1 /\ s.capturedRoot = 1 /\ s.pinRoot = 1 /\ s.sponsor = 1
    /\ s.pin \in PinOwners /\ s.backing \in BOOLEAN /\ s.charge \in BOOLEAN
    /\ s.cpuMapped \in BOOLEAN /\ s.cpuAccess \in {"Owned", "Restricted", "Volatile", "None", "Rust"}
    /\ s.closeRequested \in BOOLEAN /\ s.mapSubmissions \in 0..1 /\ s.mapWork \in 0..1
    /\ s.mapClaimProof \in BOOLEAN /\ s.mapResult \in {"None", "Complete", "Error"}
    /\ s.mapUncertain \in BOOLEAN /\ s.failedMapReturned \in BOOLEAN
    /\ s.unmapState \in {"Active", "Retained", "Complete"}
    /\ s.unmapSubmissions \in 0..MaxUnmapAttempts /\ s.unmapWork \in 0..1
    /\ s.unmapResult \in UnmapResults /\ s.uncertainAt \in 0..MaxUnmapAttempts
    /\ s.confirmedAt \in 0..MaxUnmapAttempts /\ s.backendAck \in BOOLEAN
    /\ s.dmaConfirmed \in BOOLEAN /\ s.hardwarePossible \in BOOLEAN
    /\ s.startedFailure \in BOOLEAN /\ s.dropRequested \in BOOLEAN /\ s.cpuFailure \in BOOLEAN
    /\ s.rootCloseRejected \in BOOLEAN /\ s.abandoned \in BOOLEAN
    /\ s.abandonedPin \in PinOwners /\ s.abandonedUnit \in UnitOwners
    /\ s.abandonedStorage \in BOOLEAN

ClaimNeedsCompleteUnit == s.mapWork > 0 => s.mapClaimProof
ReadinessIsNotCompletion == s.dmaConfirmed => s.backendAck
PreclaimRetainsPreparation == Preclaim =>
    s.root /\ s.claim /\ s.pin = "Preparation" /\ s.backing /\ s.charge
MapSubmittedOnce == s.mapSubmissions <= 1
StartedFailureNotRearmed == s.startedFailure => s.unmapState # "Active"
UncertainAttemptNotResubmitted == s.uncertainAt > 0 => s.unmapSubmissions = s.uncertainAt
ConfirmationSticky == s.dmaConfirmed =>
    s.unmapState = "Complete" /\ s.backendAck /\ s.pin = "None" /\
        s.unmapSubmissions = s.confirmedAt
FailedSharedRestricted == s.failedMapReturned /\ s.mode = "Shared" /\
    s.phase \in {"MapFailed", "MapFailedRetained"} =>
    s.cpuAccess \in {"Restricted", "None"}
PossibleDmaRetainsBacking == s.hardwarePossible => s.pin # "None" /\ s.backing /\ s.charge
AbandonmentRetainsOwner == s.abandoned =>
    s.root /\ s.claim /\ s.pin = s.abandonedPin /\ s.backing /\ s.charge /\
    (s.abandonedStorage => s.sourceStorage) /\
    (s.abandonedUnit \in {"Map", "Unmap"} =>
        s.unit = s.abandonedUnit /\ ~s.engine /\ ~s.cell)
WaitNeedsQualifiedContext == s.phase = "Waiting" => Qualified
RootAndPinKeepIdentity == s.root =>
    s.rootGeneration = s.capturedRoot /\ s.capturedRoot = s.pinRoot /\ s.sponsor = s.capturedRoot
ClaimKeepsRoot == s.claim => s.root
CpuOwnerNeedsNoDma == s.cpuAccess \in {"Owned", "Rust"} => ~s.hardwarePossible /\ s.pin = "None"
BackingChargeRetained == s.backing = s.charge
HeldPinRetainsBacking == s.pin # "None" => s.backing /\ s.charge
PublicRejectionRetainsAttempt == s.phase = "UnmapFailedRetained" => s.unmapState = "Retained"
KernelOperationRetainsOwner ==
    s.phase \in {"MapRunning", "MapReturned", "UnmapAdmission", "UnmapRunning", "UnmapReturned"} =>
        s.root /\ s.claim /\ s.backing /\ s.charge
MaintenanceOwnsUnit == s.unit \in {"Map", "Unmap"} =>
    ~s.engine /\ ~s.cell /\ s.pin = "Maintenance" /\ s.root /\ s.claim
AcknowledgementNeedsWork == s.backendAck => s.unmapWork = 1
FailedMapHasNoUnmapSubmission == s.failedMapReturned => s.unmapSubmissions = 0
ExclusiveHasNoCpuMapping == s.mode = "Exclusive" => ~s.cpuMapped

Invariants == TypeOK /\ ClaimNeedsCompleteUnit /\ ReadinessIsNotCompletion
    /\ PreclaimRetainsPreparation /\ MapSubmittedOnce /\ StartedFailureNotRearmed
    /\ UncertainAttemptNotResubmitted /\ ConfirmationSticky /\ FailedSharedRestricted
    /\ PossibleDmaRetainsBacking /\ AbandonmentRetainsOwner /\ WaitNeedsQualifiedContext
    /\ RootAndPinKeepIdentity /\ ClaimKeepsRoot /\ CpuOwnerNeedsNoDma
    /\ BackingChargeRetained /\ HeldPinRetainsBacking
    /\ PublicRejectionRetainsAttempt /\ KernelOperationRetainsOwner
    /\ MaintenanceOwnsUnit /\ AcknowledgementNeedsWork
    /\ FailedMapHasNoUnmapSubmission /\ ExclusiveHasNoCpuMapping

=============================================================================
