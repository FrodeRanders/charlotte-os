-------------------------- MODULE CharlotteScheduler --------------------------
\* Finite safety model of CharlotteOS thread admission, physical execution,
\* blocking/wakeup, migration, cross-LP abort, owner-LP retirement, reaping,
\* generation-safe wakeup, and address-space teardown.

EXTENDS Naturals, FiniteSets

CONSTANTS Threads, LPs, ASID, MaxGeneration, NullAsid, NullLp

ASSUME NullAsid \notin ASID
ASSUME NullLp \notin LPs
ASSUME MaxGeneration > 1

ThreadPhase == {"Absent", "NeedsLp", "Ready", "Running", "Blocked", "Dead"}
AddressSpacePhase == {"Live", "Aborting", "Destroyed"}

VARIABLES phase, generation, owner, location, affinity, pinned,
          migrationSafe, waiterGeneration, onCpu,
          abortRequested, abortOwner, addressSpacePhase

vars == <<phase, generation, owner, location, affinity, pinned,
          migrationSafe, waiterGeneration, onCpu,
          abortRequested, abortOwner, addressSpacePhase>>

Init ==
    /\ phase = [t \in Threads |-> "Absent"]
    /\ generation = [t \in Threads |-> 0]
    /\ owner = [t \in Threads |-> NullAsid]
    /\ location = [t \in Threads |-> NullLp]
    /\ affinity = [t \in Threads |-> NullLp]
    /\ pinned = [t \in Threads |-> NullLp]
    /\ migrationSafe = [t \in Threads |-> FALSE]
    /\ waiterGeneration = [t \in Threads |-> 0]
    /\ onCpu = [t \in Threads |-> FALSE]
    /\ abortRequested = [t \in Threads |-> FALSE]
    /\ abortOwner = [t \in Threads |-> NullLp]
    /\ addressSpacePhase = [as \in ASID |-> "Live"]

\* Physical execution is deliberately separate from ThreadState. A self-blocked
\* thread remains on its LP until the IRQ/syscall tail switches stacks, and a
\* wake may change its table phase before that switch occurs.
LpIdle(lp) == ~(\E t \in Threads : onCpu[t] /\ location[t] = lp)

Spawn(t, as, canMigrate, pin) ==
    /\ phase[t] = "Absent"
    /\ addressSpacePhase[as] = "Live"
    /\ generation[t] < MaxGeneration
    /\ pin \in LPs \cup {NullLp}
    /\ phase' = [phase EXCEPT ![t] = "NeedsLp"]
    /\ generation' = [generation EXCEPT ![t] = @ + 1]
    /\ owner' = [owner EXCEPT ![t] = as]
    /\ location' = [location EXCEPT ![t] = NullLp]
    /\ affinity' = [affinity EXCEPT ![t] = pin]
    /\ pinned' = [pinned EXCEPT ![t] = pin]
    /\ migrationSafe' = [migrationSafe EXCEPT ![t] = canMigrate /\ pin = NullLp]
    /\ abortRequested' = [abortRequested EXCEPT ![t] = FALSE]
    /\ abortOwner' = [abortOwner EXCEPT ![t] = NullLp]
    /\ UNCHANGED <<waiterGeneration, onCpu, addressSpacePhase>>

Admit(t, lp) ==
    /\ phase[t] = "NeedsLp"
    /\ ~abortRequested[t]
    /\ pinned[t] \in {NullLp, lp}
    /\ phase' = [phase EXCEPT ![t] = "Ready"]
    /\ location' = [location EXCEPT ![t] = lp]
    /\ affinity' = [affinity EXCEPT ![t] = lp]
    /\ UNCHANGED <<generation, owner, pinned, migrationSafe,
                   waiterGeneration, onCpu, abortRequested, abortOwner,
                   addressSpacePhase>>

Dispatch(t) ==
    /\ phase[t] = "Ready"
    /\ ~onCpu[t]
    /\ ~abortRequested[t]
    /\ LpIdle(location[t])
    /\ phase' = [phase EXCEPT ![t] = "Running"]
    /\ onCpu' = [onCpu EXCEPT ![t] = TRUE]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration, abortRequested, abortOwner,
                   addressSpacePhase>>

Preempt(t) ==
    /\ phase[t] = "Running"
    /\ onCpu[t]
    /\ ~abortRequested[t]
    /\ phase' = [phase EXCEPT ![t] = "Ready"]
    /\ onCpu' = [onCpu EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration, abortRequested, abortOwner,
                   addressSpacePhase>>

