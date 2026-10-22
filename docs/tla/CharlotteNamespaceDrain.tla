--------------------- MODULE CharlotteNamespaceDrain ---------------------
\* One poll of an already-fenced namespace, with nonreused token identities.
\* Peer completion assumes its own typed cleanup proof. Root/loan/backing
\* ownership and physical completion belong to the enclosing retirement model.
EXTENDS Naturals, FiniteSets
CONSTANT Tokens
ASSUME Tokens # {} /\ 0 \notin Tokens
VARIABLES tokens, claimed, selected, phase, qualified
vars == <<tokens, claimed, selected, phase, qualified>>

Init ==
    /\ tokens = Tokens /\ claimed \in SUBSET Tokens
    /\ selected = 0 /\ phase = "Select" /\ qualified = TRUE

Select ==
    /\ phase = "Select" /\ tokens # {}
    /\ selected' \in tokens /\ phase' = "Selected"
    /\ UNCHANGED <<tokens, claimed, qualified>>
SelectEmpty ==
    /\ phase = "Select" /\ tokens = {}
    /\ phase' = "Done"
    /\ UNCHANGED <<tokens, claimed, selected, qualified>>

\* Actual peer close can consume a loan-free token between selection and
\* preparation. A held claim needs its original owner's completion instead.
ClosePeer ==
    /\ phase = "Selected" /\ selected \in tokens \ claimed
    /\ tokens' = tokens \ {selected}
    /\ UNCHANGED <<claimed, selected, phase, qualified>>
FinishClaim ==
    /\ phase = "Selected" /\ selected \in claimed
    /\ tokens' = tokens \ {selected} /\ claimed' = claimed \ {selected}
    /\ UNCHANGED <<selected, phase, qualified>>
CancelSelected ==
    /\ phase = "Selected" /\ selected \in tokens \ claimed
    /\ tokens' = tokens \ {selected} /\ selected' = 0 /\ phase' = "Select"
    /\ UNCHANGED <<claimed, qualified>>

\* Pending may also result from revalidation or peer-admission rejection.
RejectPreparation ==
    /\ phase = "Selected" /\ phase' = "Recheck"
    /\ UNCHANGED <<tokens, claimed, selected, qualified>>
RecheckMissing ==
    /\ phase = "Recheck" /\ selected \notin tokens
    /\ selected' = 0 /\ phase' = "Select"
    /\ UNCHANGED <<tokens, claimed, qualified>>
ReturnPending ==
    /\ phase = "Recheck" /\ selected \in tokens
    /\ phase' = "Pending" /\ qualified' = TRUE
    /\ UNCHANGED <<tokens, claimed, selected>>

UnsafeStalePending ==
    /\ phase = "Recheck" /\ selected \notin tokens
    /\ phase' = "Pending" /\ qualified' = FALSE
    /\ UNCHANGED <<tokens, claimed, selected>>
UnsafeSkipLive ==
    /\ phase = "Recheck" /\ selected \in tokens
    /\ phase' = "Done"
    /\ UNCHANGED <<tokens, claimed, selected, qualified>>

Next == Select \/ SelectEmpty \/ ClosePeer \/ FinishClaim \/ CancelSelected \/
        RejectPreparation \/ RecheckMissing \/ ReturnPending
Spec == Init /\ [][Next]_vars
StaleSpec == Init /\ [][Next \/ UnsafeStalePending]_vars
SkipSpec == Init /\ [][Next \/ UnsafeSkipLive]_vars
TypeOK ==
    /\ tokens \subseteq Tokens /\ claimed \subseteq tokens
    /\ selected \in Tokens \union {0}
    /\ phase \in {"Select", "Selected", "Recheck", "Pending", "Done"}
    /\ qualified \in BOOLEAN
NoStalePending == phase = "Pending" => qualified
CompleteOnlyDrained == phase = "Done" => tokens = {}
Invariants == TypeOK /\ NoStalePending /\ CompleteOnlyDrained
=============================================================================
