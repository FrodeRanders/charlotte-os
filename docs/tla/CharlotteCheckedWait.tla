------------------------- MODULE CharlotteCheckedWait -------------------------
\* One already accepted operation and its borrowed buffer. Source epochs are
\* readiness episodes, not operation completion proofs or replay permission.

EXTENDS Naturals

CONSTANTS Budget, MaxEpoch
ASSUME Budget > 0
ASSUME MaxEpoch > 1

Phases == {"Idle", "Preparing", "Armed", "Parked", "Woken", "Ready",
           "Rejected", "TimedOut", "Done", "Abandoned"}
OperationStates == {"Pending", "Completed", "Cancelled"}

VARIABLES phase, operation, now, deadline, epoch, capturedEpoch,
          watchdog, source, registered, wakePending, owner, loan,
          submissions, returnedHidden, returnedOwned, rejection

vars == <<phase, operation, now, deadline, epoch, capturedEpoch,
          watchdog, source, registered, wakePending, owner, loan,
          submissions, returnedHidden, returnedOwned, rejection>>

Init ==
    /\ phase = "Idle"
    /\ operation = "Pending"
    /\ now = 0 /\ deadline = Budget
    /\ epoch = 1 /\ capturedEpoch = 1
    /\ watchdog = FALSE /\ source = FALSE /\ registered = FALSE
    /\ wakePending = FALSE
    /\ owner = TRUE /\ loan = TRUE /\ submissions = 1
    /\ returnedHidden = FALSE /\ returnedOwned = FALSE
    /\ rejection = "None"

BeginWait ==
    /\ phase = "Idle"
    /\ phase' = "Preparing"
    /\ capturedEpoch' = epoch
    /\ rejection' = "None"
    /\ UNCHANGED <<operation, now, deadline, epoch, watchdog, source,
                   registered, wakePending, owner, loan, submissions,
                   returnedHidden, returnedOwned>>

PrepareWatchdog ==
    /\ phase = "Preparing" /\ ~watchdog
    /\ watchdog' = TRUE
    /\ UNCHANGED <<phase, operation, now, deadline, epoch, capturedEpoch,
                   source, registered, wakePending, owner, loan,
                   submissions, returnedHidden, returnedOwned, rejection>>

RejectWatchdog ==
    /\ phase = "Preparing" /\ ~watchdog
    /\ phase' = "Rejected" /\ rejection' = "Watchdog"
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch,
                   watchdog, source, registered, wakePending, owner, loan,
                   submissions, returnedHidden, returnedOwned>>

PrepareSource ==
    /\ phase = "Preparing" /\ watchdog /\ ~source
    /\ source' = TRUE
    /\ UNCHANGED <<phase, operation, now, deadline, epoch, capturedEpoch,
                   watchdog, registered, wakePending, owner, loan,
                   submissions, returnedHidden, returnedOwned, rejection>>

RejectSource ==
    /\ phase = "Preparing" /\ watchdog /\ ~source
    /\ phase' = "Rejected" /\ rejection' = "Source"
    /\ watchdog' = FALSE
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch, source,
                   registered, wakePending, owner, loan, submissions,
                   returnedHidden, returnedOwned>>

Register ==
    /\ phase = "Preparing" /\ watchdog /\ source
    /\ phase' = "Armed" /\ registered' = TRUE
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch, watchdog,
                   source, wakePending, owner, loan, submissions,
                   returnedHidden, returnedOwned, rejection>>

\* Preparation consumes budget. Recheck time, exact episode and the durable
\* result after registration, before publishing the park.
PublishPark ==
    /\ phase = "Armed"
    /\ IF operation # "Pending" \/ epoch # capturedEpoch \/ wakePending
        THEN /\ phase' = "Woken"
             /\ watchdog' = FALSE /\ source' = FALSE /\ registered' = FALSE
             /\ wakePending' = FALSE
        ELSE IF now >= deadline
             THEN /\ phase' = "TimedOut"
                  /\ watchdog' = FALSE /\ source' = FALSE /\ registered' = FALSE
                  /\ wakePending' = FALSE
             ELSE /\ phase' = "Parked"
                  /\ UNCHANGED <<watchdog, source, registered, wakePending>>
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch, owner,
                   loan, submissions, returnedHidden, returnedOwned, rejection>>