Block(t) ==
    /\ phase[t] \in {"Running", "Ready"}
    /\ phase' = [phase EXCEPT ![t] = "Blocked"]
    /\ waiterGeneration' = [waiterGeneration EXCEPT ![t] = generation[t]]
    \* Preserve onCpu: concrete self-block changes ThreadState before switch_ctx.
    /\ UNCHANGED <<generation, owner, location, affinity, pinned, migrationSafe,
                   onCpu, abortRequested, abortOwner, addressSpacePhase>>

Wake(t, observedGeneration) ==
    /\ phase[t] = "Blocked"
    /\ ~abortRequested[t]
    /\ observedGeneration = generation[t]
    /\ observedGeneration = waiterGeneration[t]
    /\ phase' = [phase EXCEPT ![t] = "Ready"]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration, onCpu,
                   abortRequested, abortOwner, addressSpacePhase>>

\* Finish an ordinary block, or the ready-before-switch wake race, by leaving
\* the outgoing stack. Ready remains queued for a later dispatch.
SwitchOff(t) ==
    /\ phase[t] \in {"Blocked", "Ready", "Dead"}
    /\ onCpu[t]
    /\ ~abortRequested[t]
    /\ onCpu' = [onCpu EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<phase, generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration, abortRequested, abortOwner,
                   addressSpacePhase>>

Migrate(t, destination) ==
    /\ phase[t] = "Ready"
    /\ ~onCpu[t]
    /\ ~abortRequested[t]
    /\ migrationSafe[t]
    /\ pinned[t] = NullLp
    /\ destination /= location[t]
    /\ location' = [location EXCEPT ![t] = destination]
    /\ affinity' = [affinity EXCEPT ![t] = destination]
    /\ UNCHANGED <<phase, generation, owner, pinned, migrationSafe,
                   waiterGeneration, onCpu, abortRequested, abortOwner,
                   addressSpacePhase>>

\* A caller on another LP cannot remove a physically executing context. It
\* records the owning LP and requests a scheduler IPI. The target may execute
\* Block before the IPI is handled, which is why phase is left unchanged.
RequestRemoteAbort(t, requester) ==
    /\ phase[t] \in {"Running", "Ready", "Blocked"}
    /\ onCpu[t]
    /\ requester \in LPs
    /\ requester /= location[t]
    /\ ~abortRequested[t]
    /\ abortRequested' = [abortRequested EXCEPT ![t] = TRUE]
    /\ abortOwner' = [abortOwner EXCEPT ![t] = location[t]]
    /\ UNCHANGED <<phase, generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration, onCpu, addressSpacePhase>>

\* The recorded owner LP switches off the target and atomically removes it from
\* scheduling authority into that LP's deferred-dead set.
RetireRemoteAbort(t, lp) ==
    /\ abortRequested[t]
    /\ abortOwner[t] = lp
    /\ location[t] = lp
    /\ onCpu[t]
    /\ phase' = [phase EXCEPT ![t] = "Dead"]
    /\ onCpu' = [onCpu EXCEPT ![t] = FALSE]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration, abortRequested, abortOwner,
                   addressSpacePhase>>

\* Non-running aborts can be removed immediately. A self-exit is staged Dead
\* while still on its stack and must take SwitchOff before Reap.
AbortNotRunning(t) ==
    /\ phase[t] \in {"NeedsLp", "Ready", "Blocked"}
    /\ ~onCpu[t]
    /\ phase' = [phase EXCEPT ![t] = "Dead"]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration, onCpu,
                   abortRequested, abortOwner, addressSpacePhase>>

SelfAbort(t) ==
    /\ phase[t] \in {"Running", "Ready", "Blocked"}
    /\ onCpu[t]
    /\ ~abortRequested[t]
    /\ phase' = [phase EXCEPT ![t] = "Dead"]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration, onCpu,
                   abortRequested, abortOwner, addressSpacePhase>>

\* Establish the domain-wide abort fence in one abstract transition. Concrete
\* Rust first blocks thread publication, snapshots the address space, and then
\* performs these per-thread state changes while publication remains blocked.
\* Threads already executing are retired by their owner LP; all other threads
\* are immediately removed from scheduling authority.
BeginDomainAbort(as) ==
    /\ addressSpacePhase[as] = "Live"
    /\ addressSpacePhase' = [addressSpacePhase EXCEPT ![as] = "Aborting"]
    /\ phase' =
        [t \in Threads |->
            IF owner[t] = as /\ phase[t] \in {"NeedsLp", "Ready", "Blocked"}
               /\ ~onCpu[t]
            THEN "Dead"
            ELSE phase[t]]
    /\ abortRequested' =
        [t \in Threads |->
            IF owner[t] = as /\ phase[t] # "Absent" /\ onCpu[t]
            THEN TRUE
            ELSE abortRequested[t]]
    /\ abortOwner' =
        [t \in Threads |->
            IF owner[t] = as /\ phase[t] # "Absent" /\ onCpu[t]
            THEN location[t]
            ELSE abortOwner[t]]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration, onCpu>>

