------------------ MODULE CharlotteEndpointObservers ------------------
\* Endpoint readiness and endpoint closure are distinct observable events.
\*
\* The repaired kernel stores their observers in separate queues. The
\* UnsafeMessageWake action models the former shared queue: message arrival
\* incorrectly completes a close watch while the endpoint remains open.

EXTENDS Naturals

CONSTANT UnsafeSharedObservers

VARIABLES messageQueued,
          closed,
          readinessArmed,
          closeArmed,
          readinessSignaled,
          closeSignaled

vars == <<messageQueued, closed, readinessArmed, closeArmed,
          readinessSignaled, closeSignaled>>

Init ==
    /\ messageQueued = FALSE
    /\ closed = FALSE
    /\ readinessArmed = FALSE
    /\ closeArmed = FALSE
    /\ readinessSignaled = FALSE
    /\ closeSignaled = FALSE

ArmReadiness ==
    /\ ~closed
    /\ ~readinessArmed
    /\ readinessArmed' = TRUE
    /\ UNCHANGED <<messageQueued, closed, closeArmed,
                    readinessSignaled, closeSignaled>>

ArmCloseWatch ==
    /\ ~closed
    /\ ~closeArmed
    /\ closeArmed' = TRUE
    /\ UNCHANGED <<messageQueued, closed, readinessArmed,
                    readinessSignaled, closeSignaled>>

Send ==
    /\ ~UnsafeSharedObservers
    /\ ~closed
    /\ ~messageQueued
    /\ messageQueued' = TRUE
    /\ readinessSignaled' = (readinessSignaled \/ readinessArmed)
    /\ readinessArmed' = FALSE
    /\ UNCHANGED <<closed, closeArmed, closeSignaled>>

\* Former implementation: enqueue drained one observer queue containing both
\* readiness waiters and endpoint-close completions.
UnsafeMessageWake ==
    /\ UnsafeSharedObservers
    /\ ~closed
    /\ ~messageQueued
    /\ messageQueued' = TRUE
    /\ readinessSignaled' = (readinessSignaled \/ readinessArmed)
    /\ closeSignaled' = (closeSignaled \/ closeArmed)
    /\ readinessArmed' = FALSE
    /\ closeArmed' = FALSE
    /\ UNCHANGED closed

Receive ==
    /\ messageQueued
    /\ messageQueued' = FALSE
    /\ readinessSignaled' = FALSE
    /\ UNCHANGED <<closed, readinessArmed, closeArmed, closeSignaled>>

Close ==
    /\ ~closed
    /\ closed' = TRUE
    /\ readinessSignaled' = (readinessSignaled \/ readinessArmed)
    /\ closeSignaled' = (closeSignaled \/ closeArmed)
    /\ readinessArmed' = FALSE
    /\ closeArmed' = FALSE
    /\ UNCHANGED messageQueued

ObserveClose ==
    /\ closeSignaled
    /\ closeSignaled' = FALSE
    /\ UNCHANGED <<messageQueued, closed, readinessArmed, closeArmed,
                    readinessSignaled>>

Next ==
    \/ ArmReadiness
    \/ ArmCloseWatch
    \/ Send
    \/ UnsafeMessageWake
    \/ Receive
    \/ Close
    \/ ObserveClose

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ messageQueued \in BOOLEAN
    /\ closed \in BOOLEAN
    /\ readinessArmed \in BOOLEAN
    /\ closeArmed \in BOOLEAN
    /\ readinessSignaled \in BOOLEAN
    /\ closeSignaled \in BOOLEAN

\* A close completion is authoritative evidence that the endpoint is closed.
CloseSignalImpliesClosed == closeSignaled => closed

Invariants == TypeOK /\ CloseSignalImpliesClosed

=======================================================================
