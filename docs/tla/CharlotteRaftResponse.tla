----------------------- MODULE CharlotteRaftResponse -----------------------
\* Delayed acknowledgements across a replaced log prefix. Election and log
\* selection are assumed from the separate Raft layers, not proved here.
EXTENDS Naturals
CONSTANT Unsafe
Peer == {"Voter", "Learner", "Joiner"}
Zero == [p \in Peer |-> 0]
One == [p \in Peer |-> 1]
Max(a, b) == IF a > b THEN a ELSE b

VARIABLES term, leader, oldReply, actual, matched, nextIndex, confirmed,
          contact, contactProof, committed
vars == <<term, leader, oldReply, actual, matched, nextIndex, confirmed,
          contact, contactProof, committed>>

Init ==
    /\ term = 1 /\ leader = TRUE /\ oldReply = FALSE
    /\ actual = Zero /\ matched = Zero /\ nextIndex = One
    /\ confirmed = Zero /\ contact = FALSE /\ contactProof = FALSE /\ committed = FALSE

CaptureOldReply ==
    /\ term = 1 /\ ~oldReply
    \* An honest voter has acknowledged index three in the original prefix.
    /\ oldReply' = TRUE
    /\ UNCHANGED <<term, leader, actual, matched, nextIndex, confirmed,
                    contact, contactProof, committed>>

ReplacePrefix ==
    /\ term = 1 /\ oldReply
    \* Another elected leader installs a different two-entry term-two prefix.
    /\ term' = 2 /\ leader' = FALSE
    /\ UNCHANGED <<oldReply, actual, matched, nextIndex, confirmed,
                    contact, contactProof, committed>>

Reelect ==
    /\ term = 2
    \* This node wins term three with that newer prefix, then appends a no-op
    \* at index three. The delayed old reply still describes different bytes.
    /\ term' = 3 /\ leader' = TRUE
    /\ actual' = Zero /\ matched' = Zero /\ nextIndex' = [p \in Peer |-> 3]
    /\ confirmed' = Zero /\ contact' = FALSE /\ contactProof' = FALSE
    /\ UNCHANGED <<oldReply, committed>>

Replicate(p, index) ==
    /\ term = 3 /\ leader /\ index \in 1..3
    /\ actual' = [actual EXCEPT ![p] = Max(@, index)]
    /\ UNCHANGED <<term, leader, oldReply, matched, nextIndex, confirmed,
                    contact, contactProof, committed>>

Acknowledge(p, index) ==
    /\ term = 3 /\ leader /\ index \in 0..actual[p]
    \* Delivery may be reordered after a stronger response. Append and
    \* completed-snapshot successes share this progress update in Rust.
    /\ matched' = [matched EXCEPT ![p] = Max(@, index)]
    /\ nextIndex' = [nextIndex EXCEPT ![p] = Max(@, matched'[p] + 1)]
    /\ confirmed' = [confirmed EXCEPT ![p] = Max(@, index)]
    /\ contact' = (contact \/ (p = "Voter"))
    /\ contactProof' = (contactProof \/ (p = "Voter"))
    /\ UNCHANGED <<term, leader, oldReply, actual, committed>>

Failure(p) ==
    /\ term = 3 /\ leader
    /\ nextIndex' = [nextIndex EXCEPT ![p] = Max(@ - 1, matched[p] + 1)]
    /\ UNCHANGED <<term, leader, oldReply, actual, matched, confirmed,
                    contact, contactProof, committed>>

RejectOld ==
    /\ term = 3 /\ leader /\ oldReply
    /\ oldReply' = FALSE
    /\ UNCHANGED <<term, leader, actual, matched, nextIndex, confirmed,
                    contact, contactProof, committed>>

RejectInvalid ==
    /\ term = 3 /\ leader
    \* Mismatched/self/removed source, or a success beyond the local tail.
    \* The caller's clock can advance in Rust; it is outside this tuple.
    /\ UNCHANGED vars

