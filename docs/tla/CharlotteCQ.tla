-------------------------------- MODULE CharlotteCQ ----------------------------------
\* TLA+ model of CharlotteOS Completion Queues.
\*
\* Covers: operation lifecycle (submit/in-flight/complete/cancel/observe),
\*          CQ rings with non-lossy backlog, CQ_WAIT/CQ_WAKE with race-free
\*          generation counter, timer submissions.
\*
\* Based on: docs/manual-v2/chapters/07-completion-queues.tex
\*           crates/catten/src/completion/

EXTENDS Naturals, Sequences, FiniteSets, Integers

\* -----------------------------------------------------------------------------
\* 1. CONSTANTS
\* -----------------------------------------------------------------------------

CONSTANTS
    ASID,           \* Address space IDs (e.g. {a1, a2})
    MaxOps,         \* Maximum operation IDs (e.g. 6)
    MaxCqs,         \* Maximum CQ IDs (e.g. 2)
    MaxRing,        \* CQ ring capacity in entries (e.g. 2)
    MaxTimers,      \* Maximum active timers (e.g. 2)
    NullAsid        \* Distinguished non-asid

ASSUME NullAsid \notin ASID
ASSUME MaxOps > 1 /\ MaxCqs > 0 /\ MaxRing > 0 /\ MaxTimers > 0

OpId      == 1 .. MaxOps
CqId      == 1 .. MaxCqs
TimerId   == 1 .. MaxTimers

\* -----------------------------------------------------------------------------
\* 2. TYPES
\* -----------------------------------------------------------------------------

\* Completion::OpState, plus Reclaimed for a removed capability slot.
OpState == {"InFlight", "CancelPending", "Completed", "Observed", "Reclaimed"}

\* A completion entry delivered to a CQ ring.
CompletionEntry == [
    operation : OpId,
    status    : {"Ok", "Failed", "Cancelled", "TimerFired"},
    result    : 0 .. 10,
    buffer    : 0 .. MaxOps    \* 0 = no buffer, otherwise buffer ID
]

\* An operation in the kernel.
Operation == [
    op       : OpId,
    owner    : ASID \cup {NullAsid},
    state    : OpState,
    cq       : CqId \cup {0},
    buffer   : 0 .. MaxOps,
    cqDrained : BOOLEAN
]

\* A CQ ring (shared-memory ring buffer between kernel and userspace).
CqRing == [
    owner    : ASID \cup {NullAsid},
    capacity : 1 .. MaxRing,
    entries  : Seq(CompletionEntry),
    backlog  : Seq(CompletionEntry),   \* kernel-side overflow buffer
    gen      : Nat,                    \* monotonic work-generation counter
    closed   : BOOLEAN
]

\* A timer waiting to fire.
Timer == [
    timer    : TimerId,
    owner    : ASID \cup {NullAsid},
    op       : OpId \cup {0},      \* which operation this timer is linked to
    fired    : BOOLEAN
]

\* CQ_WAIT waiter state (per-CQ generation tracking).
Waiter == [
    cq       : CqId,
    owner    : ASID \cup {NullAsid},
    lastSeen : Nat,                 \* last observed gen when waiting started
    waiting  : BOOLEAN
]

\* -----------------------------------------------------------------------------
\* 3. STATE VARIABLES
\* -----------------------------------------------------------------------------

VARIABLES
    operations,   \* [OpId -> Operation]
    cqRings,      \* [CqId -> CqRing]
    timers,       \* [TimerId -> Timer]
    waiters,      \* per-CQ waiter records (at most one per CQ)
    nextOpId, nextTimerId,
    kernelOwnsBuffer

vars == <<operations, cqRings, timers, waiters, nextOpId, nextTimerId,
          kernelOwnsBuffer>>

\* -----------------------------------------------------------------------------
\* 4. INITIAL STATE
\* -----------------------------------------------------------------------------

