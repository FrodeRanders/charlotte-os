-------------------- MODULE CharlottePrincipalBacking --------------------
\* Three charged footprints: object (one page), reserved stack capacity (two),
\* and a private table (one). Heap/image billing follows the same unit policy.
\* Three root slots
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
Cost(u) == IF u = 2 THEN 2 ELSE 1
Pages(S) == (IF 1 \in S THEN 1 ELSE 0) + (IF 2 \in S THEN 2 ELSE 0) + (IF 3 \in S THEN 1 ELSE 0)
VARIABLES alive, generation, principal, used, held, origin, stamp, paid, owner, charged, grown
vars == <<alive, generation, principal, used, held, origin, stamp, paid, owner, charged, grown>>
Init == /\ alive = Roots /\ generation = [r \in Roots |-> 1]
        /\ principal = [r \in Roots |-> IF r = 3 THEN "Other" ELSE "App"]
        /\ used = {} /\ held = {} /\ grown = {}
        /\ origin = [u \in Units |-> 1] /\ stamp = [u \in Units |-> 1]
        /\ paid = [u \in Units |-> "App"] /\ owner = [u \in Units |-> 1]
        /\ charged = [p \in Principals |-> 0]
AllocateWith(r, u) ==
    /\ r \in alive /\ u \in Units \ used
    /\ used' = used \cup {u} /\ held' = held \cup {u}
    /\ origin' = [origin EXCEPT ![u] = r] /\ stamp' = [stamp EXCEPT ![u] = generation[r]]
    /\ paid' = [paid EXCEPT ![u] = principal[r]] /\ owner' = [owner EXCEPT ![u] = r]
    /\ charged' = [charged EXCEPT ![principal[r]] = @ + Cost(u)]
    /\ UNCHANGED <<alive, generation, principal, grown>>
Allocate == \E r \in alive, u \in Units \ used :
    /\ charged[principal[r]] + Cost(u) <= Limit /\ AllocateWith(r, u)
Move == \E u \in held \cap Objects, r \in alive :
    /\ owner[u] # r /\ owner' = [owner EXCEPT ![u] = r]
    /\ UNCHANGED <<alive, generation, principal, used, held, origin, stamp, paid, charged, grown>>
Retire == \E r \in alive :
    /\ alive' = alive \ {r}
    /\ UNCHANGED <<generation, principal, used, held, origin, stamp, paid, owner, charged, grown>>
RestartWith(r, p) ==
    /\ r = 1 /\ r \notin alive /\ generation[r] = 1
    /\ alive' = alive \cup {r} /\ generation' = [generation EXCEPT ![r] = 2]
    /\ principal' = [principal EXCEPT ![r] = p]
Restart == \E r \in Roots, p \in Principals :
    /\ RestartWith(r, p) /\ UNCHANGED <<used, held, origin, stamp, paid, owner, charged, grown>>
ConfirmRelease == \E u \in held :
    /\ held' = held \ {u} /\ charged' = [charged EXCEPT ![paid[u]] = @ - Cost(u)]
    /\ UNCHANGED <<alive, generation, principal, used, origin, stamp, paid, owner, grown>>
Grow ==
    /\ 2 \in held /\ 2 \notin grown /\ grown' = grown \cup {2}
    /\ UNCHANGED <<alive, generation, principal, used, held, origin, stamp, paid, owner, charged>>
Next == Allocate \/ Grow \/ Move \/ Retire \/ Restart \/ ConfirmRelease
Spec == Init /\ [][Next]_vars

UnsafeGenerationOnly == \E r \in alive, u \in Units \ used :
    /\ Pages({v \in held : origin[v] = r /\ stamp[v] = generation[r]}) + Cost(u) <= Limit
    /\ AllocateWith(r, u)
UnsafeRetirementRefund == \E u \in held :
    /\ origin[u] \notin alive /\ charged[paid[u]] > 0
    /\ charged' = [charged EXCEPT ![paid[u]] = @ - Cost(u)]
    /\ UNCHANGED <<alive, generation, principal, used, held, origin, stamp, paid, owner, grown>>
UnsafeMoveSponsor == \E u \in held \cap Objects, r \in alive :
    /\ paid[u] # principal[r] /\ charged[paid[u]] > 0
    /\ owner' = [owner EXCEPT ![u] = r]
    /\ charged' = [charged EXCEPT ![paid[u]] = @ - Cost(u), ![principal[r]] = @ + Cost(u)]
    /\ UNCHANGED <<alive, generation, principal, used, held, origin, stamp, paid, grown>>
UnsafeReuseSponsor == \E u \in held :
    /\ stamp[u] # generation[origin[u]] /\ paid[u] # principal[origin[u]]
    /\ held' = held \ {u}
    /\ charged' = [charged EXCEPT ![principal[origin[u]]] = @ - Cost(u)]
    /\ UNCHANGED <<alive, generation, principal, used, origin, stamp, paid, owner, grown>>
UnsafeGrowCharge ==
    /\ 2 \in held /\ 2 \notin grown /\ grown' = grown \cup {2}
    /\ charged' = [charged EXCEPT ![paid[2]] = @ + 1]
    /\ UNCHANGED <<alive, generation, principal, used, held, origin, stamp, paid, owner>>
UnsafeDropRefund ==
    /\ 2 \in held /\ 2 \notin grown /\ charged[paid[2]] >= Cost(2)
    /\ charged' = [charged EXCEPT ![paid[2]] = @ - Cost(2)]
    /\ UNCHANGED <<alive, generation, principal, used, held, origin, stamp, paid, owner, grown>>
GrowthSpec == Init /\ [][Next \/ UnsafeGrowCharge]_vars
DropSpec == Init /\ [][Next \/ UnsafeDropRefund]_vars
GenerationSpec == Init /\ [][Next \/ UnsafeGenerationOnly]_vars
RefundSpec == Init /\ [][Next \/ UnsafeRetirementRefund]_vars
MoveSpec == Init /\ [][Next \/ UnsafeMoveSponsor]_vars
ReuseSpec == Init /\ [][Next \/ UnsafeReuseSponsor]_vars

TypeOK == /\ alive \subseteq Roots /\ generation \in [Roots -> 1..2]
          /\ grown \subseteq used /\ principal \in [Roots -> Principals] /\ held \subseteq used /\ used \subseteq Units
          /\ origin \in [Units -> Roots] /\ stamp \in [Units -> 1..2]
          /\ paid \in [Units -> Principals] /\ owner \in [Units -> Roots]
          /\ charged \in [Principals -> 0..Pages(Units)]
Accounted == \A p \in Principals : charged[p] = Pages({u \in held : paid[u] = p})
PrincipalCeiling == \A p \in Principals : charged[p] <= Limit
Invariants == TypeOK /\ Accounted /\ PrincipalCeiling
=============================================================================