PublishTerminal(result) ==
    /\ operation = "Pending"
    /\ result \in {"Completed", "Cancelled"}
    /\ operation' = result
    /\ returnedHidden' = (result = "Completed")
    /\ wakePending' = registered
    /\ UNCHANGED <<phase, now, deadline, epoch, capturedEpoch, watchdog,
                   source, registered, owner, loan, submissions,
                   returnedOwned, rejection>>

\* Rollback followed by another claim is a new episode. Closing the old
\* source wakes its subscriber, without completing the accepted operation.
ChangeEpisode ==
    /\ operation = "Pending" /\ epoch < MaxEpoch
    /\ epoch' = epoch + 1
    /\ wakePending' = registered
    /\ UNCHANGED <<phase, operation, now, deadline, capturedEpoch, watchdog,
                   source, registered, owner, loan, submissions,
                   returnedHidden, returnedOwned, rejection>>

Wake ==
    /\ phase = "Parked"
    /\ wakePending \/ now >= deadline
    /\ phase' = "Woken"
    /\ watchdog' = FALSE /\ source' = FALSE /\ registered' = FALSE
    /\ wakePending' = FALSE
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch, owner,
                   loan, submissions, returnedHidden, returnedOwned, rejection>>

UnrelatedWake ==
    /\ phase = "Parked"
    /\ phase' = "Woken"
    /\ watchdog' = FALSE /\ source' = FALSE /\ registered' = FALSE
    /\ wakePending' = FALSE
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch, owner,
                   loan, submissions, returnedHidden, returnedOwned, rejection>>

Recheck ==
    /\ phase \in {"Woken", "Rejected"}
    /\ phase' = IF operation # "Pending" THEN "Ready"
                 ELSE IF now >= deadline THEN "TimedOut" ELSE "Idle"
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch, watchdog,
                   source, registered, wakePending, owner, loan, submissions,
                   returnedHidden, returnedOwned, rejection>>

ObserveTerminal ==
    /\ phase \in {"Idle", "Woken", "Ready", "Rejected", "TimedOut"}
    /\ operation # "Pending"
    /\ phase' = "Done" /\ owner' = FALSE /\ loan' = FALSE
    /\ returnedOwned' = returnedHidden /\ returnedHidden' = FALSE
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch, watchdog,
                   source, registered, wakePending, submissions, rejection>>

AdvanceTime ==
    /\ now < Budget + 1
    /\ now' = now + 1
    /\ UNCHANGED <<phase, operation, deadline, epoch, capturedEpoch,
                   watchdog, source, registered, wakePending, owner, loan,
                   submissions, returnedHidden, returnedOwned, rejection>>

\* An uncertain outer teardown retains operation, borrow and subscription
\* storage. Its later disposal/abort policy is outside this safety model.
Abandon ==
    /\ phase \notin {"Done", "Abandoned"}
    /\ phase' = "Abandoned"
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch, watchdog,
                   source, registered, wakePending, owner, loan, submissions,
                   returnedHidden, returnedOwned, rejection>>

Next == BeginWait \/ PrepareWatchdog \/ RejectWatchdog \/ PrepareSource
    \/ RejectSource \/ Register \/ PublishPark \/ ChangeEpisode \/ Wake
    \/ UnrelatedWake \/ Recheck \/ ObserveTerminal \/ AdvanceTime \/ Abandon
    \/ \E result \in {"Completed", "Cancelled"} : PublishTerminal(result)

Spec == Init /\ [][Next]_vars

UnsafeRejectedRelease ==
    /\ phase = "Rejected" /\ operation = "Pending"
    /\ loan' = FALSE /\ owner' = FALSE
    /\ UNCHANGED <<phase, operation, now, deadline, epoch, capturedEpoch,
                   watchdog, source, registered, wakePending, submissions,
                   returnedHidden, returnedOwned, rejection>>

