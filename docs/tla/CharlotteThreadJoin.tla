-------------------------- MODULE CharlotteThreadJoin --------------------------
\* Generation-bound EL0 thread handles and delayed exit-observer registration.

EXTENDS Naturals, FiniteSets

CONSTANTS Tids, MaxGeneration

ASSUME MaxGeneration > 1

ThreadPhase == {"Absent", "Live", "Dead"}
HandlePhase == {"None", "Captured", "Observing", "Completed"}

VARIABLES phase, generation, handlePhase, handleGeneration, observerGeneration,
          nextGeneration, allocatedGenerations, allocationCount, spawnRejected

vars == <<phase, generation, handlePhase, handleGeneration, observerGeneration,
          nextGeneration, allocatedGenerations, allocationCount, spawnRejected>>

Init ==
    /\ phase = [t \in Tids |-> "Absent"]
    /\ generation = [t \in Tids |-> 0]
    /\ handlePhase = [t \in Tids |-> "None"]
    /\ handleGeneration = [t \in Tids |-> 0]
    /\ observerGeneration = [t \in Tids |-> 0]
    /\ nextGeneration = 1
    /\ allocatedGenerations = {}
    /\ allocationCount = 0
    /\ spawnRejected = [t \in Tids |-> FALSE]

Spawn(t) ==
    /\ phase[t] = "Absent"
    /\ nextGeneration < MaxGeneration
    /\ phase' = [phase EXCEPT ![t] = "Live"]
    /\ generation' = [generation EXCEPT ![t] = nextGeneration]
    /\ nextGeneration' = nextGeneration + 1
    /\ allocatedGenerations' = allocatedGenerations \union {nextGeneration}
    /\ allocationCount' = allocationCount + 1
    /\ spawnRejected' = [spawnRejected EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<handlePhase, handleGeneration, observerGeneration>>

\* The implementation refuses allocation before the non-zero generation
\* namespace could wrap and alias an earlier thread handle.
RejectExhaustedSpawn(t) ==
    /\ phase[t] = "Absent"
    /\ nextGeneration = MaxGeneration
    /\ ~spawnRejected[t]
    /\ spawnRejected' = [spawnRejected EXCEPT ![t] = TRUE]
    /\ UNCHANGED <<phase, generation, handlePhase, handleGeneration,
                    observerGeneration, nextGeneration, allocatedGenerations,
                    allocationCount>>

CaptureHandle(t) ==
    /\ phase[t] = "Live"
    /\ handlePhase[t] = "None"
    /\ handlePhase' = [handlePhase EXCEPT ![t] = "Captured"]
    /\ handleGeneration' = [handleGeneration EXCEPT ![t] = generation[t]]
    /\ UNCHANGED <<phase, generation, observerGeneration,
                    nextGeneration, allocatedGenerations, allocationCount,
                    spawnRejected>>

Exit(t) ==
    /\ phase[t] = "Live"
    /\ phase' = [phase EXCEPT ![t] = "Dead"]
    /\ UNCHANGED <<generation, handlePhase, handleGeneration,
                    observerGeneration, nextGeneration, allocatedGenerations,
                    allocationCount, spawnRejected>>

\* A matching live generation installs an observer. If the thread is already
\* gone or the slot contains a replacement, the completion is terminal now.
ObserveJoin(t) ==
    /\ handlePhase[t] = "Captured"
    /\ IF phase[t] = "Live" /\ generation[t] = handleGeneration[t]
          THEN /\ handlePhase' = [handlePhase EXCEPT ![t] = "Observing"]
               /\ observerGeneration' =
                    [observerGeneration EXCEPT ![t] = handleGeneration[t]]
          ELSE /\ handlePhase' = [handlePhase EXCEPT ![t] = "Completed"]
               /\ UNCHANGED observerGeneration
    /\ UNCHANGED <<phase, generation, handleGeneration,
                    nextGeneration, allocatedGenerations, allocationCount,
                    spawnRejected>>

Reap(t) ==
    /\ phase[t] = "Dead"
    /\ phase' = [phase EXCEPT ![t] = "Absent"]
    /\ IF handlePhase[t] = "Observing"
          THEN handlePhase' = [handlePhase EXCEPT ![t] = "Completed"]
          ELSE UNCHANGED handlePhase
    /\ UNCHANGED <<generation, handleGeneration, observerGeneration,
                    nextGeneration, allocatedGenerations, allocationCount,
                    spawnRejected>>

Next ==
    \/ \E t \in Tids : Spawn(t)
    \/ \E t \in Tids : RejectExhaustedSpawn(t)
    \/ \E t \in Tids : CaptureHandle(t)
    \/ \E t \in Tids : Exit(t)
    \/ \E t \in Tids : ObserveJoin(t)
    \/ \E t \in Tids : Reap(t)

Spec == Init /\ [][Next]_vars

\* Former TID-only observation: attach the old handle to whichever generation
\* currently occupies the numeric slot.
UnsafeObserveJoin(t) ==
    /\ handlePhase[t] = "Captured"
    /\ phase[t] = "Live"
    /\ handlePhase' = [handlePhase EXCEPT ![t] = "Observing"]
    /\ observerGeneration' = [observerGeneration EXCEPT ![t] = generation[t]]
    /\ UNCHANGED <<phase, generation, handleGeneration,
                    nextGeneration, allocatedGenerations, allocationCount,
                    spawnRejected>>

\* Regression: wrapping to generation one reuses an identity that a delayed
\* handle may still carry.
UnsafeSpawnWrap(t) ==
    /\ phase[t] = "Absent"
    /\ nextGeneration = MaxGeneration
    /\ phase' = [phase EXCEPT ![t] = "Live"]
    /\ generation' = [generation EXCEPT ![t] = 1]
    /\ nextGeneration' = 2
    /\ allocatedGenerations' = allocatedGenerations \union {1}
    /\ allocationCount' = allocationCount + 1
    /\ spawnRejected' = [spawnRejected EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<handlePhase, handleGeneration, observerGeneration>>

UnsafeNext ==
    \/ Next
    \/ \E t \in Tids : UnsafeObserveJoin(t)
    \/ \E t \in Tids : UnsafeSpawnWrap(t)
UnsafeSpec == Init /\ [][UnsafeNext]_vars

TypeOK ==
    /\ phase \in [Tids -> ThreadPhase]
    /\ generation \in [Tids -> 0..(MaxGeneration - 1)]
    /\ handlePhase \in [Tids -> HandlePhase]
    /\ handleGeneration \in [Tids -> 0..(MaxGeneration - 1)]
    /\ observerGeneration \in [Tids -> 0..(MaxGeneration - 1)]
    /\ nextGeneration \in 1..MaxGeneration
    /\ allocatedGenerations \subseteq 1..(MaxGeneration - 1)
    /\ allocationCount \in 0..MaxGeneration
    /\ spawnRejected \in [Tids -> BOOLEAN]

CapturedGenerationExisted ==
    \A t \in Tids : handleGeneration[t] <= generation[t]

ObserverMatchesCapturedHandle ==
    \A t \in Tids :
        handlePhase[t] = "Observing" =>
            observerGeneration[t] = handleGeneration[t]

GenerationNeverReused ==
    allocationCount = Cardinality(allocatedGenerations)

ExhaustionFailsClosed ==
    \A t \in Tids :
        spawnRejected[t] => phase[t] = "Absent" /\ nextGeneration = MaxGeneration

Invariants ==
    /\ TypeOK
    /\ CapturedGenerationExisted
    /\ ObserverMatchesCapturedHandle
    /\ GenerationNeverReused
    /\ ExhaustionFailsClosed

=============================================================================
