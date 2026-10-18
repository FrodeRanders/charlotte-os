------------------------- MODULE CharlotteRaftRead -------------------------
\* Read prerequisites after an election with an inherited term-one prefix.
\* Quorum contact is an input, not proof of a valid time lease or authority.
EXTENDS Naturals
CONSTANT Unsafe
VARIABLES term, leader, contact, commit, commitTerm, applied,
          readAccepted, readTerm, readCommit, readCommitTerm, readApplied
vars == <<term, leader, contact, commit, commitTerm, applied,
          readAccepted, readTerm, readCommit, readCommitTerm, readApplied>>

Init ==
    /\ term = 1 /\ leader = FALSE /\ contact = FALSE
    /\ commit \in 0..1 /\ commitTerm = (IF commit = 0 THEN 0 ELSE 1)
    /\ applied = commit
    /\ readAccepted = FALSE /\ readTerm = 0 /\ readCommit = 0
    /\ readCommitTerm = 0 /\ readApplied = 0

Elect ==
    /\ term = 1 /\ term' = 2 /\ leader' = TRUE /\ contact' = FALSE
    \* The leader appends its term-two no-op, but has not committed it.
    /\ UNCHANGED <<commit, commitTerm, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied>>

ReceiveOldPrefix ==
    /\ term = 2 /\ leader /\ contact' = TRUE
    \* Exact-term success can acknowledge only the inherited prefix.
    /\ UNCHANGED <<term, leader, commit, commitTerm, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied>>

CommitCurrent ==
    /\ term = 2 /\ leader /\ commit < 2
    \* A qualified current-term quorum acknowledges the election no-op.
    /\ commit' = 2 /\ commitTerm' = 2 /\ contact' = TRUE
    /\ UNCHANGED <<term, leader, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied>>

ApplyCurrent ==
    /\ commit = 2 /\ applied < commit /\ applied' = commit
    /\ UNCHANGED <<term, leader, contact, commit, commitTerm,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied>>

ReadWith(skipCommit, skipApply) ==
    /\ leader /\ contact
    /\ (skipCommit \/ (commit > 0 /\ commitTerm = term))
    /\ (skipApply \/ applied = commit)
    /\ readAccepted' = TRUE /\ readTerm' = term /\ readCommit' = commit
    /\ readCommitTerm' = commitTerm /\ readApplied' = applied
    /\ UNCHANGED <<term, leader, contact, commit, commitTerm, applied>>

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
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied>>

StepDown ==
    /\ leader /\ term' = 3 /\ leader' = FALSE /\ contact' = FALSE
    /\ UNCHANGED <<commit, commitTerm, applied,
                    readAccepted, readTerm, readCommit, readCommitTerm, readApplied>>

Next == Elect \/ ReceiveOldPrefix \/ CommitCurrent \/ ApplyCurrent \/ Read
        \/ RejectBeforeCommit \/ RejectBeforeApply \/ ExpireContact \/ StepDown
        \/ UnsafeReadBeforeCommit \/ UnsafeReadBeforeApply
Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ term \in 1..3 /\ leader \in BOOLEAN /\ contact \in BOOLEAN
    /\ commit \in 0..2 /\ commitTerm \in 0..2 /\ applied \in 0..2
    /\ readAccepted \in BOOLEAN /\ readTerm \in 0..3
    /\ readCommit \in 0..2 /\ readCommitTerm \in 0..2 /\ readApplied \in 0..2
ReadHasCurrentCommit == readAccepted => (readCommit > 0 /\ readCommitTerm = readTerm)
ReadHasAppliedCommit == readAccepted => readApplied = readCommit
Invariants == TypeOK /\ ReadHasCurrentCommit /\ ReadHasAppliedCommit
=============================================================================
