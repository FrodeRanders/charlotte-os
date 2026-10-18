---------------------- MODULE CharlotteRootRetirement ----------------------
\* One reusable ASID, exact-generation leases, and complete retirement owners.
\* Thread quiescence and each subsystem's cleanup proof are boundary inputs.

EXTENDS Naturals, FiniteSets

CONSTANTS Operations, MaxGeneration, MaxAttempts

ASSUME Operations # {}
ASSUME MaxGeneration > 1
ASSUME MaxAttempts > 1

Roots == 1..MaxGeneration
Phases == {"Unused", "Live", "Closing", "Detached", "Invalidating",
           "Retained", "Invalidated", "Physical", "Released", "Partial",
           "Abandoned"}
Owners == {"None", "Active", "Custody", "Attempt", "Abandoned"}
Stages == {"Devices", "IPC", "Memory"}
OperationStates == {"Free", "Active", "Done", "Abandoned"}

VARIABLES phase, owner, slot, generation, operationState, operationRoot,
          cleanup, pendingSeen, invalidated, attempts, physicalAttempts,
          pages, charges, metadata, abandoned

vars == <<phase, owner, slot, generation, operationState, operationRoot,
          cleanup, pendingSeen, invalidated, attempts, physicalAttempts,
          pages, charges, metadata, abandoned>>

Init ==
    /\ phase = [g \in Roots |-> "Unused"]
    /\ owner = [g \in Roots |-> "None"]
    /\ slot = 0
    /\ generation = 0
    /\ operationState = [op \in Operations |-> "Free"]
    /\ operationRoot = [op \in Operations |-> 0]
    /\ cleanup = [g \in Roots |-> {}]
    /\ pendingSeen = [g \in Roots |-> FALSE]
    /\ invalidated = [g \in Roots |-> FALSE]
    /\ attempts = [g \in Roots |-> 0]
    /\ physicalAttempts = [g \in Roots |-> 0]
    /\ pages = [g \in Roots |-> 0]
    /\ charges = [g \in Roots |-> 0]
    /\ metadata = [g \in Roots |-> FALSE]
    /\ abandoned = [g \in Roots |-> FALSE]

Leased(g) ==
    \E op \in Operations : operationRoot[op] = g /\
        operationState[op] \in {"Active", "Abandoned"}

Allocate ==
    /\ slot = 0
    /\ generation < MaxGeneration
    /\ LET g == generation + 1 IN
        /\ phase' = [phase EXCEPT ![g] = "Live"]
        /\ owner' = [owner EXCEPT ![g] = "Active"]
        /\ pages' = [pages EXCEPT ![g] = 2]
        /\ charges' = [charges EXCEPT ![g] = 2]
        /\ metadata' = [metadata EXCEPT ![g] = TRUE]
        /\ slot' = g
        /\ generation' = g
    /\ UNCHANGED <<operationState, operationRoot, cleanup, pendingSeen,
                   invalidated, attempts, physicalAttempts, abandoned>>

Acquire(op, g) ==
    /\ slot = g /\ phase[g] = "Live"
    /\ operationState[op] = "Free"
    /\ operationState' = [operationState EXCEPT ![op] = "Active"]
    /\ operationRoot' = [operationRoot EXCEPT ![op] = g]
    /\ UNCHANGED <<phase, owner, slot, generation, cleanup, pendingSeen,
                   invalidated, attempts, physicalAttempts, pages, charges,
                   metadata, abandoned>>

FinishOperation(op) ==
    /\ operationState[op] = "Active"
    /\ operationState' = [operationState EXCEPT ![op] = "Done"]
    /\ UNCHANGED <<phase, owner, slot, generation, operationRoot, cleanup,
                   pendingSeen, invalidated, attempts, physicalAttempts,
                   pages, charges, metadata, abandoned>>

AbandonOperation(op) ==
    /\ operationState[op] = "Active"
    /\ operationState' = [operationState EXCEPT ![op] = "Abandoned"]
    /\ UNCHANGED <<phase, owner, slot, generation, operationRoot, cleanup,
                   pendingSeen, invalidated, attempts, physicalAttempts,
                   pages, charges, metadata, abandoned>>