Commit ==
    /\ term = 3 /\ leader /\ matched["Voter"] = 3
    \* Self plus the other voter is the quorum in this fixed configuration.
    /\ committed' = TRUE
    /\ UNCHANGED <<term, leader, oldReply, actual, matched, nextIndex,
                    confirmed, contact, contactProof>>

ObserveHigherTerm ==
    /\ term = 3 /\ leader
    /\ term' = 4 /\ leader' = FALSE /\ contact' = FALSE /\ contactProof' = FALSE
    /\ UNCHANGED <<oldReply, actual, matched, nextIndex, confirmed,
                    committed>>

UnsafeOldTermAck ==
    /\ Unsafe = "OldTerm" /\ term = 3 /\ leader /\ oldReply
    /\ matched' = [matched EXCEPT !["Voter"] = 3]
    /\ nextIndex' = [nextIndex EXCEPT !["Voter"] = 4]
    /\ contact' = TRUE /\ oldReply' = FALSE
    /\ UNCHANGED <<term, leader, actual, confirmed, contactProof, committed>>

UnsafeRegressSuccess(p) ==
    /\ Unsafe = "Regress" /\ term = 3 /\ leader /\ matched[p] > 0
    /\ matched' = [matched EXCEPT ![p] = 0]
    /\ nextIndex' = [nextIndex EXCEPT ![p] = 1]
    /\ UNCHANGED <<term, leader, oldReply, actual, confirmed,
                    contact, contactProof, committed>>

UnsafeBackoff(p) ==
    /\ Unsafe = "Backoff" /\ term = 3 /\ leader /\ nextIndex[p] > 1
    /\ nextIndex' = [nextIndex EXCEPT ![p] = @ - 1]
    /\ UNCHANGED <<term, leader, oldReply, actual, matched, confirmed,
                    contact, contactProof, committed>>

UnsafeInvalidCredit ==
    /\ Unsafe = "Invalid" /\ term = 3 /\ leader
    /\ actual["Voter"] < 3
    \* Credit an unqualified response to the voter without prefix evidence.
    /\ matched' = [matched EXCEPT !["Voter"] = 3]
    /\ nextIndex' = [nextIndex EXCEPT !["Voter"] = 4]
    /\ UNCHANGED <<term, leader, oldReply, actual,
                    confirmed, contact, contactProof, committed>>

Next == CaptureOldReply \/ ReplacePrefix \/ Reelect
        \/ (\E p \in Peer, index \in 1..3 : Replicate(p, index))
        \/ (\E p \in Peer, index \in 0..3 : Acknowledge(p, index))
        \/ (\E p \in Peer : Failure(p))
        \/ RejectOld \/ RejectInvalid \/ Commit \/ ObserveHigherTerm
        \/ UnsafeOldTermAck \/ UnsafeInvalidCredit
        \/ (\E p \in Peer : UnsafeRegressSuccess(p))
        \/ (\E p \in Peer : UnsafeBackoff(p))
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ term \in 1..4 /\ leader \in BOOLEAN /\ oldReply \in BOOLEAN
    /\ actual \in [Peer -> 0..3] /\ matched \in [Peer -> 0..3]
    /\ nextIndex \in [Peer -> 1..4] /\ confirmed \in [Peer -> 0..3]
    /\ contact \in BOOLEAN /\ contactProof \in BOOLEAN /\ committed \in BOOLEAN
AcknowledgementsHaveProof == \A p \in Peer : matched[p] <= actual[p]
ConfirmedProgressRetained == \A p \in Peer : matched[p] >= confirmed[p]
BackoffRespectsConfirmedPrefix == \A p \in Peer : nextIndex[p] > matched[p]
CommitHasCurrentPrefix == committed => actual["Voter"] = 3
ContactHasCurrentResponse == contact => contactProof
Invariants == TypeOK /\ AcknowledgementsHaveProof /\ ConfirmedProgressRetained
              /\ BackoffRespectsConfirmedPrefix /\ CommitHasCurrentPrefix
              /\ ContactHasCurrentResponse
=============================================================================
