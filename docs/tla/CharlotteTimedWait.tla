--------------------------- MODULE CharlotteTimedWait ---------------------------
\* Race between a completion generation change, wake delivery, and a timeout.

EXTENDS Naturals

CONSTANT MaxGeneration

ASSUME MaxGeneration > 1

WaitPhase == {"Idle", "Waiting", "Ready"}
WaitResult == {"None", "Work", "Timeout"}

VARIABLES phase, generation, registeredGeneration, result, wakePending

vars == <<phase, generation, registeredGeneration, result, wakePending>>

Init ==
    /\ phase = "Idle"
    /\ generation = 0
    /\ registeredGeneration = 0
    /\ result = "None"
    /\ wakePending = FALSE

ArmWait ==
    /\ phase = "Idle"
    /\ phase' = "Waiting"
    /\ registeredGeneration' = generation
    /\ result' = "None"
    /\ wakePending' = FALSE
    /\ UNCHANGED generation

\* Split publication from wake delivery to expose the timer race window.
PublishWork ==
    /\ phase = "Waiting"
    /\ generation < MaxGeneration
    /\ generation' = generation + 1
    /\ wakePending' = TRUE
    /\ UNCHANGED <<phase, registeredGeneration, result>>

DeliverWake ==
    /\ phase = "Waiting"
    /\ wakePending
    /\ generation > registeredGeneration
    /\ phase' = "Ready"
    /\ result' = "Work"
    /\ wakePending' = FALSE
    /\ UNCHANGED <<generation, registeredGeneration>>

\* The timer may win scheduling after work publication but before its wake.
\* Rechecking the generation makes that return report Work, not Timeout.
TimerFire ==
    /\ phase = "Waiting"
    /\ phase' = "Ready"
    /\ result' = IF generation = registeredGeneration THEN "Timeout" ELSE "Work"
    /\ wakePending' = FALSE
    /\ UNCHANGED <<generation, registeredGeneration>>

Consume ==
    /\ phase = "Ready"
    /\ phase' = "Idle"
    /\ result' = "None"
    /\ registeredGeneration' = generation
    /\ wakePending' = FALSE
    /\ UNCHANGED generation

Next == ArmWait \/ PublishWork \/ DeliverWake \/ TimerFire \/ Consume
Spec == Init /\ [][Next]_vars

\* Regression: a timer return is accepted without the post-registration
\* generation recheck, misreporting a concurrent publication as a timeout.
UnsafeTimerFire ==
    /\ phase = "Waiting"
    /\ phase' = "Ready"
    /\ result' = "Timeout"
    /\ wakePending' = FALSE
    /\ UNCHANGED <<generation, registeredGeneration>>

UnsafeNext == Next \/ UnsafeTimerFire
UnsafeSpec == Init /\ [][UnsafeNext]_vars

TypeOK ==
    /\ phase \in WaitPhase
    /\ generation \in 0..MaxGeneration
    /\ registeredGeneration \in 0..MaxGeneration
    /\ result \in WaitResult
    /\ wakePending \in BOOLEAN

TimeoutObservedNoWork ==
    result = "Timeout" => generation = registeredGeneration

Invariants == TypeOK /\ TimeoutObservedNoWork

=============================================================================
