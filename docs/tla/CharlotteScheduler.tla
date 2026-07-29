-------------------------- MODULE CharlotteScheduler --------------------------
\* Finite safety model of CharlotteOS thread admission, execution, blocking,
\* migration, abort, per-LP dead staging, reaping, and generation-safe wakeup.

EXTENDS Naturals, FiniteSets

CONSTANTS Threads, LPs, ASID, MaxGeneration, NullAsid, NullLp

ASSUME NullAsid \notin ASID
ASSUME NullLp \notin LPs
ASSUME MaxGeneration > 1

ThreadPhase == {"Absent", "NeedsLp", "Ready", "Running", "Blocked", "Dead"}

VARIABLES phase, generation, owner, location, affinity, pinned,
          migrationSafe, waiterGeneration

vars == <<phase, generation, owner, location, affinity, pinned,
          migrationSafe, waiterGeneration>>

Init ==
    /\ phase = [t \in Threads |-> "Absent"]
    /\ generation = [t \in Threads |-> 0]
    /\ owner = [t \in Threads |-> NullAsid]
    /\ location = [t \in Threads |-> NullLp]
    /\ affinity = [t \in Threads |-> NullLp]
    /\ pinned = [t \in Threads |-> NullLp]
    /\ migrationSafe = [t \in Threads |-> FALSE]
    /\ waiterGeneration = [t \in Threads |-> 0]

LpIdle(lp) == ~(\E t \in Threads : phase[t] = "Running" /\ location[t] = lp)

Spawn(t, as, canMigrate, pin) ==
    /\ phase[t] = "Absent"
    /\ generation[t] < MaxGeneration
    /\ pin \in LPs \cup {NullLp}
    /\ phase' = [phase EXCEPT ![t] = "NeedsLp"]
    /\ generation' = [generation EXCEPT ![t] = @ + 1]
    /\ owner' = [owner EXCEPT ![t] = as]
    /\ location' = [location EXCEPT ![t] = NullLp]
    /\ affinity' = [affinity EXCEPT ![t] = pin]
    /\ pinned' = [pinned EXCEPT ![t] = pin]
    /\ migrationSafe' = [migrationSafe EXCEPT ![t] = canMigrate /\ pin = NullLp]
    /\ UNCHANGED waiterGeneration

Admit(t, lp) ==
    /\ phase[t] = "NeedsLp"
    /\ pinned[t] \in {NullLp, lp}
    /\ phase' = [phase EXCEPT ![t] = "Ready"]
    /\ location' = [location EXCEPT ![t] = lp]
    /\ affinity' = [affinity EXCEPT ![t] = lp]
    /\ UNCHANGED <<generation, owner, pinned, migrationSafe, waiterGeneration>>

Dispatch(t) ==
    /\ phase[t] = "Ready"
    /\ LpIdle(location[t])
    /\ phase' = [phase EXCEPT ![t] = "Running"]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration>>

Preempt(t) ==
    /\ phase[t] = "Running"
    /\ phase' = [phase EXCEPT ![t] = "Ready"]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration>>

Block(t) ==
    /\ phase[t] \in {"Running", "Ready"}
    /\ phase' = [phase EXCEPT ![t] = "Blocked"]
    /\ waiterGeneration' = [waiterGeneration EXCEPT ![t] = generation[t]]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned, migrationSafe>>

Wake(t, observedGeneration) ==
    /\ phase[t] = "Blocked"
    /\ observedGeneration = generation[t]
    /\ observedGeneration = waiterGeneration[t]
    /\ phase' = [phase EXCEPT ![t] = "Ready"]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration>>

Migrate(t, destination) ==
    /\ phase[t] = "Ready"
    /\ migrationSafe[t]
    /\ pinned[t] = NullLp
    /\ destination /= location[t]
    /\ location' = [location EXCEPT ![t] = destination]
    /\ affinity' = [affinity EXCEPT ![t] = destination]
    /\ UNCHANGED <<phase, generation, owner, pinned,
                   migrationSafe, waiterGeneration>>

\* abort_thread removes the master-table entry before staging the owned Thread
\* on the LP-local dead list. "Dead" is that staged-but-not-dropped interval.
Abort(t) ==
    /\ phase[t] \in {"NeedsLp", "Ready", "Running", "Blocked"}
    /\ phase' = [phase EXCEPT ![t] = "Dead"]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration>>

Reap(t) ==
    /\ phase[t] = "Dead"
    /\ phase' = [phase EXCEPT ![t] = "Absent"]
    /\ owner' = [owner EXCEPT ![t] = NullAsid]
    /\ location' = [location EXCEPT ![t] = NullLp]
    /\ affinity' = [affinity EXCEPT ![t] = NullLp]
    /\ pinned' = [pinned EXCEPT ![t] = NullLp]
    /\ migrationSafe' = [migrationSafe EXCEPT ![t] = FALSE]
    \* Retain waiterGeneration: a stale Waker may outlive slot reuse.
    /\ UNCHANGED <<generation, waiterGeneration>>

Next ==
    \/ \E t \in Threads, as \in ASID, movable \in BOOLEAN,
          pin \in LPs \cup {NullLp} : Spawn(t, as, movable, pin)
    \/ \E t \in Threads, lp \in LPs : Admit(t, lp)
    \/ \E t \in Threads : Dispatch(t)
    \/ \E t \in Threads : Preempt(t)
    \/ \E t \in Threads : Block(t)
    \/ \E t \in Threads, gen \in 0..MaxGeneration : Wake(t, gen)
    \/ \E t \in Threads, lp \in LPs : Migrate(t, lp)
    \/ \E t \in Threads : Abort(t)
    \/ \E t \in Threads : Reap(t)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ phase \in [Threads -> ThreadPhase]
    /\ generation \in [Threads -> 0..MaxGeneration]
    /\ owner \in [Threads -> ASID \cup {NullAsid}]
    /\ location \in [Threads -> LPs \cup {NullLp}]
    /\ affinity \in [Threads -> LPs \cup {NullLp}]
    /\ pinned \in [Threads -> LPs \cup {NullLp}]
    /\ migrationSafe \in [Threads -> BOOLEAN]
    /\ waiterGeneration \in [Threads -> 0..MaxGeneration]

OneRunningPerLp ==
    \A lp \in LPs :
        Cardinality({t \in Threads : phase[t] = "Running" /\ location[t] = lp}) <= 1

PlacementValid ==
    \A t \in Threads :
        /\ (phase[t] \in {"Ready", "Running", "Blocked"} => location[t] \in LPs)
        /\ (phase[t] \in {"Absent", "NeedsLp"} => location[t] = NullLp)
        /\ (pinned[t] /= NullLp /\ phase[t] \in {"Ready", "Running", "Blocked"})
              => location[t] = pinned[t]

LiveOwnerValid ==
    \A t \in Threads :
        (phase[t] /= "Absent") => owner[t] \in ASID

DeadNotSchedulable ==
    \A t \in Threads :
        phase[t] = "Dead" => phase[t] \notin {"Ready", "Running", "Blocked"}

BlockedWakerMatches ==
    \A t \in Threads :
        phase[t] = "Blocked" => waiterGeneration[t] = generation[t]

MigrationRespectsAuthority ==
    \A t \in Threads :
        pinned[t] /= NullLp => ~migrationSafe[t]

Invariants ==
    /\ TypeOK
    /\ OneRunningPerLp
    /\ PlacementValid
    /\ LiveOwnerValid
    /\ DeadNotSchedulable
    /\ BlockedWakerMatches
    /\ MigrationRespectsAuthority

=============================================================================