BeginClose(g) ==
    /\ slot = g /\ phase[g] = "Live"
    /\ phase' = [phase EXCEPT ![g] = "Closing"]
    /\ UNCHANGED <<owner, slot, generation, operationState, operationRoot,
                   cleanup, pendingSeen, invalidated, attempts,
                   physicalAttempts, pages, charges, metadata, abandoned>>

ClosePending(g) ==
    /\ phase[g] = "Closing" /\ Leased(g) /\ ~pendingSeen[g]
    /\ pendingSeen' = [pendingSeen EXCEPT ![g] = TRUE]
    /\ UNCHANGED <<phase, owner, slot, generation, operationState,
                   operationRoot, cleanup, invalidated, attempts,
                   physicalAttempts, pages, charges, metadata, abandoned>>

FinishCleanup(g, stage) ==
    /\ phase[g] = "Closing" /\ ~Leased(g)
    /\ stage \notin cleanup[g]
    /\ (stage = "IPC" => "Devices" \in cleanup[g])
    /\ (stage = "Memory" => "IPC" \in cleanup[g])
    /\ cleanup' = [cleanup EXCEPT ![g] = @ \union {stage}]
    /\ UNCHANGED <<phase, owner, slot, generation, operationState,
                   operationRoot, pendingSeen, invalidated, attempts,
                   physicalAttempts, pages, charges, metadata, abandoned>>

Detach(g) ==
    /\ phase[g] = "Closing" /\ ~Leased(g) /\ cleanup[g] = Stages
    /\ phase' = [phase EXCEPT ![g] = "Detached"]
    /\ UNCHANGED <<owner, slot, generation, operationState, operationRoot,
                   cleanup, pendingSeen, invalidated, attempts,
                   physicalAttempts, pages, charges, metadata, abandoned>>

BeginInvalidation(g) ==
    /\ phase[g] = "Detached" /\ owner[g] = "Active"
    /\ phase' = [phase EXCEPT ![g] = "Invalidating"]
    /\ attempts' = [attempts EXCEPT ![g] = @ + 1]
    /\ UNCHANGED <<owner, slot, generation, operationState, operationRoot,
                   cleanup, pendingSeen, invalidated, physicalAttempts,
                   pages, charges, metadata, abandoned>>

InvalidationFailed(g) ==
    /\ phase[g] = "Invalidating"
    /\ phase' = [phase EXCEPT ![g] = "Retained"]
    /\ owner' = [owner EXCEPT ![g] = "Active"]
    /\ UNCHANGED <<slot, generation, operationState, operationRoot, cleanup,
                   pendingSeen, invalidated, attempts, physicalAttempts,
                   pages, charges, metadata, abandoned>>

RetainForRetry(g) ==
    /\ phase[g] = "Retained" /\ owner[g] = "Active"
    /\ owner' = [owner EXCEPT ![g] = "Custody"]
    /\ UNCHANGED <<phase, slot, generation, operationState, operationRoot,
                   cleanup, pendingSeen, invalidated, attempts,
                   physicalAttempts, pages, charges, metadata, abandoned>>

\* Custody transfers the complete owner. A diagnostic ticket alone cannot
\* enable this action; registry admission and retry limits are abstracted.
ClaimRetry(g) ==
    /\ phase[g] = "Retained" /\ owner[g] = "Custody"
    /\ attempts[g] < MaxAttempts
    /\ phase' = [phase EXCEPT ![g] = "Invalidating"]
    /\ owner' = [owner EXCEPT ![g] = "Attempt"]
    /\ attempts' = [attempts EXCEPT ![g] = @ + 1]
    /\ UNCHANGED <<slot, generation, operationState, operationRoot, cleanup,
                   pendingSeen, invalidated, physicalAttempts, pages,
                   charges, metadata, abandoned>>

InvalidationSucceeded(g) ==
    /\ phase[g] = "Invalidating"
    /\ phase' = [phase EXCEPT ![g] = "Invalidated"]
    /\ invalidated' = [invalidated EXCEPT ![g] = TRUE]
    /\ owner' = [owner EXCEPT ![g] = "Active"]
    /\ UNCHANGED <<slot, generation, operationState, operationRoot, cleanup,
                   pendingSeen, attempts, physicalAttempts, pages, charges,
                   metadata, abandoned>>

