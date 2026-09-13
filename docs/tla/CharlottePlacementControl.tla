--------------------- MODULE CharlottePlacementControl ---------------------
\* Safety core of the cluster placement feedback controller.
\*
\* Raw observations remain diagnostic data. Placement selects candidates from
\* filtered values, requires pressure to persist for a dwell interval, and
\* imposes a cooldown after movement. Membership/drain loss is represented by
\* removing the assigned node from Eligible and may force an immediate move.
\* Static reservations are admitted independently of transient free memory.

EXTENDS Naturals, FiniteSets

CONSTANTS Node, NoNode, MaxCapacity, Reserve, Demand, Dwell, Cooldown,
          AllowEarlyPressureMove, AllowOvercommit

ASSUME Node \subseteq Nat
ASSUME Node /= {}
ASSUME NoNode \notin Node
ASSUME MaxCapacity > Reserve + Demand
ASSUME Demand > 0
ASSUME Dwell > 0
ASSUME Cooldown > 0

FreeValue == 0..MaxCapacity
Age == 0..Dwell
CooldownValue == 0..Cooldown

Feasible(signal, commitments, members) ==
    {n \in members :
        /\ signal[n] >= Demand + Reserve
        /\ commitments[n] + Demand <= MaxCapacity - Reserve}

Best(signal, commitments, members) ==
    {n \in Feasible(signal, commitments, members) :
        \A other \in Feasible(signal, commitments, members) :
            signal[n] >= signal[other]}

VARIABLES eligible, raw, filtered, assigned, candidate, candidateAge,
          cooldownRemaining, reserved, unstablePressureMove, overcommitted

vars == <<eligible, raw, filtered, assigned, candidate, candidateAge,
          cooldownRemaining, reserved, unstablePressureMove, overcommitted>>

InitialNode == CHOOSE n \in Node : TRUE

Init ==
    /\ eligible = Node
    /\ raw = [n \in Node |-> MaxCapacity]
    /\ filtered = [n \in Node |-> MaxCapacity]
    /\ assigned = InitialNode
    /\ candidate = InitialNode
    /\ candidateAge = 0
    /\ cooldownRemaining = 0
    /\ reserved = [n \in Node |-> IF n = InitialNode THEN Demand ELSE 0]
    /\ unstablePressureMove = FALSE
    /\ overcommitted = FALSE

\* The concrete integer EWMA uses alpha 1/4. Alpha 1/2 keeps the bounded model
\* small while retaining the safety-relevant distinction between raw and
\* filtered values.
Observe(node, value) ==
    /\ node \in Node
    /\ value \in FreeValue
    /\ raw' = [raw EXCEPT ![node] = value]
    /\ filtered' = [filtered EXCEPT ![node] = (@ + value) \div 2]
    /\ UNCHANGED <<eligible, assigned, candidate, candidateAge,
                    cooldownRemaining, reserved, unstablePressureMove,
                    overcommitted>>

SelectCandidate(node) ==
    /\ node \in Best(filtered, reserved, eligible)
    /\ IF node = candidate
          THEN /\ candidate' = candidate
               /\ candidateAge' = candidateAge
          ELSE /\ candidate' = node
               /\ candidateAge' = 0
    /\ UNCHANGED <<eligible, raw, filtered, assigned, cooldownRemaining,
                    reserved, unstablePressureMove, overcommitted>>

Tick ==
    /\ candidateAge' =
          IF candidate /= assigned
          THEN IF candidateAge < Dwell THEN candidateAge + 1 ELSE Dwell
          ELSE 0
    /\ cooldownRemaining' =
          IF cooldownRemaining = 0 THEN 0 ELSE cooldownRemaining - 1
    /\ UNCHANGED <<eligible, raw, filtered, assigned, candidate, reserved,
                    unstablePressureMove, overcommitted>>

