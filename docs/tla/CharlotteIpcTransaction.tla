------------------------- MODULE CharlotteIpcTransaction -------------------------
\* IPC attachment transactions across memory, IPC-capability, queue, and
\* pending-call registries. Vector calls carry the memory entries; scalar
\* calls carry a connection attachment and may also carry copied memory. The
\* model deliberately composes those concrete paths into a stronger rollback
\* obligation, and also isolates a non-terminal reply timeout.

EXTENDS Naturals, FiniteSets

CONSTANT Items

ASSUME Cardinality(Items) > 1

Phases == {"Idle", "Preparing", "Queued", "Delivered", "RolledBack", "Closed"}
CallStates == {"None", "Pending", "Replied", "Observed"}

VARIABLES phase, transferred, senderOwns, receiverOwns, loanActive,
          connectionAttached, queueVisible, callState, timeoutObserved

vars == <<phase, transferred, senderOwns, receiverOwns, loanActive,
          connectionAttached, queueVisible, callState, timeoutObserved>>

Init ==
    /\ phase = "Idle"
    /\ transferred = {}
    /\ senderOwns = [i \in Items |-> TRUE]
    /\ receiverOwns = [i \in Items |-> FALSE]
    /\ loanActive = FALSE
    /\ connectionAttached = FALSE
    /\ queueVisible = FALSE
    /\ callState = "None"
    /\ timeoutObserved = FALSE

BeginVector ==
    /\ phase = "Idle"
    /\ phase' = "Preparing"
    /\ UNCHANGED <<transferred, senderOwns, receiverOwns, loanActive,
                    connectionAttached, queueVisible, callState, timeoutObserved>>

TransferMove(i) ==
    /\ phase = "Preparing"
    /\ i \in Items \ transferred
    /\ senderOwns[i]
    /\ transferred' = transferred \union {i}
    /\ senderOwns' = [senderOwns EXCEPT ![i] = FALSE]
    /\ receiverOwns' = [receiverOwns EXCEPT ![i] = TRUE]
    /\ UNCHANGED <<phase, loanActive, connectionAttached, queueVisible,
                    callState, timeoutObserved>>

BeginBorrow ==
    /\ phase = "Preparing"
    /\ ~loanActive
    /\ loanActive' = TRUE
    /\ UNCHANGED <<phase, transferred, senderOwns, receiverOwns,
                    connectionAttached, queueVisible, callState, timeoutObserved>>

AttachConnection ==
    /\ phase = "Preparing"
    /\ ~connectionAttached
    /\ connectionAttached' = TRUE
    /\ UNCHANGED <<phase, transferred, senderOwns, receiverOwns, loanActive,
                    queueVisible, callState, timeoutObserved>>

Commit ==
    /\ phase = "Preparing"
    /\ transferred = Items
    /\ loanActive
    /\ connectionAttached
    /\ phase' = "Queued"
    /\ queueVisible' = TRUE
    /\ callState' = "Pending"
    /\ UNCHANGED <<transferred, senderOwns, receiverOwns, loanActive,
                    connectionAttached, timeoutObserved>>

FailAndRollback ==
    /\ phase = "Preparing"
    /\ transferred /= {} \/ loanActive \/ connectionAttached
    /\ phase' = "RolledBack"
    /\ transferred' = {}
    /\ senderOwns' = [i \in Items |-> TRUE]
    /\ receiverOwns' = [i \in Items |-> FALSE]
    /\ loanActive' = FALSE
    /\ connectionAttached' = FALSE
    /\ queueVisible' = FALSE
    /\ callState' = "None"
    /\ timeoutObserved' = FALSE

Deliver ==
    /\ phase = "Queued"
    /\ phase' = "Delivered"
    /\ queueVisible' = FALSE
    /\ UNCHANGED <<transferred, senderOwns, receiverOwns, loanActive,
                    connectionAttached, callState, timeoutObserved>>