Init ==
    /\ operations = [o \in OpId |-> [
          op       |-> o,
          owner    |-> NullAsid,
          state    |-> "Reclaimed",
          cq       |-> 0,
          buffer   |-> 0,
          cqDrained |-> TRUE
       ]]
    /\ cqRings = [c \in CqId |-> [
          owner    |-> NullAsid,
          capacity |-> MaxRing,
          entries  |-> <<>>,
          backlog  |-> <<>>,
          gen      |-> 0,
          closed   |-> TRUE
       ]]
    /\ timers = [t \in TimerId |-> [
          timer    |-> t,
          owner    |-> NullAsid,
          op       |-> 0,
          fired    |-> TRUE
       ]]
    \* One waiter record per CQ (at most one thread waits at a time).
    /\ waiters = [c \in CqId |-> [
          cq       |-> c,
          owner    |-> NullAsid,
          lastSeen |-> 0,
          waiting  |-> FALSE
       ]]
    /\ nextOpId    = 1
    /\ nextTimerId = 1
    /\ kernelOwnsBuffer = [b \in OpId |-> FALSE]

\* -----------------------------------------------------------------------------
\* 5. HELPERS
\* -----------------------------------------------------------------------------

CanAllocOp    == nextOpId \in OpId
CanAllocTimer == nextTimerId \in TimerId

\* Post a completion to a CQ ring. If the ring is full, store in backlog.
\* Also wakes any waiter on this CQ.
PostCompletion(cqId, entry) ==
    LET ring == cqRings[cqId]
    IN /\ IF Len(ring.entries) < ring.capacity
          THEN \* Ring has space: append directly.
               cqRings' = [cqRings EXCEPT ![cqId].entries =
                                   Append(ring.entries, entry),
                                              ![cqId].gen = ring.gen + 1]
          ELSE \* Ring full: append to backlog.
               cqRings' = [cqRings EXCEPT ![cqId].backlog =
                                   Append(ring.backlog, entry),
                                              ![cqId].gen = ring.gen + 1]
       \* Wake any waiter on this CQ.
       /\ IF waiters[cqId].waiting
          THEN waiters' = [waiters EXCEPT ![cqId].waiting  = FALSE,
                                           ![cqId].lastSeen = ring.gen + 1]
          ELSE UNCHANGED waiters

\* -----------------------------------------------------------------------------
\* 6. TRANSITIONS
\* -----------------------------------------------------------------------------

\* -- 6.1 Submit an operation (no buffer). Rust creates it InFlight. ---------
SubmitNoBuffer(as, cqId) ==
    /\ CanAllocOp
    /\ cqRings[cqId].owner = as
    /\ ~cqRings[cqId].closed
    /\ LET opId == nextOpId
       IN /\ operations' = [operations EXCEPT ![opId] = [
                op |-> opId, owner |-> as, state |-> "InFlight",
                cq |-> cqId, buffer |-> 0, cqDrained |-> FALSE]]
          /\ nextOpId' = nextOpId + 1
    /\ UNCHANGED <<cqRings, timers, waiters, nextTimerId, kernelOwnsBuffer>>

\* -- 6.2 Submit an operation with a buffer -----------------------------------
SubmitWithBuffer(as, cqId, bufId) ==
    /\ CanAllocOp
    /\ cqRings[cqId].owner = as
    /\ ~cqRings[cqId].closed
    /\ bufId /= 0
    /\ ~kernelOwnsBuffer[bufId]
    /\ LET opId == nextOpId
       IN /\ operations' = [operations EXCEPT ![opId] = [
                op |-> opId, owner |-> as, state |-> "InFlight",
                cq |-> cqId, buffer |-> bufId, cqDrained |-> FALSE]]
          /\ nextOpId' = nextOpId + 1
    /\ kernelOwnsBuffer' = [kernelOwnsBuffer EXCEPT ![bufId] = TRUE]
    /\ UNCHANGED <<cqRings, timers, waiters, nextTimerId>>

