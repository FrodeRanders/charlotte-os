----------------------- MODULE CharlotteAddressSpace -----------------------
\* Reusable numeric ASID slots and generation-fenced lifecycle authority.

EXTENDS Naturals

CONSTANTS Slots, MaxGeneration

ASSUME MaxGeneration > 1

VARIABLES alive, generation, staleGeneration, protectedGeneration

vars == <<alive, generation, staleGeneration, protectedGeneration>>

Init ==
    /\ alive = [s \in Slots |-> FALSE]
    /\ generation = [s \in Slots |-> 0]
    /\ staleGeneration = [s \in Slots |-> 0]
    /\ protectedGeneration = [s \in Slots |-> 0]

Allocate(s) ==
    /\ ~alive[s]
    /\ generation[s] < MaxGeneration
    /\ alive' = [alive EXCEPT ![s] = TRUE]
    /\ generation' = [generation EXCEPT ![s] = @ + 1]
    /\ protectedGeneration' =
          IF staleGeneration[s] > 0
          THEN [protectedGeneration EXCEPT ![s] = generation[s] + 1]
          ELSE protectedGeneration
    /\ UNCHANGED staleGeneration

CaptureHandle(s) ==
    /\ alive[s]
    /\ staleGeneration[s] = 0
    /\ staleGeneration' = [staleGeneration EXCEPT ![s] = generation[s]]
    /\ UNCHANGED <<alive, generation, protectedGeneration>>

CloseExact(s) ==
    /\ alive[s]
    /\ staleGeneration[s] = generation[s]
    /\ alive' = [alive EXCEPT ![s] = FALSE]
    /\ UNCHANGED <<generation, staleGeneration, protectedGeneration>>

\* The bug: a delayed lifecycle operation checks only the reusable number.
UnsafeStaleClose(s) ==
    /\ alive[s]
    /\ staleGeneration[s] > 0
    /\ staleGeneration[s] # generation[s]
    /\ alive' = [alive EXCEPT ![s] = FALSE]
    /\ UNCHANGED <<generation, staleGeneration, protectedGeneration>>

SafeNext ==
    \/ \E s \in Slots : Allocate(s)
    \/ \E s \in Slots : CaptureHandle(s)
    \/ \E s \in Slots : CloseExact(s)

UnsafeNext == SafeNext \/ \E s \in Slots : UnsafeStaleClose(s)

Spec == Init /\ [][SafeNext]_vars
UnsafeSpec == Init /\ [][UnsafeNext]_vars

TypeOK ==
    /\ alive \in [Slots -> BOOLEAN]
    /\ generation \in [Slots -> 0..MaxGeneration]
    /\ staleGeneration \in [Slots -> 0..MaxGeneration]
    /\ protectedGeneration \in [Slots -> 0..MaxGeneration]

ReplacementSurvivesStaleHandle ==
    \A s \in Slots :
        protectedGeneration[s] > 0 =>
            alive[s] /\ generation[s] = protectedGeneration[s]

Invariants == TypeOK /\ ReplacementSurvivesStaleHandle

=============================================================================