\* A watchdog is only an observation about waiting. It does not terminate the
\* call and therefore cannot release a mutable or immutable memory loan.
WaitTimeout ==
    /\ phase \in {"Queued", "Delivered"}
    /\ callState = "Pending"
    /\ ~timeoutObserved
    /\ timeoutObserved' = TRUE
    /\ UNCHANGED <<phase, transferred, senderOwns, receiverOwns, loanActive,
                    connectionAttached, queueVisible, callState>>

Reply ==
    /\ phase = "Delivered"
    /\ callState = "Pending"
    /\ callState' = "Replied"
    /\ loanActive' = FALSE
    /\ UNCHANGED <<phase, transferred, senderOwns, receiverOwns,
                    connectionAttached, queueVisible, timeoutObserved>>

ObserveReply ==
    /\ phase = "Delivered"
    /\ callState = "Replied"
    /\ phase' = "Closed"
    /\ callState' = "Observed"
    /\ UNCHANGED <<transferred, senderOwns, receiverOwns, loanActive,
                    connectionAttached, queueVisible, timeoutObserved>>

Next ==
    \/ BeginVector
    \/ \E i \in Items : TransferMove(i)
    \/ BeginBorrow
    \/ AttachConnection
    \/ Commit
    \/ FailAndRollback
    \/ Deliver
    \/ WaitTimeout
    \/ Reply
    \/ ObserveReply

Spec == Init /\ [][Next]_vars

\* Regression: one partially moved entry and the attached connection survive
\* a failed multi-registry submission instead of returning to the source.
UnsafeRollback ==
    /\ phase = "Preparing"
    /\ transferred /= {}
    /\ connectionAttached
    /\ \E missed \in transferred :
        /\ phase' = "RolledBack"
        /\ transferred' = {missed}
        /\ senderOwns' =
             [i \in Items |-> IF i = missed THEN FALSE ELSE TRUE]
        /\ receiverOwns' =
             [i \in Items |-> IF i = missed THEN TRUE ELSE FALSE]
        /\ loanActive' = FALSE
        /\ connectionAttached' = TRUE
        /\ queueVisible' = FALSE
        /\ callState' = "None"
        /\ timeoutObserved' = FALSE

\* Regression: the timeout path is mistaken for terminal cancellation and
\* returns the borrowed memory while the pending call still exists.
UnsafeTimeoutRelease ==
    /\ phase \in {"Queued", "Delivered"}
    /\ callState = "Pending"
    /\ loanActive
    /\ timeoutObserved' = TRUE
    /\ loanActive' = FALSE
    /\ UNCHANGED <<phase, transferred, senderOwns, receiverOwns,
                    connectionAttached, queueVisible, callState>>

UnsafeNext == Next \/ UnsafeRollback \/ UnsafeTimeoutRelease
UnsafeSpec == Init /\ [][UnsafeNext]_vars

TypeOK ==
    /\ phase \in Phases
    /\ transferred \subseteq Items
    /\ senderOwns \in [Items -> BOOLEAN]
    /\ receiverOwns \in [Items -> BOOLEAN]
    /\ loanActive \in BOOLEAN
    /\ connectionAttached \in BOOLEAN
    /\ queueVisible \in BOOLEAN
    /\ callState \in CallStates
    /\ timeoutObserved \in BOOLEAN

MovedOwnershipUnique ==
    \A i \in Items : senderOwns[i] # receiverOwns[i]

RegistryProjectionConsistent ==
    transferred = {i \in Items : receiverOwns[i]}

RollbackRestoresAll ==
    phase = "RolledBack" =>
        /\ transferred = {}
        /\ \A i \in Items : senderOwns[i] /\ ~receiverOwns[i]
        /\ ~loanActive
        /\ ~connectionAttached
        /\ ~queueVisible
        /\ callState = "None"

TimeoutRetainsLoan ==
    timeoutObserved /\ callState = "Pending" => loanActive

AttachedConnectionHasTransaction ==
    connectionAttached => phase \notin {"Idle", "RolledBack"}

Invariants ==
    /\ TypeOK
    /\ MovedOwnershipUnique
    /\ RegistryProjectionConsistent
    /\ RollbackRestoresAll
    /\ TimeoutRetainsLoan
    /\ AttachedConnectionHasTransaction

=============================================================================