PressureMove ==
    /\ candidate \in eligible
    /\ candidate /= assigned
    /\ candidateAge = Dwell
    /\ cooldownRemaining = 0
    /\ reserved[assigned] >= Demand
    /\ reserved[candidate] + Demand <= MaxCapacity - Reserve
    /\ assigned' = candidate
    /\ reserved' = [reserved EXCEPT
          ![assigned] = @ - Demand,
          ![candidate] = @ + Demand]
    /\ candidateAge' = 0
    /\ cooldownRemaining' = Cooldown
    /\ UNCHANGED <<eligible, raw, filtered, candidate, unstablePressureMove,
                    overcommitted>>

RemoveEligibility(node) ==
    /\ node \in eligible
    /\ Cardinality(eligible) > 1
    /\ eligible' = eligible \ {node}
    /\ UNCHANGED <<raw, filtered, assigned, candidate, candidateAge,
                    cooldownRemaining, reserved, unstablePressureMove,
                    overcommitted>>

ForcedMove(node) ==
    /\ assigned \notin eligible
    /\ node \in Feasible(filtered, reserved, eligible)
    /\ reserved[assigned] >= Demand
    /\ assigned' = node
    /\ candidate' = node
    /\ reserved' = [reserved EXCEPT
          ![assigned] = @ - Demand,
          ![node] = @ + Demand]
    /\ candidateAge' = 0
    /\ cooldownRemaining' = Cooldown
    /\ UNCHANGED <<eligible, raw, filtered, unstablePressureMove,
                    overcommitted>>

Admit(node) ==
    /\ node \in eligible
    /\ reserved[node] + Demand <= MaxCapacity - Reserve
    /\ reserved' = [reserved EXCEPT ![node] = @ + Demand]
    /\ UNCHANGED <<eligible, raw, filtered, assigned, candidate,
                    candidateAge, cooldownRemaining, unstablePressureMove,
                    overcommitted>>

UnsafeEarlyPressureMove ==
    /\ AllowEarlyPressureMove
    /\ candidate \in eligible
    /\ candidate /= assigned
    /\ (candidateAge < Dwell \/ cooldownRemaining > 0)
    /\ reserved[assigned] >= Demand
    /\ reserved[candidate] + Demand <= MaxCapacity - Reserve
    /\ assigned' = candidate
    /\ reserved' = [reserved EXCEPT
          ![assigned] = @ - Demand,
          ![candidate] = @ + Demand]
    /\ unstablePressureMove' = TRUE
    /\ candidateAge' = 0
    /\ cooldownRemaining' = Cooldown
    /\ UNCHANGED <<eligible, raw, filtered, candidate, overcommitted>>

UnsafeAdmit(node) ==
    /\ AllowOvercommit
    /\ node \in eligible
    /\ reserved[node] + Demand > MaxCapacity - Reserve
    /\ reserved' = [reserved EXCEPT ![node] = @ + Demand]
    /\ overcommitted' = TRUE
    /\ UNCHANGED <<eligible, raw, filtered, assigned, candidate,
                    candidateAge, cooldownRemaining, unstablePressureMove>>

Next ==
    \/ \E node \in Node, value \in FreeValue : Observe(node, value)
    \/ \E node \in Node : SelectCandidate(node)
    \/ Tick
    \/ PressureMove
    \/ \E node \in Node : RemoveEligibility(node)
    \/ \E node \in Node : ForcedMove(node)
    \/ \E node \in Node : Admit(node)
    \/ UnsafeEarlyPressureMove
    \/ \E node \in Node : UnsafeAdmit(node)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ eligible \subseteq Node
    /\ eligible /= {}
    /\ raw \in [Node -> FreeValue]
    /\ filtered \in [Node -> FreeValue]
    /\ assigned \in Node
    /\ candidate \in Node \cup {NoNode}
    /\ candidateAge \in Age
    /\ cooldownRemaining \in CooldownValue
    /\ reserved \in [Node -> Nat]
    /\ unstablePressureMove \in BOOLEAN
    /\ overcommitted \in BOOLEAN

ReservationsPreserveReserve ==
    /\ ~overcommitted
    /\ \A node \in Node : reserved[node] <= MaxCapacity - Reserve

PressureRespectsStability == ~unstablePressureMove

Invariants == TypeOK /\ ReservationsPreserveReserve /\ PressureRespectsStability

=============================================================================
