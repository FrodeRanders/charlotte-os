-------------------- MODULE CharlotteThreadMetadata --------------------
\* One prepaid context/node bundle follows its captured principal through
\* construction, publication, explicit disposal and actual node deallocation.
\* Root retirement/reuse and abandonment do not refund storage. Byte/count
\* policy is finite; node reserve, physical stacks, allocator overhead, IRQs
\* and observer/table allocations are exercised separately or remain gaps.
EXTENDS Naturals, FiniteSets
Roots == {1, 2}
Units == {1, 2}
Principals == {"App", "Other"}
Bytes(u) == u
ByteSum(S) == (IF 1 \in S THEN 1 ELSE 0) + (IF 2 \in S THEN 2 ELSE 0)
VARIABLES alive, generation, principal, phase, abandoned, origin, stamp, paid, billed, charged
vars == <<alive, generation, principal, phase, abandoned, origin, stamp, paid, billed, charged>>
\* 0 unused, 1 reserved, 2 node, 3 context, 4 published, 5 context disposed,
\* 6 node freed, 7 refund complete. A unit is admitted at most once.
Init == /\ alive = Roots /\ generation = [r \in Roots |-> 1]
        /\ principal = [r \in Roots |-> "App"]
        /\ phase = [u \in Units |-> 0] /\ abandoned = {} /\ charged = {}
        /\ origin = [u \in Units |-> 1] /\ stamp = [u \in Units |-> 1]
        /\ paid = [u \in Units |-> "App"] /\ billed = paid
AdmitWith(r, u) ==
    /\ r \in alive /\ phase[u] = 0
    /\ phase' = [phase EXCEPT ![u] = 1] /\ charged' = charged \cup {u}
    /\ origin' = [origin EXCEPT ![u] = r] /\ stamp' = [stamp EXCEPT ![u] = generation[r]]
    /\ paid' = [paid EXCEPT ![u] = principal[r]] /\ billed' = [billed EXCEPT ![u] = principal[r]]
    /\ UNCHANGED <<alive, generation, principal, abandoned>>
Admit == \E r \in alive, u \in Units :
    LET S == {v \in charged : billed[v] = principal[r]} IN
    /\ ByteSum(S) + Bytes(u) <= 2 /\ Cardinality(S) + 1 <= 1
    /\ AdmitWith(r, u)
AllocateNode == \E u \in Units :
    /\ u \notin abandoned /\ phase[u] \in {1}
    /\ phase' = [phase EXCEPT ![u] = 2]
    /\ UNCHANGED <<alive, generation, principal, abandoned, origin, stamp, paid, billed, charged>>
AllocateContext == \E u \in Units :
    /\ u \notin abandoned /\ phase[u] \in {2}
    /\ phase' = [phase EXCEPT ![u] = 3]
    /\ UNCHANGED <<alive, generation, principal, abandoned, origin, stamp, paid, billed, charged>>
Publish == \E u \in Units :
    /\ u \notin abandoned /\ phase[u] \in {3}
    /\ phase' = [phase EXCEPT ![u] = 4]
    /\ UNCHANGED <<alive, generation, principal, abandoned, origin, stamp, paid, billed, charged>>
\* The containing thread owner supplies confirmed stack retirement.
DisposeContext == \E u \in Units :
    /\ u \notin abandoned /\ phase[u] \in {3, 4}
    /\ phase' = [phase EXCEPT ![u] = 5]
    /\ UNCHANGED <<alive, generation, principal, abandoned, origin, stamp, paid, billed, charged>>
FreeNode == \E u \in Units :
    /\ u \notin abandoned /\ phase[u] \in {2, 5}
    /\ phase' = [phase EXCEPT ![u] = 6]
    /\ UNCHANGED <<alive, generation, principal, abandoned, origin, stamp, paid, billed, charged>>
Refund == \E u \in charged :
    /\ u \notin abandoned /\ phase[u] \in {1, 6}
    /\ phase' = [phase EXCEPT ![u] = 7] /\ charged' = charged \ {u}
    /\ UNCHANGED <<alive, generation, principal, abandoned, origin, stamp, paid, billed>>
Abandon == \E u \in charged \ abandoned :
    /\ abandoned' = abandoned \cup {u}
    /\ UNCHANGED <<alive, generation, principal, phase, origin, stamp, paid, billed, charged>>
Retire == \E r \in alive :
    /\ alive' = alive \ {r}
    /\ UNCHANGED <<generation, principal, phase, abandoned, origin, stamp, paid, billed, charged>>
Reuse ==
    /\ 1 \notin alive /\ generation[1] = 1
    /\ alive' = alive \cup {1} /\ generation' = [generation EXCEPT ![1] = 2]
    /\ principal' = [principal EXCEPT ![1] = "Other"]
    /\ UNCHANGED <<phase, abandoned, origin, stamp, paid, billed, charged>>
Next == Admit \/ AllocateNode \/ AllocateContext \/ Publish \/ DisposeContext \/ FreeNode \/ Refund \/ Abandon \/ Retire \/ Reuse
Spec == Init /\ [][Next]_vars

UnsafeGenerationOnly == \E r \in alive, u \in Units :
    LET S == {v \in charged : origin[v] = r /\ stamp[v] = generation[r]} IN
    /\ ByteSum(S) + Bytes(u) <= 2 /\ Cardinality(S) + 1 <= 1
    /\ AdmitWith(r, u)
UnsafeEarlyRefund == \E u \in charged :
    /\ phase[u] \in {2, 3, 4, 5} /\ charged' = charged \ {u}
    /\ UNCHANGED <<alive, generation, principal, phase, abandoned, origin, stamp, paid, billed>>
UnsafeDropRefund == \E u \in charged \cap abandoned :
    /\ charged' = charged \ {u}
    /\ UNCHANGED <<alive, generation, principal, phase, abandoned, origin, stamp, paid, billed>>
UnsafeReplacementSponsor == \E u \in charged :
    /\ stamp[u] # generation[origin[u]] /\ paid[u] # principal[origin[u]]
    /\ billed' = [billed EXCEPT ![u] = principal[origin[u]]]
    /\ UNCHANGED <<alive, generation, principal, phase, abandoned, origin, stamp, paid, charged>>
GenerationSpec == Init /\ [][Next \/ UnsafeGenerationOnly]_vars
EarlySpec == Init /\ [][Next \/ UnsafeEarlyRefund]_vars
DropSpec == Init /\ [][Next \/ UnsafeDropRefund]_vars
ReuseSpec == Init /\ [][Next \/ UnsafeReplacementSponsor]_vars
TypeOK == /\ alive \subseteq Roots /\ generation \in [Roots -> 1..2]
          /\ principal \in [Roots -> Principals] /\ phase \in [Units -> 0..7]
          /\ abandoned \subseteq Units /\ charged \subseteq Units
          /\ origin \in [Units -> Roots] /\ stamp \in [Units -> 1..2]
          /\ paid \in [Units -> Principals] /\ billed \in [Units -> Principals]
Accounted == charged = {u \in Units : phase[u] \in 1..6}
OriginalSponsor == \A u \in charged : billed[u] = paid[u]
PrincipalCeiling == \A p \in Principals :
    LET S == {u \in charged : billed[u] = p} IN
    /\ ByteSum(S) <= 2 /\ Cardinality(S) <= 1
Invariants == TypeOK /\ Accounted /\ OriginalSponsor /\ PrincipalCeiling
=============================================================================