\* Release detached metadata after the proof, before one physical walk.
StartPhysical(g) ==
    /\ phase[g] = "Invalidated" /\ invalidated[g]
    /\ phase' = [phase EXCEPT ![g] = "Physical"]
    /\ metadata' = [metadata EXCEPT ![g] = FALSE]
    /\ physicalAttempts' = [physicalAttempts EXCEPT ![g] = @ + 1]
    /\ UNCHANGED <<owner, slot, generation, operationState, operationRoot,
                   cleanup, pendingSeen, invalidated, attempts, pages,
                   charges, abandoned>>

FinishPhysical(g, rejected) ==
    /\ phase[g] = "Physical" /\ slot = g
    /\ rejected \in 0..1
    /\ phase' = [phase EXCEPT ![g] = IF rejected = 0 THEN "Released" ELSE "Partial"]
    /\ pages' = [pages EXCEPT ![g] = rejected]
    /\ charges' = [charges EXCEPT ![g] = IF rejected = 0 THEN 0 ELSE @]
    /\ slot' = 0
    /\ owner' = [owner EXCEPT ![g] = "None"]
    /\ UNCHANGED <<generation, operationState, operationRoot, cleanup,
                   pendingSeen, invalidated, attempts, physicalAttempts,
                   metadata, abandoned>>

\* Includes rejected custody admission and consuming cleanup failure.
\* No ordinary continuation can recover this abandoned owner.
Abandon(g) ==
    /\ phase[g] \in {"Closing", "Detached", "Invalidating", "Retained", "Invalidated"}
    /\ owner[g] \in {"Active", "Attempt"}
    /\ phase' = [phase EXCEPT ![g] = "Abandoned"]
    /\ owner' = [owner EXCEPT ![g] = "Abandoned"]
    /\ abandoned' = [abandoned EXCEPT ![g] = TRUE]
    /\ UNCHANGED <<slot, generation, operationState, operationRoot, cleanup,
                   pendingSeen, invalidated, attempts, physicalAttempts,
                   pages, charges, metadata>>

Next ==
    \/ Allocate
    \/ \E op \in Operations, g \in Roots : Acquire(op, g)
    \/ \E op \in Operations : FinishOperation(op) \/ AbandonOperation(op)
    \/ \E g \in Roots :
        BeginClose(g) \/ ClosePending(g) \/ Detach(g) \/ BeginInvalidation(g)
        \/ InvalidationFailed(g) \/ RetainForRetry(g) \/ ClaimRetry(g)
        \/ InvalidationSucceeded(g) \/ StartPhysical(g) \/ Abandon(g)
    \/ \E g \in Roots, stage \in Stages : FinishCleanup(g, stage)
    \/ \E g \in Roots, rejected \in 0..1 : FinishPhysical(g, rejected)

Spec == Init /\ [][Next]_vars

UnsafeDetachWithLease(g) ==
    /\ phase[g] = "Closing" /\ Leased(g)
    /\ phase' = [phase EXCEPT ![g] = "Detached"]
    /\ UNCHANGED <<owner, slot, generation, operationState, operationRoot,
                   cleanup, pendingSeen, invalidated, attempts,
                   physicalAttempts, pages, charges, metadata, abandoned>>

UnsafeReleaseBeforeInvalidation(g) ==
    /\ phase[g] \in {"Detached", "Invalidating", "Retained"}
    /\ phase' = [phase EXCEPT ![g] = "Released"]
    /\ pages' = [pages EXCEPT ![g] = 0]
    /\ charges' = [charges EXCEPT ![g] = 0]
    /\ metadata' = [metadata EXCEPT ![g] = FALSE]
    /\ slot' = 0
    /\ owner' = [owner EXCEPT ![g] = "None"]
    /\ UNCHANGED <<generation, operationState, operationRoot, cleanup,
                   pendingSeen, invalidated, attempts, physicalAttempts, abandoned>>

