-------------------------- MODULE CharlotteThreadJoin --------------------------
\* Generation-bound EL0 thread handles and delayed exit-observer registration.

EXTENDS Naturals

CONSTANTS Tids, MaxGeneration

ASSUME MaxGeneration > 1

ThreadPhase == {"Absent", "Live", "Dead"}
HandlePhase == {"None", "Captured", "Observing", "Completed"}

VARIABLES phase, generation, handlePhase, handleGeneration, observerGeneration

vars == <<phase, generation, handlePhase, handleGeneration, observerGeneration>>

Init ==
    /\ phase = [t \in Tids |-> "Absent"]
    /\ generation = [t \in Tids |-> 0]
    /\ handlePhase = [t \in Tids |-> "None"]
    /\ handleGeneration = [t \in Tids |-> 0]
    /\ observerGeneration = [t \in Tids |-> 0]

Spawn(t) ==
    /\ phase[t] = "Absent"
    /\ generation[t] < MaxGeneration
    /\ phase' = [phase EXCEPT ![t] = "Live"]
    /\ generation' = [generation EXCEPT ![t] = @ + 1]
    /\ UNCHANGED <<handlePhase, handleGeneration, observerGeneration>>

CaptureHandle(t) ==
    /\ phase[t] = "Live"
    /\ handlePhase[t] = "None"
    /\ handlePhase' = [handlePhase EXCEPT ![t] = "Captured"]
    /\ handleGeneration' = [handleGeneration EXCEPT ![t] = generation[t]]
    /\ UNCHANGED <<phase, generation, observerGeneration>>

Exit(t) ==
    /\ phase[t] = "Live"
    /\ phase' = [phase EXCEPT ![t] = "Dead"]
    /\ UNCHANGED <<generation, handlePhase, handleGeneration,
                    observerGeneration>>

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
    /\ UNCHANGED <<phase, generation, handleGeneration>>

Reap(t) ==
    /\ phase[t] = "Dead"
    /\ phase' = [phase EXCEPT ![t] = "Absent"]
    /\ IF handlePhase[t] = "Observing"
          THEN handlePhase' = [handlePhase EXCEPT ![t] = "Completed"]
          ELSE UNCHANGED handlePhase
    /\ UNCHANGED <<generation, handleGeneration, observerGeneration>>

Next ==
    \/ \E t \in Tids : Spawn(t)
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
    /\ UNCHANGED <<phase, generation, handleGeneration>>

UnsafeNext == Next \/ \E t \in Tids : UnsafeObserveJoin(t)
UnsafeSpec == Init /\ [][UnsafeNext]_vars

TypeOK ==
    /\ phase \in [Tids -> ThreadPhase]
    /\ generation \in [Tids -> 0..MaxGeneration]
    /\ handlePhase \in [Tids -> HandlePhase]
    /\ handleGeneration \in [Tids -> 0..MaxGeneration]
    /\ observerGeneration \in [Tids -> 0..MaxGeneration]

CapturedGenerationExisted ==
    \A t \in Tids : handleGeneration[t] <= generation[t]

ObserverMatchesCapturedHandle ==
    \A t \in Tids :
        handlePhase[t] = "Observing" =>
            observerGeneration[t] = handleGeneration[t]

Invariants ==
    /\ TypeOK
    /\ CapturedGenerationExisted
    /\ ObserverMatchesCapturedHandle

=============================================================================
