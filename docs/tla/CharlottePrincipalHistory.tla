-------------------- MODULE CharlottePrincipalHistory --------------------
\* Three candidate nodes share total/ordinary admission. Nodes 1/3 resolve to
\* the same principal with different captured classes. Node 2 is independent.
\* Actual allocator free and root-invalidation proofs are boundary inputs.
EXTENDS Naturals, FiniteSets
Units == {1, 2, 3}
Principal(u) == IF u = 2 THEN "Other" ELSE "App"
Class(u) == IF u = 3 THEN "Platform" ELSE "Ordinary"
VARIABLES phase, charged, billed, decided, published, attached, exited, confirmed, abandoned
vars == <<phase, charged, billed, decided, published, attached, exited, confirmed, abandoned>>
Init == /\ phase = [u \in Units |-> "Unused"] /\ charged = {}
        /\ billed = [u \in Units |-> Class(u)]
        /\ decided = {} /\ published = {} /\ attached = {}
        /\ exited = {} /\ confirmed = {} /\ abandoned = {}
Ordinary == {u \in charged : billed[u] = "Ordinary"}
PrepareWith(u) ==
    /\ phase[u] = "Unused" /\ phase' = [phase EXCEPT ![u] = "Prepared"]
    /\ charged' = charged \cup {u}
    /\ UNCHANGED <<billed, decided, published, attached, exited, confirmed, abandoned>>
Prepare == \E u \in Units :
    /\ Cardinality(charged) < 2
    /\ (Class(u) = "Platform" \/ Cardinality(Ordinary) < 1)
    /\ PrepareWith(u)
Attach == \E u \in Units :
    /\ phase[u] = "Prepared" /\ u \notin abandoned
    /\ phase' = [phase EXCEPT ![u] = "Attached"] /\ attached' = attached \cup {u}
    /\ UNCHANGED <<charged, billed, decided, published, exited, confirmed, abandoned>>
Decide == \E u \in Units :
    /\ phase[u] \in {"Prepared", "Attached"} /\ u \notin decided \cup abandoned
    /\ decided' = decided \cup {u}
    /\ IF \E v \in published : Principal(v) = Principal(u)
       THEN /\ UNCHANGED <<phase, published>>
       ELSE /\ phase' = [phase EXCEPT ![u] = "Record"] /\ published' = published \cup {u}
    /\ UNCHANGED <<charged, billed, attached, exited, confirmed, abandoned>>
Exit == \E u \in Units :
    /\ phase[u] = "Attached" /\ u \notin exited
    /\ exited' = exited \cup {u}
    /\ UNCHANGED <<phase, charged, billed, decided, published, attached, confirmed, abandoned>>
Confirm == \E u \in exited \ abandoned :
    /\ u \notin confirmed /\ confirmed' = confirmed \cup {u}
    /\ UNCHANGED <<phase, charged, billed, decided, published, attached, exited, abandoned>>
ReleaseRoot == \E u \in confirmed \ abandoned :
    /\ phase[u] = "Attached" /\ phase' = [phase EXCEPT ![u] = "Released"]
    /\ charged' = charged \ {u}
    /\ UNCHANGED <<billed, decided, published, attached, exited, confirmed, abandoned>>
Cancel == \E u \in Units :
    /\ phase[u] = "Prepared" /\ u \notin abandoned
    /\ phase' = [phase EXCEPT ![u] = "Released"] /\ charged' = charged \ {u}
    /\ UNCHANGED <<billed, decided, published, attached, exited, confirmed, abandoned>>
Abandon == \E u \in charged \ abandoned :
    /\ abandoned' = abandoned \cup {u}
    /\ UNCHANGED <<phase, charged, billed, decided, published, attached, exited, confirmed>>
Next == Prepare \/ Attach \/ Decide \/ Exit \/ Confirm \/ ReleaseRoot \/ Cancel \/ Abandon
Spec == Init /\ [][Next]_vars

UnsafeIgnoreOrdinary == \E u \in Units :
    /\ Cardinality(charged) < 2 /\ PrepareWith(u)
UnsafeExitRefund == \E u \in exited \cap charged :
    /\ charged' = charged \ {u}
    /\ UNCHANGED <<phase, billed, decided, published, attached, exited, confirmed, abandoned>>
UnsafeDropRefund == \E u \in abandoned \cap charged :
    /\ charged' = charged \ {u}
    /\ UNCHANGED <<phase, billed, decided, published, attached, exited, confirmed, abandoned>>
UnsafeReclassify == \E u \in Ordinary :
    /\ billed' = [billed EXCEPT ![u] = "Platform"]
    /\ UNCHANGED <<phase, charged, decided, published, attached, exited, confirmed, abandoned>>
UnsafeEvict == \E u \in published \cap charged :
    /\ phase' = [phase EXCEPT ![u] = "Released"] /\ charged' = charged \ {u}
    /\ UNCHANGED <<billed, decided, published, attached, exited, confirmed, abandoned>>
UnsafeRootFree == \E u \in exited \cap charged :
    /\ phase[u] = "Attached" /\ u \notin confirmed
    /\ phase' = [phase EXCEPT ![u] = "Released"] /\ charged' = charged \ {u}
    /\ UNCHANGED <<billed, decided, published, attached, exited, confirmed, abandoned>>
OrdinarySpec == Init /\ [][Next \/ UnsafeIgnoreOrdinary]_vars
ExitSpec == Init /\ [][Next \/ UnsafeExitRefund]_vars
DropSpec == Init /\ [][Next \/ UnsafeDropRefund]_vars
ClassSpec == Init /\ [][Next \/ UnsafeReclassify]_vars
EvictSpec == Init /\ [][Next \/ UnsafeEvict]_vars
EarlySpec == Init /\ [][Next \/ UnsafeRootFree]_vars
TypeOK == /\ phase \in [Units -> {"Unused", "Prepared", "Attached", "Record", "Released"}]
          /\ charged \subseteq Units /\ decided \subseteq Units /\ published \subseteq Units
          /\ attached \subseteq Units /\ exited \subseteq attached /\ confirmed \subseteq exited
          /\ abandoned \subseteq Units /\ billed \in [Units -> {"Ordinary", "Platform"}]
Accounted == charged = {u \in Units : phase[u] \in {"Prepared", "Attached", "Record"}}
OrdinaryCeiling == Cardinality(Ordinary) <= 1
TotalCeiling == Cardinality(charged) <= 2
OriginalClass == \A u \in charged : billed[u] = Class(u)
HistoryRetained == \A u \in published : phase[u] = "Record"
RootProof == \A u \in attached : phase[u] = "Released" => u \in confirmed
UniqueRecord == \A u, v \in published : Principal(u) = Principal(v) => u = v
Invariants == TypeOK /\ Accounted /\ OrdinaryCeiling /\ TotalCeiling /\ OriginalClass /\ HistoryRetained /\ RootProof /\ UniqueRecord
=============================================================================
