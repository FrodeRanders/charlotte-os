---------------------- MODULE CharlotteInterruptRoute ----------------------
\* Deferred device-interrupt wakes across driver route teardown and reuse.

EXTENDS Naturals

CONSTANT MaxGeneration

ASSUME MaxGeneration > 3

VARIABLES bound, routeGeneration, queuedGeneration, misdelivered

vars == <<bound, routeGeneration, queuedGeneration, misdelivered>>

Init ==
    /\ bound = FALSE
    /\ routeGeneration = 0
    /\ queuedGeneration = 0
    /\ misdelivered = FALSE

Bind ==
    /\ ~bound
    /\ routeGeneration < MaxGeneration
    /\ bound' = TRUE
    /\ routeGeneration' = routeGeneration + 1
    /\ UNCHANGED <<queuedGeneration, misdelivered>>

QueueWake ==
    /\ bound
    /\ queuedGeneration = 0
    /\ queuedGeneration' = routeGeneration
    /\ UNCHANGED <<bound, routeGeneration, misdelivered>>

Unbind ==
    /\ bound
    /\ routeGeneration < MaxGeneration
    /\ bound' = FALSE
    /\ routeGeneration' = routeGeneration + 1
    /\ UNCHANGED <<queuedGeneration, misdelivered>>

\* Generation-aware drain: a wake for a retired route is discarded.
DrainSafe ==
    /\ queuedGeneration > 0
    /\ queuedGeneration' = 0
    /\ UNCHANGED <<bound, routeGeneration, misdelivered>>

\* The bug: the consumer sees only the rebound numeric destination and
\* mistakes an old queued wake for an event on the replacement route.
DrainUnsafe ==
    /\ queuedGeneration > 0
    /\ bound
    /\ misdelivered' =
          (misdelivered \/ (queuedGeneration # routeGeneration))
    /\ queuedGeneration' = 0
    /\ UNCHANGED <<bound, routeGeneration>>

SafeNext == Bind \/ QueueWake \/ Unbind \/ DrainSafe
UnsafeNext == Bind \/ QueueWake \/ Unbind \/ DrainUnsafe

Spec == Init /\ [][SafeNext]_vars
UnsafeSpec == Init /\ [][UnsafeNext]_vars

TypeOK ==
    /\ bound \in BOOLEAN
    /\ routeGeneration \in 0..MaxGeneration
    /\ queuedGeneration \in 0..MaxGeneration
    /\ misdelivered \in BOOLEAN

NoStaleWakeDelivery == ~misdelivered

Invariants == TypeOK /\ NoStaleWakeDelivery

=============================================================================
