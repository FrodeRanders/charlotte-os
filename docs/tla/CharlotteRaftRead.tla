------------------------- MODULE CharlotteRaftRead -------------------------
\* Fresh per-read log fences. Quorum/log/election validity is assumed from
\* the other Raft layers. External authority may change without local notice.
EXTENDS Naturals
CONSTANT Unsafe
Max(a, b) == IF a > b THEN a ELSE b
VARIABLES term, leader, changedLeader, domain, epoch, lastIndex, commit, applied,
          contact, delayedIndex, pending, fence, ownerTerm, ownerDomain, ownerEpoch,
          fenceProof, readAccepted, readCurrent, readApplied, readProof, readBound
vars == <<term, leader, changedLeader, domain, epoch, lastIndex, commit, applied,
          contact, delayedIndex, pending, fence, ownerTerm, ownerDomain, ownerEpoch,
          fenceProof, readAccepted, readCurrent, readApplied, readProof, readBound>>

Init ==
    /\ term = 1 /\ leader = FALSE /\ changedLeader = FALSE
    /\ domain = 1 /\ epoch = 1 /\ lastIndex = 1 /\ commit \in 0..1 /\ applied = commit
    /\ contact = FALSE /\ delayedIndex = 0 /\ pending = FALSE /\ fence = 0
    /\ ownerTerm = 0 /\ ownerDomain = 0 /\ ownerEpoch = 0 /\ fenceProof = FALSE
    /\ readAccepted = FALSE /\ readCurrent = TRUE /\ readApplied = TRUE
    /\ readProof = TRUE /\ readBound = TRUE

Elect ==
    /\ term = 1 /\ term' = 2 /\ leader' = TRUE /\ lastIndex' = 2
    /\ UNCHANGED <<changedLeader, domain, epoch, commit, applied, contact, delayedIndex,
                    pending, fence, ownerTerm, ownerDomain, ownerEpoch, fenceProof,
                    readAccepted, readCurrent, readApplied, readProof, readBound>>

AdmitRead ==
    /\ leader /\ term = 2 /\ ~pending /\ lastIndex < 4
    /\ pending' = TRUE /\ fence' = lastIndex + 1 /\ lastIndex' = lastIndex + 1
    /\ ownerTerm' = term /\ ownerDomain' = domain /\ ownerEpoch' = epoch /\ fenceProof' = FALSE
    /\ UNCHANGED <<term, leader, changedLeader, domain, epoch, commit, applied,
                    contact, delayedIndex, readAccepted, readCurrent, readApplied, readProof, readBound>>

\* A quorum has replicated this index while this node still has authority.
\* Capture may precede delivery. Proof for a future read cannot be inherited.
CaptureReply ==
    /\ leader /\ term = 2 /\ ~changedLeader /\ delayedIndex = 0
    /\ \E index \in 2..lastIndex :
        /\ delayedIndex' = index
        /\ fenceProof' = (fenceProof \/ (pending /\ index >= fence))
    /\ UNCHANGED <<term, leader, changedLeader, domain, epoch, lastIndex, commit, applied,
                    contact, pending, fence, ownerTerm, ownerDomain, ownerEpoch,
                    readAccepted, readCurrent, readApplied, readProof, readBound>>

ReceiveOldPrefix ==
    /\ leader /\ term = 2 /\ ~changedLeader /\ contact' = TRUE
    /\ UNCHANGED <<term, leader, changedLeader, domain, epoch, lastIndex, commit, applied,
                    delayedIndex, pending, fence, ownerTerm, ownerDomain, ownerEpoch, fenceProof,
                    readAccepted, readCurrent, readApplied, readProof, readBound>>

CommitQuorum ==
    /\ leader /\ term = 2 /\ ~changedLeader /\ commit < lastIndex
    /\ commit' = lastIndex /\ contact' = TRUE
    /\ fenceProof' = (fenceProof \/ (pending /\ lastIndex >= fence))
    /\ UNCHANGED <<term, leader, changedLeader, domain, epoch, lastIndex, applied,
                    delayedIndex, pending, fence, ownerTerm, ownerDomain, ownerEpoch,
                    readAccepted, readCurrent, readApplied, readProof, readBound>>

DeliverReply ==
    /\ leader /\ term = 2 /\ delayedIndex > 0
    /\ commit' = Max(commit, delayedIndex) /\ delayedIndex' = 0 /\ contact' = TRUE
    /\ UNCHANGED <<term, leader, changedLeader, domain, epoch, lastIndex, applied,
                    pending, fence, ownerTerm, ownerDomain, ownerEpoch, fenceProof,
                    readAccepted, readCurrent, readApplied, readProof, readBound>>

Apply ==
    /\ applied < commit /\ applied' = commit
    /\ UNCHANGED <<term, leader, changedLeader, domain, epoch, lastIndex, commit,
                    contact, delayedIndex, pending, fence, ownerTerm, ownerDomain, ownerEpoch,
                    fenceProof, readAccepted, readCurrent, readApplied, readProof, readBound>>

Bound == term = ownerTerm /\ domain = ownerDomain /\ epoch = ownerEpoch
CurrentCommit == term = 2 /\ commit >= 2
Ready == pending /\ leader /\ Bound /\ CurrentCommit /\ commit >= fence /\ applied = commit