\* -- 6.3 Operation completes normally ----------------------------------------
Complete(opId, resultVal) ==
    /\ operations[opId].state \in {"InFlight", "CancelPending"}
    /\ LET op  == operations[opId]
           cq  == op.cq
       IN /\ op.owner /= NullAsid
          /\ /\ operations' = [operations EXCEPT ![opId].state = "Completed"]
             /\ PostCompletion(cq, [operation |-> opId,
                    status |-> IF op.state = "CancelPending"
                               THEN "Cancelled" ELSE "Ok",
                    result |-> resultVal, buffer |-> op.buffer])
          /\ kernelOwnsBuffer' =
                IF op.buffer = 0 THEN kernelOwnsBuffer
                ELSE [kernelOwnsBuffer EXCEPT ![op.buffer] = FALSE]
    /\ UNCHANGED <<timers, nextOpId, nextTimerId>>

\* -- 6.4 Operation fails -----------------------------------------------------
Fail(opId, resultVal) ==
    /\ operations[opId].state \in {"InFlight", "CancelPending"}
    /\ LET op  == operations[opId]
           cq  == op.cq
       IN /\ op.owner /= NullAsid
          /\ /\ operations' = [operations EXCEPT ![opId].state = "Completed"]
             /\ PostCompletion(cq, [operation |-> opId,
                    status |-> IF op.state = "CancelPending"
                               THEN "Cancelled" ELSE "Failed",
                    result |-> resultVal, buffer |-> op.buffer])
          /\ kernelOwnsBuffer' =
                IF op.buffer = 0 THEN kernelOwnsBuffer
                ELSE [kernelOwnsBuffer EXCEPT ![op.buffer] = FALSE]
    /\ UNCHANGED <<timers, nextOpId, nextTimerId>>

\* -- 6.5 Request cancellation. Rust defers the terminal completion. ----------
CancelOp(opId) ==
    /\ operations[opId].state = "InFlight"
    /\ operations' = [operations EXCEPT ![opId].state = "CancelPending"]
    /\ UNCHANGED <<cqRings, timers, waiters, nextOpId, nextTimerId,
                   kernelOwnsBuffer>>

\* -- 6.7 Userspace drains one entry from a CQ ring ---------------------------
DrainOne(as, cqId) ==
    /\ cqRings[cqId].owner = as
    /\ cqRings[cqId].entries /= <<>>
    /\ ~waiters[cqId].waiting
    /\ LET ring == cqRings[cqId]
           hd  == Head(ring.entries)
           opId == hd.operation
           drained == Tail(ring.entries)
           refill == ring.backlog /= <<>> /\ Len(drained) < ring.capacity
           newEntries == IF refill
                         THEN Append(drained, Head(ring.backlog))
                         ELSE drained
           newBacklog == IF refill THEN Tail(ring.backlog) ELSE ring.backlog
       IN /\ cqRings' = [cqRings EXCEPT
                              ![cqId].entries = newEntries,
                              ![cqId].backlog = newBacklog,
                              ![cqId].gen = IF refill THEN ring.gen + 1
                                           ELSE ring.gen]
          /\ operations' = [operations EXCEPT ![opId].cqDrained = TRUE]
    /\ UNCHANGED <<timers, waiters, nextOpId, nextTimerId, kernelOwnsBuffer>>