Reap(t) ==
    /\ phase[t] = "Dead"
    /\ ~onCpu[t]
    /\ phase' = [phase EXCEPT ![t] = "Absent"]
    /\ owner' = [owner EXCEPT ![t] = NullAsid]
    /\ location' = [location EXCEPT ![t] = NullLp]
    /\ affinity' = [affinity EXCEPT ![t] = NullLp]
    /\ pinned' = [pinned EXCEPT ![t] = NullLp]
    /\ migrationSafe' = [migrationSafe EXCEPT ![t] = FALSE]
    /\ abortRequested' = [abortRequested EXCEPT ![t] = FALSE]
    /\ abortOwner' = [abortOwner EXCEPT ![t] = NullLp]
    \* Retain waiterGeneration: a stale Waker may outlive slot reuse.
    /\ UNCHANGED <<generation, waiterGeneration, onCpu, addressSpacePhase>>

DestroyAddressSpace(as) ==
    /\ addressSpacePhase[as] \in {"Live", "Aborting"}
    /\ \A t \in Threads : owner[t] /= as \/ phase[t] = "Absent"
    /\ addressSpacePhase' = [addressSpacePhase EXCEPT ![as] = "Destroyed"]
    /\ UNCHANGED <<phase, generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration, onCpu,
                   abortRequested, abortOwner>>

Next ==
    \/ \E t \in Threads, as \in ASID, movable \in BOOLEAN,
          pin \in LPs \cup {NullLp} : Spawn(t, as, movable, pin)
    \/ \E t \in Threads, lp \in LPs : Admit(t, lp)
    \/ \E t \in Threads : Dispatch(t)
    \/ \E t \in Threads : Preempt(t)
    \/ \E t \in Threads : Block(t)
    \/ \E t \in Threads, gen \in 0..MaxGeneration : Wake(t, gen)
    \/ \E t \in Threads : SwitchOff(t)
    \/ \E t \in Threads, lp \in LPs : Migrate(t, lp)
    \/ \E t \in Threads, requester \in LPs : RequestRemoteAbort(t, requester)
    \/ \E t \in Threads, lp \in LPs : RetireRemoteAbort(t, lp)
    \/ \E t \in Threads : AbortNotRunning(t)
    \/ \E t \in Threads : SelfAbort(t)
    \/ \E as \in ASID : BeginDomainAbort(as)
    \/ \E t \in Threads : Reap(t)
    \/ \E as \in ASID : DestroyAddressSpace(as)

Spec == Init /\ [][Next]_vars

\* Regression model of the former implementation: a remote caller removed a
\* Running thread from scheduler/master-table authority while the target was
\* still physically executing on its LP. This action is intentionally excluded
\* from Spec and enabled only by CharlotteScheduler_unsafe.cfg, whose TLC run
\* must produce a ReapOnlyOffCpu counterexample.
UnsafeRemoteAbort(t, requester) ==
    /\ phase[t] = "Running"
    /\ onCpu[t]
    /\ requester \in LPs
    /\ requester /= location[t]
    /\ phase' = [phase EXCEPT ![t] = "Absent"]
    /\ UNCHANGED <<generation, owner, location, affinity, pinned,
                   migrationSafe, waiterGeneration, onCpu,
                   abortRequested, abortOwner, addressSpacePhase>>

\* Regression model of the unfenced implementation: a sibling publishes a
\* fresh thread after domain abort took its snapshot.
UnsafeSpawnDuringAbort(t, as, canMigrate, pin) ==
    /\ phase[t] = "Absent"
    /\ addressSpacePhase[as] = "Aborting"
    /\ generation[t] < MaxGeneration
    /\ pin \in LPs \cup {NullLp}
    /\ phase' = [phase EXCEPT ![t] = "NeedsLp"]
    /\ generation' = [generation EXCEPT ![t] = @ + 1]
    /\ owner' = [owner EXCEPT ![t] = as]
    /\ location' = [location EXCEPT ![t] = NullLp]
    /\ affinity' = [affinity EXCEPT ![t] = pin]
    /\ pinned' = [pinned EXCEPT ![t] = pin]
    /\ migrationSafe' = [migrationSafe EXCEPT ![t] = canMigrate /\ pin = NullLp]
    /\ abortRequested' = [abortRequested EXCEPT ![t] = FALSE]
    /\ abortOwner' = [abortOwner EXCEPT ![t] = NullLp]
    /\ UNCHANGED <<waiterGeneration, onCpu, addressSpacePhase>>