UnsafeRetryPhysical(g) ==
    /\ phase[g] = "Partial"
    /\ physicalAttempts' = [physicalAttempts EXCEPT ![g] = @ + 1]
    /\ UNCHANGED <<phase, owner, slot, generation, operationState,
                   operationRoot, cleanup, pendingSeen, invalidated,
                   attempts, pages, charges, metadata, abandoned>>

UnsafeAdoptAbandoned(g) ==
    /\ phase[g] = "Abandoned"
    /\ phase' = [phase EXCEPT ![g] = "Detached"]
    /\ owner' = [owner EXCEPT ![g] = "Active"]
    /\ UNCHANGED <<slot, generation, operationState, operationRoot, cleanup,
                   pendingSeen, invalidated, attempts, physicalAttempts,
                   pages, charges, metadata, abandoned>>

UnsafeRefundPartial(g) ==
    /\ phase[g] = "Partial"
    /\ charges' = [charges EXCEPT ![g] = 0]
    /\ UNCHANGED <<phase, owner, slot, generation, operationState,
                   operationRoot, cleanup, pendingSeen, invalidated,
                   attempts, physicalAttempts, pages, metadata, abandoned>>

UnsafeLeaseSpec == Init /\ [][Next \/ \E g \in Roots : UnsafeDetachWithLease(g)]_vars
UnsafeReleaseSpec == Init /\ [][Next \/ \E g \in Roots : UnsafeReleaseBeforeInvalidation(g)]_vars
UnsafePhysicalSpec == Init /\ [][Next \/ \E g \in Roots : UnsafeRetryPhysical(g)]_vars
UnsafeAbandonSpec == Init /\ [][Next \/ \E g \in Roots : UnsafeAdoptAbandoned(g)]_vars
UnsafeRefundSpec == Init /\ [][Next \/ \E g \in Roots : UnsafeRefundPartial(g)]_vars

TypeOK ==
    /\ phase \in [Roots -> Phases]
    /\ owner \in [Roots -> Owners]
    /\ slot \in 0..MaxGeneration /\ generation \in 0..MaxGeneration
    /\ operationState \in [Operations -> OperationStates]
    /\ operationRoot \in [Operations -> 0..MaxGeneration]
    /\ cleanup \in [Roots -> SUBSET Stages]
    /\ pendingSeen \in [Roots -> BOOLEAN]
    /\ invalidated \in [Roots -> BOOLEAN]
    /\ attempts \in [Roots -> 0..MaxAttempts]
    /\ physicalAttempts \in [Roots -> 0..1]
    /\ pages \in [Roots -> 0..2] /\ charges \in [Roots -> 0..2]
    /\ metadata \in [Roots -> BOOLEAN]
    /\ abandoned \in [Roots -> BOOLEAN]

LeasesRetainRoot ==
    \A g \in Roots : Leased(g) =>
        phase[g] \in {"Live", "Closing", "Abandoned"} /\ slot = g /\ pages[g] = 2

PhysicalRequiresInvalidation ==
    \A g \in Roots :
        phase[g] # "Unused" /\ (pages[g] < 2 \/ ~metadata[g]) => invalidated[g]

PhysicalOnce == \A g \in Roots : physicalAttempts[g] <= 1
BackingCharged == \A g \in Roots : pages[g] > 0 => charges[g] = 2
TerminalAbandonment ==
    \A g \in Roots : abandoned[g] => phase[g] = "Abandoned" /\ owner[g] = "Abandoned"

CompleteOwnerRetained ==
    \A g \in Roots :
        phase[g] \in {"Live", "Closing", "Detached", "Invalidating", "Retained",
                     "Invalidated", "Physical", "Abandoned"} =>
            slot = g /\ pages[g] = 2 /\ charges[g] = 2 /\ owner[g] # "None"

CleanupPrecedesDetach ==
    \A g \in Roots :
        phase[g] \in {"Detached", "Invalidating", "Retained", "Invalidated",
                     "Physical", "Released", "Partial"} => cleanup[g] = Stages

Invariants == TypeOK /\ LeasesRetainRoot /\ PhysicalRequiresInvalidation
    /\ PhysicalOnce /\ BackingCharged /\ TerminalAbandonment
    /\ CompleteOwnerRetained /\ CleanupPrecedesDetach

=============================================================================