ReadWith(mode) ==
    /\ pending /\ leader
    /\ (mode = "Binding" \/ Bound)
    /\ (mode = "Commit" \/ CurrentCommit)
    /\ (mode \in {"Commit", "Lease"} \/ commit >= fence)
    /\ (mode = "Apply" \/ applied = commit)
    /\ (mode \notin {"Commit", "Lease"} \/ contact)
    /\ pending' = FALSE /\ readAccepted' = TRUE
    /\ readCurrent' = CurrentCommit /\ readApplied' = (applied = commit)
    /\ readProof' = fenceProof /\ readBound' = Bound
    /\ UNCHANGED <<term, leader, changedLeader, domain, epoch, lastIndex, commit, applied,
                    contact, delayedIndex, fence, ownerTerm, ownerDomain, ownerEpoch, fenceProof>>

Read == /\ pending /\ ReadWith("None")
RejectRead == /\ pending /\ ~Ready /\ UNCHANGED vars
CancelRead ==
    /\ pending /\ pending' = FALSE
    /\ UNCHANGED <<term, leader, changedLeader, domain, epoch, lastIndex, commit, applied,
                    contact, delayedIndex, fence, ownerTerm, ownerDomain, ownerEpoch, fenceProof,
                    readAccepted, readCurrent, readApplied, readProof, readBound>>

ChangeLeader ==
    /\ leader /\ ~changedLeader /\ changedLeader' = TRUE /\ contact' = FALSE
    \* The isolated node has not observed the newer leader's term.
    /\ UNCHANGED <<term, leader, domain, epoch, lastIndex, commit, applied,
                    delayedIndex, pending, fence, ownerTerm, ownerDomain, ownerEpoch, fenceProof,
                    readAccepted, readCurrent, readApplied, readProof, readBound>>

StepDown ==
    /\ leader /\ leader' = FALSE /\ term' = 3 /\ contact' = FALSE
    /\ UNCHANGED <<changedLeader, domain, epoch, lastIndex, commit, applied,
                    delayedIndex, pending, fence, ownerTerm, ownerDomain, ownerEpoch, fenceProof,
                    readAccepted, readCurrent, readApplied, readProof, readBound>>

ChangeEpoch ==
    /\ epoch = 1 /\ epoch' = 2
    /\ UNCHANGED <<term, leader, changedLeader, domain, lastIndex, commit, applied,
                    contact, delayedIndex, pending, fence, ownerTerm, ownerDomain, ownerEpoch, fenceProof,
                    readAccepted, readCurrent, readApplied, readProof, readBound>>

ReplaceDomain ==
    /\ domain = 1 /\ domain' = 2
    \* A different node/domain can have identical scalar terms and indices.
    /\ UNCHANGED <<term, leader, changedLeader, epoch, lastIndex, commit, applied,
                    contact, delayedIndex, pending, fence, ownerTerm, ownerDomain, ownerEpoch, fenceProof,
                    readAccepted, readCurrent, readApplied, readProof, readBound>>

UnsafeReadBeforeCommit == /\ Unsafe = "Commit" /\ ReadWith("Commit")
UnsafeReadBeforeApply == /\ Unsafe = "Apply" /\ ReadWith("Apply")
UnsafeReceiveLease == /\ Unsafe = "Lease" /\ changedLeader /\ ReadWith("Lease")
UnsafeReadBinding == /\ Unsafe = "Binding" /\ ReadWith("Binding")
Next == Elect \/ AdmitRead \/ CaptureReply \/ ReceiveOldPrefix \/ CommitQuorum \/ DeliverReply \/ Apply
        \/ Read \/ RejectRead \/ CancelRead \/ ChangeLeader \/ StepDown \/ ChangeEpoch \/ ReplaceDomain
        \/ UnsafeReadBeforeCommit \/ UnsafeReadBeforeApply \/ UnsafeReceiveLease \/ UnsafeReadBinding
Spec == Init /\ [][Next]_vars
TypeOK ==
    /\ term \in 1..3 /\ leader \in BOOLEAN /\ changedLeader \in BOOLEAN
    /\ domain \in 1..2 /\ epoch \in 1..2 /\ lastIndex \in 1..4
    /\ commit \in 0..4 /\ applied \in 0..4 /\ delayedIndex \in 0..4
    /\ contact \in BOOLEAN /\ pending \in BOOLEAN /\ fence \in 0..4
    /\ ownerTerm \in 0..3 /\ ownerDomain \in 0..2 /\ ownerEpoch \in 0..2
    /\ fenceProof \in BOOLEAN /\ readAccepted \in BOOLEAN /\ readCurrent \in BOOLEAN
    /\ readApplied \in BOOLEAN /\ readProof \in BOOLEAN /\ readBound \in BOOLEAN
ReadHasCurrentCommit == readAccepted => readCurrent
ReadHasAppliedCommit == readAccepted => readApplied
ReadHasFenceProof == readAccepted => readProof
ReadBoundToAdmission == readAccepted => readBound
Invariants == TypeOK /\ ReadHasCurrentCommit /\ ReadHasAppliedCommit
              /\ ReadHasFenceProof /\ ReadBoundToAdmission
=============================================================================