UnsafeNext == Next \/
    \E t \in Threads, requester \in LPs : UnsafeRemoteAbort(t, requester)

UnsafeSpec == Init /\ [][UnsafeNext]_vars

UnsafeDomainNext == Next \/
    \E t \in Threads, as \in ASID, movable \in BOOLEAN,
       pin \in LPs \cup {NullLp} : UnsafeSpawnDuringAbort(t, as, movable, pin)

UnsafeDomainSpec == Init /\ [][UnsafeDomainNext]_vars

TypeOK ==
    /\ phase \in [Threads -> ThreadPhase]
    /\ generation \in [Threads -> 0..MaxGeneration]
    /\ owner \in [Threads -> ASID \cup {NullAsid}]
    /\ location \in [Threads -> LPs \cup {NullLp}]
    /\ affinity \in [Threads -> LPs \cup {NullLp}]
    /\ pinned \in [Threads -> LPs \cup {NullLp}]
    /\ migrationSafe \in [Threads -> BOOLEAN]
    /\ waiterGeneration \in [Threads -> 0..MaxGeneration]
    /\ onCpu \in [Threads -> BOOLEAN]
    /\ abortRequested \in [Threads -> BOOLEAN]
    /\ abortOwner \in [Threads -> LPs \cup {NullLp}]
    /\ addressSpacePhase \in [ASID -> AddressSpacePhase]

OneExecutingPerLp ==
    \A lp \in LPs :
        Cardinality({t \in Threads : onCpu[t] /\ location[t] = lp}) <= 1

PlacementValid ==
    \A t \in Threads :
        /\ (phase[t] \in {"Ready", "Running", "Blocked"} => location[t] \in LPs)
        /\ (phase[t] \in {"Absent", "NeedsLp"} => location[t] = NullLp)
        /\ (phase[t] = "Dead" => location[t] \in LPs \cup {NullLp})
        /\ (pinned[t] /= NullLp /\ phase[t] \in {"Ready", "Running", "Blocked"})
              => location[t] = pinned[t]

LiveOwnerValid ==
    \A t \in Threads : phase[t] /= "Absent" => owner[t] \in ASID

OnCpuHasLiveAddressSpace ==
    \A t \in Threads : onCpu[t] => addressSpacePhase[owner[t]] # "Destroyed"

ReapOnlyOffCpu ==
    \A t \in Threads : phase[t] = "Absent" => ~onCpu[t]

AbortRequestOwned ==
    \A t \in Threads :
        abortRequested[t] => abortOwner[t] = location[t] /\ abortOwner[t] \in LPs

AbortRequestCannotBeRedispatched ==
    \A t \in Threads : abortRequested[t] /\ phase[t] = "Ready" => onCpu[t]

BlockedWakerMatches ==
    \A t \in Threads :
        phase[t] = "Blocked" => waiterGeneration[t] = generation[t]

MigrationRespectsAuthority ==
    \A t \in Threads : pinned[t] /= NullLp => ~migrationSafe[t]

DestroyedAddressSpaceEmpty ==
    \A as \in ASID :
        addressSpacePhase[as] = "Destroyed" =>
            \A t \in Threads : owner[t] /= as \/ phase[t] = "Absent"

AbortingThreadsDoomed ==
    \A as \in ASID :
        addressSpacePhase[as] = "Aborting" =>
            \A t \in Threads :
                owner[t] /= as \/ phase[t] \in {"Absent", "Dead"} \/ abortRequested[t]

Invariants ==
    /\ TypeOK
    /\ OneExecutingPerLp
    /\ PlacementValid
    /\ LiveOwnerValid
    /\ OnCpuHasLiveAddressSpace
    /\ ReapOnlyOffCpu
    /\ AbortRequestOwned
    /\ AbortRequestCannotBeRedispatched
    /\ BlockedWakerMatches
    /\ MigrationRespectsAuthority
    /\ DestroyedAddressSpaceEmpty
    /\ AbortingThreadsDoomed

=============================================================================
