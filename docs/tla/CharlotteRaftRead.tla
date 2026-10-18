------------------------- MODULE CharlotteRaftRead -------------------------
\* Read prerequisites after an election with an inherited term-one prefix.
\* Quorum contact is an input, not proof of a valid time lease or authority.
EXTENDS Naturals
CONSTANT Unsafe
\* DelayedLease enables an implemented receive-time contact counterexample.
VARIABLES term, leader, contact, commit, commitTerm, applied,
          readAccepted, readTerm, readCommit, readCommitTerm, readApplied, changedLeader, delayedContact, readAuthority
vars == <<term, leader, contact, commit, commitTerm, applied,
          readAccepted, readTerm, readCommit, readCommitTerm, readApplied, changedLeader, delayedContact, readAuthority>>

Init ==
    /\ changedLeader = FALSE /\ delayedContact = FALSE /\ readAuthority = TRUE
    /\ term = 1 /\ leader = FALSE /\ contact = FALSE
    /\ commit \in 0..1 /\ commitTerm = (IF commit = 0 THEN 0 ELSE 1)
    /\ applied = commit
    /\ readAccepted = FALSE /\ readTerm = 0 /\ readCommit = 0
    /\ readCommitTerm = 0 /\ readApplied = 0

Elect ==
    /\ term = 1 /\ term' = 2 /\ leader' = TRUE /\ contact' = FALSE
    \* The leader appends its term-two no-op, but has not committed it.
    /\ UNCHANGED <<commit, commitTerm, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied, changedLeader, delayedContact, readAuthority>>

ReceiveOldPrefix ==
    /\ term = 2 /\ leader /\ ~changedLeader /\ contact' = TRUE
    \* Exact-term success can acknowledge only the inherited prefix.
    /\ UNCHANGED <<term, leader, commit, commitTerm, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied, changedLeader, delayedContact, readAuthority>>

CommitCurrent ==
    /\ term = 2 /\ leader /\ commit < 2
    \* A qualified current-term quorum acknowledges the election no-op.
    /\ commit' = 2 /\ commitTerm' = 2 /\ contact' = TRUE
    /\ UNCHANGED <<term, leader, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied, changedLeader, delayedContact, readAuthority>>

ApplyCurrent ==
    /\ commit = 2 /\ applied < commit /\ applied' = commit
    /\ UNCHANGED <<term, leader, contact, commit, commitTerm,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied, changedLeader, delayedContact, readAuthority>>

ReadWith(skipCommit, skipApply) ==
    /\ leader /\ contact
    /\ (skipCommit \/ (commit > 0 /\ commitTerm = term))
    /\ (skipApply \/ applied = commit)
    /\ readAccepted' = TRUE /\ readTerm' = term /\ readCommit' = commit
    /\ readCommitTerm' = commitTerm /\ readApplied' = applied
    /\ readAuthority' = ~changedLeader
    /\ UNCHANGED <<term, leader, contact, commit, commitTerm, applied,
                    changedLeader, delayedContact>>

Read == /\ leader /\ ReadWith(FALSE, FALSE)
UnsafeReadBeforeCommit ==
    /\ Unsafe = "Commit" /\ ReadWith(TRUE, FALSE)
UnsafeReadBeforeApply ==
    /\ Unsafe = "Apply" /\ ReadWith(FALSE, TRUE)

RejectBeforeCommit ==
    /\ leader /\ contact /\ commitTerm # term /\ UNCHANGED vars
RejectBeforeApply ==
    /\ leader /\ contact /\ commitTerm = term /\ applied < commit /\ UNCHANGED vars

ExpireContact ==
    /\ contact /\ contact' = FALSE
    /\ UNCHANGED <<term, leader, commit, commitTerm, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied, changedLeader, delayedContact, readAuthority>>

StepDown ==
    /\ leader /\ term' = 3 /\ leader' = FALSE /\ contact' = FALSE
    /\ UNCHANGED <<commit, commitTerm, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied, changedLeader, delayedContact, readAuthority>>

\* Honest response created before a different quorum elects a new leader.
CaptureDelayedContact ==
    /\ Unsafe = "DelayedLease" /\ leader /\ commit = 2 /\ applied = 2
    /\ ~changedLeader /\ ~delayedContact /\ delayedContact' = TRUE
    /\ UNCHANGED <<term, leader, contact, commit, commitTerm, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied,
                    changedLeader, readAuthority>>

ElectOtherLeader ==
    /\ Unsafe = "DelayedLease" /\ delayedContact /\ ~changedLeader
    \* The old node remains unaware; its local term/role do not change.
    /\ changedLeader' = TRUE /\ contact' = FALSE
    /\ UNCHANGED <<term, leader, commit, commitTerm, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied,
                    delayedContact, readAuthority>>

DeliverDelayedContact ==
    /\ Unsafe = "DelayedLease" /\ changedLeader /\ delayedContact
    /\ leader /\ delayedContact' = FALSE /\ contact' = TRUE
    /\ UNCHANGED <<term, leader, commit, commitTerm, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied,
                    changedLeader, readAuthority>>

Next == Elect \/ ReceiveOldPrefix \/ CommitCurrent \/ ApplyCurrent \/ Read
        \/ RejectBeforeCommit \/ RejectBeforeApply \/ ExpireContact \/ StepDown
        \/ UnsafeReadBeforeCommit \/ UnsafeReadBeforeApply
        \/ CaptureDelayedContact \/ ElectOtherLeader \/ DeliverDelayedContact
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ changedLeader \in BOOLEAN /\ delayedContact \in BOOLEAN /\ readAuthority \in BOOLEAN
    /\ term \in 1..3 /\ leader \in BOOLEAN /\ contact \in BOOLEAN
    /\ commit \in 0..2 /\ commitTerm \in 0..2 /\ applied \in 0..2
    /\ readAccepted \in BOOLEAN /\ readTerm \in 0..3
    /\ readCommit \in 0..2 /\ readCommitTerm \in 0..2 /\ readApplied \in 0..2
ReadHasCurrentCommit == readAccepted => (readCommit > 0 /\ readCommitTerm = readTerm)
ReadHasAppliedCommit == readAccepted => readApplied = readCommit
\* Diagnostic only: current source does not establish this stronger property.
ReadHasAuthority == readAccepted => readAuthority
Invariants == TypeOK /\ ReadHasCurrentCommit /\ ReadHasAppliedCommit
=============================================================================
