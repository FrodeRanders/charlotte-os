-------------------- MODULE CharlottePrincipalBacking --------------------
\* Unit-page abstraction of heap, image and object backing. Three root slots
\* may restart with another authenticated principal. Physical confirmation is
\* an explicit action; retirement/move/restart cannot refund retained backing.
\* Node pools, metadata, mappings and concurrent pressure fairness are outside
\* this model. The production shared Budget also has an object-count dimension.
EXTENDS Naturals, FiniteSets
CONSTANT Limit
Roots == {1, 2, 3}
Principals == {"App", "Other"}
Units == {1, 2, 3}
Objects == {1}
VARIABLES alive, generation, principal, used, held, origin, stamp, paid, owner, charged
vars == <<alive, generation, principal, used, held, origin, stamp, paid, owner, charged>>
Init == /\ alive = Roots /\ generation = [r \in Roots |-> 1]
        /\ principal = [r \in Roots |-> IF r = 3 THEN "Other" ELSE "App"]
        /\ used = {} /\ held = {}
        /\ origin = [u \in Units |-> 1] /\ stamp = [u \in Units |-> 1]
        /\ paid = [u \in Units |-> "App"] /\ owner = [u \in Units |-> 1]
        /\ charged = [p \in Principals |-> 0]
AllocateWith(r, u) ==
    /\ r \in alive /\ u \in Units \ used
    /\ used' = used \cup {u} /\ held' = held \cup {u}
    /\ origin' = [origin EXCEPT ![u] = r] /\ stamp' = [stamp EXCEPT ![u] = generation[r]]
    /\ paid' = [paid EXCEPT ![u] = principal[r]] /\ owner' = [owner EXCEPT ![u] = r]
    /\ charged' = [charged EXCEPT ![principal[r]] = @ + 1]
    /\ UNCHANGED <<alive, generation, principal>>
Allocate == \E r \in alive, u \in Units \ used :
    /\ charged[principal[r]] < Limit /\ AllocateWith(r, u)
Move == \E u \in held \cap Objects, r \in alive :
    /\ owner[u] # r /\ owner' = [owner EXCEPT ![u] = r]
    /\ UNCHANGED <<alive, generation, principal, used, held, origin, stamp, paid, charged>>
Retire == \E r \in alive :
    /\ alive' = alive \ {r}
    /\ UNCHANGED <<generation, principal, used, held, origin, stamp, paid, owner, charged>>
RestartWith(r, p) ==
    /\ r = 1 /\ r \notin alive /\ generation[r] = 1
    /\ alive' = alive \cup {r} /\ generation' = [generation EXCEPT ![r] = 2]
    /\ principal' = [principal EXCEPT ![r] = p]
Restart == \E r \in Roots, p \in Principals :
    /\ RestartWith(r, p) /\ UNCHANGED <<used, held, origin, stamp, paid, owner, charged>>
ConfirmRelease == \E u \in held :
    /\ held' = held \ {u} /\ charged' = [charged EXCEPT ![paid[u]] = @ - 1]
    /\ UNCHANGED <<alive, generation, principal, used, origin, stamp, paid, owner>>
Next == Allocate \/ Move \/ Retire \/ Restart \/ ConfirmRelease
Spec == Init /\ [][Next]_vars

UnsafeGenerationOnly == \E r \in alive, u \in Units \ used :
    /\ Cardinality({v \in held : origin[v] = r /\ stamp[v] = generation[r]}) < Limit
    /\ AllocateWith(r, u)
UnsafeRetirementRefund == \E u \in held :
    /\ origin[u] \notin alive /\ charged[paid[u]] > 0
    /\ charged' = [charged EXCEPT ![paid[u]] = @ - 1]
    /\ UNCHANGED <<alive, generation, principal, used, held, origin, stamp, paid, owner>>
UnsafeMoveSponsor == \E u \in held \cap Objects, r \in alive :
    /\ paid[u] # principal[r] /\ charged[paid[u]] > 0
    /\ owner' = [owner EXCEPT ![u] = r]
    /\ charged' = [charged EXCEPT ![paid[u]] = @ - 1, ![principal[r]] = @ + 1]
    /\ UNCHANGED <<alive, generation, principal, used, held, origin, stamp, paid>>
UnsafeReuseSponsor == \E u \in held :
    /\ stamp[u] # generation[origin[u]] /\ paid[u] # principal[origin[u]]
    /\ held' = held \ {u}
    /\ charged' = [charged EXCEPT ![principal[origin[u]]] = @ - 1]
    /\ UNCHANGED <<alive, generation, principal, used, origin, stamp, paid, owner>>
GenerationSpec == Init /\ [][Next \/ UnsafeGenerationOnly]_vars
RefundSpec == Init /\ [][Next \/ UnsafeRetirementRefund]_vars
MoveSpec == Init /\ [][Next \/ UnsafeMoveSponsor]_vars
ReuseSpec == Init /\ [][Next \/ UnsafeReuseSponsor]_vars

TypeOK == /\ alive \subseteq Roots /\ generation \in [Roots -> 1..2]
          /\ principal \in [Roots -> Principals] /\ held \subseteq used /\ used \subseteq Units
          /\ origin \in [Units -> Roots] /\ stamp \in [Units -> 1..2]
          /\ paid \in [Units -> Principals] /\ owner \in [Units -> Roots]
          /\ charged \in [Principals -> 0..Cardinality(Units)]
Accounted == \A p \in Principals : charged[p] = Cardinality({u \in held : paid[u] = p})
PrincipalCeiling == \A p \in Principals : charged[p] <= Limit
Invariants == TypeOK /\ Accounted /\ PrincipalCeiling
=============================================================================