\* -- 6.8 Userspace drains ALL entries from a CQ ring -------------------------
DrainAll(as, cqId) ==
    /\ cqRings[cqId].owner = as
    /\ cqRings[cqId].entries /= <<>>
    /\ ~waiters[cqId].waiting
    /\ LET ring == cqRings[cqId]
           \* Mark all entries in the ring as observed.
           obsOps == { ring.entries[i].operation : i \in 1 .. Len(ring.entries) }
           \* Move backlog entries into the ring (up to capacity).
           bl      == ring.backlog
           ringCap == ring.capacity
           filled  == IF Len(bl) <= ringCap
                      THEN bl
                      ELSE SubSeq(bl, 1, ringCap)
           remaining == IF Len(bl) <= ringCap
                       THEN <<>>
                       ELSE SubSeq(bl, ringCap + 1, Len(bl))
       IN /\ cqRings' = [cqRings EXCEPT ![cqId].entries  = filled,
                                          ![cqId].backlog  = remaining,
                                          ![cqId].gen      = ring.gen + 1]
          /\ operations' = [op \in OpId |->
               IF op \in obsOps
               THEN [operations[op] EXCEPT !.cqDrained = TRUE]
               ELSE operations[op]]
    /\ UNCHANGED <<timers, waiters, nextOpId, nextTimerId, kernelOwnsBuffer>>

\* -- 6.9 CQ_WAIT: block until a completion arrives or deadline -----------------
CqWait(as, cqId) ==
    /\ cqRings[cqId].owner = as
    /\ ~waiters[cqId].waiting
    /\ \* Check if work is already pending (ring non-empty or backlog non-empty).
       IF cqRings[cqId].entries /= <<>> \/ cqRings[cqId].backlog /= <<>>
       THEN \* Work already available: don't block, just update gen.
            /\ waiters' = [waiters EXCEPT ![cqId].lastSeen = cqRings[cqId].gen]
            /\ UNCHANGED <<operations, cqRings, timers, nextOpId, nextTimerId,
                            kernelOwnsBuffer>>
       ELSE \* No work: register waiter and block.
            /\ waiters' = [waiters EXCEPT ![cqId].owner    = as,
                                           ![cqId].lastSeen = cqRings[cqId].gen,
                                           ![cqId].waiting  = TRUE]
            /\ UNCHANGED <<operations, cqRings, timers, nextOpId, nextTimerId,
                            kernelOwnsBuffer>>

\* -- 6.10 CQ_WAKE: unblock a waiter (called when new completion arrives) ----
CqWake(cqId) ==
    /\ waiters[cqId].waiting
    /\ waiters' = [waiters EXCEPT ![cqId].waiting  = FALSE,
                                   ![cqId].lastSeen = cqRings[cqId].gen]
    /\ UNCHANGED <<operations, cqRings, timers, nextOpId, nextTimerId,
                   kernelOwnsBuffer>>

\* -- 6.11 Submit a timer ------------------------------------------------------
SubmitTimer(as, cqId, timeout) ==
    /\ CanAllocOp /\ CanAllocTimer
    /\ cqRings[cqId].owner = as
    /\ LET opId    == nextOpId
           timerId == nextTimerId
       IN /\ operations' = [operations EXCEPT ![opId] = [
                op |-> opId, owner |-> as, state |-> "InFlight",
                cq |-> cqId, buffer |-> 0, cqDrained |-> FALSE]]
          /\ timers' = [timers EXCEPT ![timerId] = [
                timer |-> timerId, owner |-> as, op |-> opId, fired |-> FALSE]]
          /\ nextOpId'    = nextOpId + 1
          /\ nextTimerId' = nextTimerId + 1
    /\ UNCHANGED <<cqRings, waiters, kernelOwnsBuffer>>

\* -- 6.12 Timer fires ---------------------------------------------------------
TimerFire(timerId) ==
    /\ ~timers[timerId].fired
    /\ LET opId == timers[timerId].op
           op   == operations[opId]
       IN /\ op.state \in {"InFlight", "CancelPending"}
          /\ timers' = [timers EXCEPT ![timerId].fired = TRUE]
          /\ operations' = [operations EXCEPT ![opId].state = "Completed"]
          /\ PostCompletion(op.cq, [operation |-> opId,
                                    status |-> IF op.state = "CancelPending"
                                               THEN "Cancelled" ELSE "TimerFired",
                                    result |-> 0, buffer |-> 0])
    /\ UNCHANGED <<nextOpId, nextTimerId, kernelOwnsBuffer>>