UnsafeReplay ==
    /\ phase = "TimedOut" /\ operation = "Pending"
    /\ submissions' = 2
    /\ UNCHANGED <<phase, operation, now, deadline, epoch, capturedEpoch,
                   watchdog, source, registered, wakePending, owner, loan,
                   returnedHidden, returnedOwned, rejection>>

UnsafeRestartDeadline ==
    /\ phase = "Woken" /\ operation = "Pending" /\ now > 0
    /\ deadline' = now + Budget
    /\ UNCHANGED <<phase, operation, now, epoch, capturedEpoch, watchdog,
                   source, registered, wakePending, owner, loan, submissions,
                   returnedHidden, returnedOwned, rejection>>

UnsafeWakeAdoptsResult ==
    /\ phase = "Woken" /\ operation = "Pending"
    /\ phase' = "Done" /\ returnedOwned' = TRUE
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch, watchdog,
                   source, registered, wakePending, owner, loan, submissions,
                   returnedHidden, rejection>>

UnsafeParkWithoutRecheck ==
    /\ phase = "Armed" /\ operation # "Pending" /\ ~wakePending
    /\ phase' = "Parked"
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch, watchdog,
                   source, registered, wakePending, owner, loan, submissions,
                   returnedHidden, returnedOwned, rejection>>

UnsafeAdmissionTimeout ==
    /\ phase = "Rejected" /\ now < deadline
    /\ phase' = "TimedOut"
    /\ UNCHANGED <<operation, now, deadline, epoch, capturedEpoch, watchdog,
                   source, registered, wakePending, owner, loan, submissions,
                   returnedHidden, returnedOwned, rejection>>

UnsafeRejectionSpec == Init /\ [][Next \/ UnsafeRejectedRelease]_vars
UnsafeReplaySpec == Init /\ [][Next \/ UnsafeReplay]_vars
UnsafeDeadlineSpec == Init /\ [][Next \/ UnsafeRestartDeadline]_vars
UnsafeResultSpec == Init /\ [][Next \/ UnsafeWakeAdoptsResult]_vars
UnsafeParkSpec == Init /\ [][Next \/ UnsafeParkWithoutRecheck]_vars
UnsafeTimeoutSpec == Init /\ [][Next \/ UnsafeAdmissionTimeout]_vars

TypeOK ==
    /\ phase \in Phases /\ operation \in OperationStates
    /\ now \in 0..(Budget + 1) /\ deadline \in 0..(2 * Budget + 1)
    /\ epoch \in 1..MaxEpoch /\ capturedEpoch \in 1..MaxEpoch
    /\ watchdog \in BOOLEAN /\ source \in BOOLEAN /\ registered \in BOOLEAN
    /\ wakePending \in BOOLEAN /\ owner \in BOOLEAN /\ loan \in BOOLEAN
    /\ submissions = 1
    /\ returnedHidden \in BOOLEAN /\ returnedOwned \in BOOLEAN
    /\ rejection \in {"None", "Watchdog", "Source"}

PendingRetainsOwnerAndLoan == operation = "Pending" => owner /\ loan
OneSubmission == submissions = 1
DeadlineUnchanged == deadline = Budget
ResultNeedsTerminalProof == returnedOwned => operation = "Completed" /\ phase = "Done"
ParkedTerminalHasWake == phase = "Parked" /\ operation # "Pending" => wakePending
TimeoutRespectsDeadline == phase = "TimedOut" => now >= Budget
UnobservedResultHidden == operation = "Completed" /\ phase # "Done" => returnedHidden
WaitStorageBalanced ==
    /\ (registered => watchdog /\ source)
    /\ (phase \in {"Idle", "Woken", "Ready", "Rejected", "TimedOut", "Done"} =>
            ~watchdog /\ ~source /\ ~registered)

Invariants == TypeOK /\ PendingRetainsOwnerAndLoan /\ OneSubmission
    /\ DeadlineUnchanged /\ ResultNeedsTerminalProof /\ ParkedTerminalHasWake
    /\ TimeoutRespectsDeadline /\ UnobservedResultHidden /\ WaitStorageBalanced

=============================================================================