\* -- 6.14 Open a CQ (assign owner) --------------------------------------------
OpenCq(as, cqId) ==
    /\ cqRings[cqId].closed
    /\ cqRings[cqId].owner = NullAsid
    /\ cqRings' = [cqRings EXCEPT ![cqId].owner  = as,
                                   ![cqId].closed = FALSE,
                                   ![cqId].gen    = cqRings[cqId].gen + 1]
    /\ UNCHANGED <<operations, timers, waiters, nextOpId, nextTimerId,
                   kernelOwnsBuffer>>

\* -- 6.14 Observe a capability result through poll/take ----------------------
ObserveResult(opId) ==
    /\ operations[opId].state = "Completed"
    /\ operations' = [operations EXCEPT ![opId].state = "Observed"]
    /\ UNCHANGED <<cqRings, timers, waiters, nextOpId, nextTimerId,
                   kernelOwnsBuffer>>

\* -- 6.15 Revoke a terminal completion capability ----------------------------
Reclaim(opId) ==
    /\ operations[opId].state \in {"Completed", "Observed"}
    /\ operations' = [operations EXCEPT ![opId].state = "Reclaimed"]
    /\ UNCHANGED <<cqRings, timers, waiters, nextOpId, nextTimerId,
                   kernelOwnsBuffer>>

\* -----------------------------------------------------------------------------
\* 7. NEXT-STATE RELATION
\* -----------------------------------------------------------------------------

Next ==
    \/ \E as \in ASID : \E cqId \in CqId : OpenCq(as, cqId)
    \/ \E as \in ASID : \E cqId \in CqId :
            SubmitNoBuffer(as, cqId)
    \/ \E as \in ASID : \E cqId \in CqId : \E buf \in (1 .. MaxOps) :
            SubmitWithBuffer(as, cqId, buf)
    \/ \E opId \in OpId : \E r \in 0 .. 3 : Complete(opId, r)
    \/ \E opId \in OpId : \E r \in 0 .. 3 : Fail(opId, r)
    \/ \E opId \in OpId : CancelOp(opId)
    \/ \E as \in ASID : \E cqId \in CqId : DrainOne(as, cqId)
    \/ \E as \in ASID : \E cqId \in CqId : DrainAll(as, cqId)
    \/ \E as \in ASID : \E cqId \in CqId : CqWait(as, cqId)
    \/ \E cqId \in CqId : CqWake(cqId)
    \/ \E as \in ASID : \E cqId \in CqId : \E t \in 0 .. 3 :
            SubmitTimer(as, cqId, t)
    \/ \E timerId \in TimerId : TimerFire(timerId)
    \/ \E opId \in OpId : ObserveResult(opId)
    \/ \E opId \in OpId : Reclaim(opId)

Spec == Init /\ [][Next]_vars

\* Regression action: treating cancellation as terminal releases the buffer
\* while the kernel operation may still complete against it.
UnsafeCancelReleasesBuffer(opId) ==
    /\ operations[opId].state = "InFlight"
    /\ operations[opId].buffer # 0
    /\ operations' = [operations EXCEPT ![opId].state = "CancelPending"]
    /\ kernelOwnsBuffer' =
        [kernelOwnsBuffer EXCEPT ![operations[opId].buffer] = FALSE]
    /\ UNCHANGED <<cqRings, timers, waiters, nextOpId, nextTimerId>>

UnsafeNext == Next \/ \E opId \in OpId : UnsafeCancelReleasesBuffer(opId)
UnsafeSpec == Init /\ [][UnsafeNext]_vars

\* State constraint: bound the generation counter to prevent state explosion.
MaxGen == 5
StateConstraint == \A c \in CqId : cqRings[c].gen <= MaxGen

\* -----------------------------------------------------------------------------
\* 8. INVARIANTS
\* -----------------------------------------------------------------------------

\* I0: Every state variable remains within its declared abstract type.
TypeOK ==
    /\ operations \in [OpId -> Operation]
    /\ cqRings \in [CqId -> CqRing]
    /\ timers \in [TimerId -> Timer]
    /\ waiters \in [CqId -> Waiter]
    /\ nextOpId \in 1 .. (MaxOps + 1)
    /\ nextTimerId \in 1 .. (MaxTimers + 1)
    /\ kernelOwnsBuffer \in [OpId -> BOOLEAN]

\* I1: Non-lossy CQ delivery. Until userspace drains an operation's CQ entry,
\*     it remains in the shared ring or the kernel backlog. Polling the
\*     completion capability is independent and may set state=Observed first.
CompletionIsTracked ==
    \A opId \in OpId :
        LET op == operations[opId]
        IN op.state \in {"Completed", "Observed"} =>
           (op.cqDrained
            \/ \E c \in CqId :
                 (\E i \in 1 .. Len(cqRings[c].entries) :
                      cqRings[c].entries[i].operation = opId)
                 \/ (\E i \in 1 .. Len(cqRings[c].backlog) :
                      cqRings[c].backlog[i].operation = opId))

\* I2: Ring boundedness. Ring entries never exceed capacity.
RingBounded ==
    \A c \in CqId :
        Len(cqRings[c].entries) <= cqRings[c].capacity

\* I4: Waiter invariants. A waiting waiter has owner \= NullAsid.
WaiterValid ==
    \A c \in CqId :
        waiters[c].waiting => waiters[c].owner /= NullAsid

\* I5: No lost wakeups. If a waiter is registered and a completion is posted,
\*     either the ring is non-empty (waiter should wake and drain) or the
\*     backlog is non-empty.
\*     (Waiter checking before blocking ensures this.)
NoLostWakeups ==
    \A c \in CqId :
        waiters[c].waiting =>
            (waiters[c].lastSeen = cqRings[c].gen)
            \/ (cqRings[c].entries /= <<>>)
            \/ (cqRings[c].backlog /= <<>>)

\* I6: CQ ring owner is valid. Active CQs have valid owners.
CqOwnerValid ==
    \A c \in CqId :
        (~cqRings[c].closed) => cqRings[c].owner /= NullAsid

\* I7: Operation references the correct CQ.
OpCqValid ==
    \A opId \in OpId :
        operations[opId].state /= "Reclaimed" =>
            operations[opId].cq /= 0

\* I8: Timer fired implies operation completed (or later reclaimed).
TimerFiredImpliesCompleted ==
    \A t \in TimerId :
        timers[t].fired /\ timers[t].op /= 0 =>
            operations[timers[t].op].state \in {"Completed", "Observed", "Reclaimed"}

NonTerminalBufferRemainsLoaned ==
    \A opId \in OpId :
        (operations[opId].state \in {"InFlight", "CancelPending"}
            /\ operations[opId].buffer # 0) =>
            kernelOwnsBuffer[operations[opId].buffer]

KernelBufferHasOneLiveOperation ==
    \A buf \in OpId :
        kernelOwnsBuffer[buf] =>
            \E opId \in OpId :
                /\ operations[opId].buffer = buf
                /\ operations[opId].state \in {"InFlight", "CancelPending"}

Invariants ==
    /\ TypeOK
    /\ CompletionIsTracked
    /\ RingBounded
    /\ WaiterValid
    /\ NoLostWakeups
    /\ CqOwnerValid
    /\ OpCqValid
    /\ TimerFiredImpliesCompleted
    /\ NonTerminalBufferRemainsLoaned
    /\ KernelBufferHasOneLiveOperation

\* Convert sequence to set (helper for invariants).
SeqToSet(seq) == { seq[i] : i \in 1 .. Len(seq) }

=============================================================================
