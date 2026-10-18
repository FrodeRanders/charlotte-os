---------------------- MODULE CharlotteSnapshotRestore ----------------------
\* Application acceptance before Raft publication. CharlotteRaftSnapshot covers
\* chunks, suffixes and durable replacement; this model covers the omitted
\* rejection boundary. Actions inside one handler do not imply reader isolation.
\* exposed means this Raft caller can return/continue with its node. It does
\* not assert that thread_exit stops sibling threads or withdraws endpoints.
EXTENDS Naturals

CONSTANT Unsafe
Modes == {"Stream", "Recovery"}
Verdicts == {"Supported", "Unsupported", "Malformed"}
Memberships == {"IncludingSelf", "ExcludingSelf", "JointIncludingSelf"}
Snapshots == [index : 1..2, membership : Memberships]
Old == [index |-> 1, membership |-> "IncludingSelf"]
IncludesSelf(snapshot) == snapshot.membership # "ExcludingSelf"

VARIABLES mode, phase, verdict, candidate, durable, application,
          commitIndex, appliedIndex, membership, joining, exposed,
          validated, restored, replyDone, baseline

vars == <<mode, phase, verdict, candidate, durable, application,
          commitIndex, appliedIndex, membership, joining, exposed,
          validated, restored, replyDone, baseline>>

Protected == [application |-> application, durable |-> durable,
              commitIndex |-> commitIndex, appliedIndex |-> appliedIndex,
              membership |-> membership, joining |-> joining]

Init ==
    /\ mode \in Modes
    /\ verdict \in Verdicts
    /\ candidate \in {s \in Snapshots : s.index = 2}
    /\ durable = IF mode = "Stream" THEN Old ELSE candidate
    /\ application = IF mode = "Stream" THEN 1 ELSE 0
    /\ commitIndex = IF mode = "Stream" THEN 1 ELSE 0
    /\ appliedIndex = commitIndex
    /\ membership = "IncludingSelf"
    \* A durable join admission captured snapshot index 1. Recovery must not
    \* clear it merely because that same old snapshot includes this node.
    /\ joining = TRUE
    /\ exposed = (mode = "Stream")
    /\ validated = FALSE
    /\ restored = FALSE
    /\ replyDone = FALSE
    /\ baseline = Protected
    /\ phase = "Header"

ReadHeader ==
    /\ phase = "Header"
    /\ verdict # "Unsupported"
    /\ phase' = "Sections"
    /\ UNCHANGED <<mode, verdict, candidate, durable, application,
                    commitIndex, appliedIndex, membership, joining, exposed,
                    validated, restored, replyDone, baseline>>

ParseSections ==
    /\ phase = "Sections"
    /\ phase' = "Parsed"
    \* Parsing can build temporary sections, but no live section changes.
    /\ UNCHANGED <<mode, verdict, candidate, durable, application,
                    commitIndex, appliedIndex, membership, joining, exposed,
                    validated, restored, replyDone, baseline>>

Reject ==
    /\ \/ phase = "Header" /\ verdict = "Unsupported"
       \/ phase = "Parsed" /\ verdict = "Malformed"
    /\ phase' = "Rejected"
    /\ UNCHANGED <<mode, verdict, candidate, durable, application,
                    commitIndex, appliedIndex, membership, joining, exposed,
                    validated, restored, replyDone, baseline>>

RejectUnsupported == /\ verdict = "Unsupported" /\ Reject
RejectMalformed == /\ verdict = "Malformed" /\ Reject

Validate ==
    /\ phase = "Parsed"
    /\ verdict = "Supported"
    /\ phase' = "Validated"
    /\ validated' = TRUE
    /\ UNCHANGED <<mode, verdict, candidate, durable, application,
                    commitIndex, appliedIndex, membership, joining, exposed,
                    restored, replyDone, baseline>>

Restore ==
    /\ phase = "Validated"
    /\ validated
    /\ application' = candidate.index
    /\ restored' = TRUE
    /\ phase' = "Restored"
    /\ UNCHANGED <<mode, verdict, candidate, durable,
                    commitIndex, appliedIndex, membership, joining, exposed,
                    validated, replyDone, baseline>>

Persist ==
    /\ mode = "Stream"
    /\ phase = "Restored"
    /\ restored
    /\ durable' = candidate
    /\ phase' = "Persisted"
    /\ UNCHANGED <<mode, verdict, candidate, application,
                    commitIndex, appliedIndex, membership, joining, exposed,
                    validated, restored, replyDone, baseline>>

PersistenceFailure(committed) ==
    /\ mode = "Stream"
    /\ phase = "Restored"
    /\ durable' = IF committed THEN candidate ELSE durable
    \* DiskLogStore terminates the calling thread. Restore can precede a failed
    \* write; this is not an application+storage atomic transaction. The
    \* old/new whole-object choice assumes the existing object-store contract.
    /\ phase' = "Stopped"
    /\ exposed' = FALSE
    /\ UNCHANGED <<mode, verdict, candidate, application,
                    commitIndex, appliedIndex, membership, joining,
                    validated, restored, replyDone, baseline>>

DefinitePersistenceFailure == /\ mode = "Stream" /\ PersistenceFailure(FALSE)
AmbiguousPersistenceFailure == /\ mode = "Stream" /\ PersistenceFailure(TRUE)

Activate ==
    /\ \/ mode = "Stream" /\ phase = "Persisted"
       \/ mode = "Recovery" /\ phase = "Restored"
    /\ restored
    /\ phase' = "Done"
    /\ commitIndex' = durable.index
    /\ appliedIndex' = durable.index
    /\ membership' = durable.membership
    /\ joining' = (joining /\ ~(durable.index > 1 /\ IncludesSelf(durable)))
    /\ exposed' = TRUE
    /\ replyDone' = (mode = "Stream")
    /\ UNCHANGED <<mode, verdict, candidate, durable, application,
                    validated, restored, baseline>>

ActivateStreamJoin ==
    /\ mode = "Stream" /\ joining /\ IncludesSelf(durable) /\ Activate
ActivateStreamRetained ==
    /\ mode = "Stream" /\ ~IncludesSelf(durable) /\ Activate
ActivateRecoveryJoin ==
    /\ mode = "Recovery" /\ joining
    /\ durable.index > 1 /\ IncludesSelf(durable) /\ Activate
ActivateRecoveryRetained ==
    /\ mode = "Recovery"
    /\ (~joining \/ durable.index = 1 \/ ~IncludesSelf(durable)) /\ Activate

Crash ==
    /\ phase \notin {"Stopped", "Rejected"}
    /\ phase' = "Stopped"
    /\ exposed' = FALSE
    /\ replyDone' = FALSE
    /\ UNCHANGED <<mode, verdict, candidate, durable, application,
                    commitIndex, appliedIndex, membership, joining,
                    validated, restored, baseline>>

BeginRecovery ==
    /\ phase = "Stopped"
    /\ mode' = "Recovery"
    /\ phase' = "Header"
    \* A new decoder/context may accept or reject the persisted application.
    \* This abstracts validation, not spontaneous durable corruption.
    /\ verdict' \in Verdicts
    /\ candidate' = durable
    /\ application' = 0
    /\ commitIndex' = 0
    /\ appliedIndex' = 0
    /\ membership' = "IncludingSelf"
    /\ exposed' = FALSE
    /\ validated' = FALSE
    /\ restored' = FALSE
    /\ replyDone' = FALSE
    /\ baseline' = [application |-> 0, durable |-> durable,
                     commitIndex |-> 0, appliedIndex |-> 0,
                     membership |-> "IncludingSelf", joining |-> joining]
    /\ UNCHANGED <<durable, joining>>

\* Each control weakens exactly one rejected-publication boundary.
UnsafeRejectApplication ==
    /\ Unsafe = "Application" /\ phase = "Rejected"
    /\ application' = candidate.index
    /\ UNCHANGED <<mode, phase, verdict, candidate, durable,
                    commitIndex, appliedIndex, membership, joining, exposed,
                    validated, restored, replyDone, baseline>>

UnsafeRejectDurable ==
    /\ Unsafe = "Durable" /\ phase = "Rejected" /\ mode = "Stream"
    /\ durable' = candidate
    /\ UNCHANGED <<mode, phase, verdict, candidate, application,
                    commitIndex, appliedIndex, membership, joining, exposed,
                    validated, restored, replyDone, baseline>>

UnsafeRejectProgress ==
    /\ Unsafe = "Progress" /\ phase = "Rejected"
    /\ commitIndex' = candidate.index /\ appliedIndex' = candidate.index
    /\ UNCHANGED <<mode, phase, verdict, candidate, durable, application,
                    membership, joining, exposed, validated, restored,
                    replyDone, baseline>>

UnsafeRejectMembership ==
    /\ Unsafe = "Membership" /\ phase = "Rejected"
    /\ membership' = "ExcludingSelf"
    /\ UNCHANGED <<mode, phase, verdict, candidate, durable, application,
                    commitIndex, appliedIndex, joining, exposed,
                    validated, restored, replyDone, baseline>>

UnsafeRejectJoin ==
    /\ Unsafe = "Join" /\ phase = "Rejected"
    /\ joining' = FALSE
    /\ UNCHANGED <<mode, phase, verdict, candidate, durable, application,
                    commitIndex, appliedIndex, membership, exposed,
                    validated, restored, replyDone, baseline>>

UnsafeExposeRecovery ==
    /\ Unsafe = "Exposure" /\ phase = "Rejected" /\ mode = "Recovery"
    /\ exposed' = TRUE
    /\ UNCHANGED <<mode, phase, verdict, candidate, durable, application,
                    commitIndex, appliedIndex, membership, joining,
                    validated, restored, replyDone, baseline>>

UnsafePublishBeforeRestore ==
    /\ Unsafe = "Ordering" /\ phase = "Validated" /\ mode = "Stream"
    /\ durable' = candidate /\ phase' = "Persisted"
    /\ UNCHANGED <<mode, verdict, candidate, application,
                    commitIndex, appliedIndex, membership, joining, exposed,
                    validated, restored, replyDone, baseline>>

Next == ReadHeader \/ ParseSections \/ RejectUnsupported \/ RejectMalformed
        \/ Validate \/ Restore
        \/ Persist \/ DefinitePersistenceFailure \/ AmbiguousPersistenceFailure
        \/ ActivateStreamJoin \/ ActivateStreamRetained
        \/ ActivateRecoveryJoin \/ ActivateRecoveryRetained
        \/ Crash \/ BeginRecovery
        \/ UnsafeRejectApplication \/ UnsafeRejectDurable
        \/ UnsafeRejectProgress \/ UnsafeRejectMembership \/ UnsafeRejectJoin
        \/ UnsafeExposeRecovery \/ UnsafePublishBeforeRestore

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ mode \in Modes
    /\ phase \in {"Header", "Sections", "Parsed", "Validated", "Restored",
                   "Persisted", "Done", "Rejected", "Stopped"}
    /\ verdict \in Verdicts
    /\ candidate \in Snapshots /\ durable \in Snapshots
    /\ application \in 0..2 /\ commitIndex \in 0..2 /\ appliedIndex \in 0..2
    /\ membership \in Memberships
    /\ joining \in BOOLEAN /\ exposed \in BOOLEAN
    /\ validated \in BOOLEAN /\ restored \in BOOLEAN /\ replyDone \in BOOLEAN
    /\ baseline \in [application : 0..2, durable : Snapshots,
                       commitIndex : 0..2, appliedIndex : 0..2,
                       membership : Memberships, joining : BOOLEAN]

RejectionPreservesApplication ==
    phase = "Rejected" => application = baseline.application
RejectionPreservesDurable ==
    phase = "Rejected" => durable = baseline.durable
RejectionPreservesProgress ==
    phase = "Rejected" => /\ commitIndex = baseline.commitIndex
                           /\ appliedIndex = baseline.appliedIndex
RejectionPreservesMembership ==
    phase = "Rejected" => membership = baseline.membership
RejectionPreservesJoin ==
    phase = "Rejected" => joining = baseline.joining
RecoveryExposureRequiresRestore ==
    mode = "Recovery" /\ exposed => restored /\ phase = "Done"
PublicationRequiresRestore ==
    mode = "Stream" /\ durable # baseline.durable => restored
CompletedConsistent ==
    phase = "Done" => /\ restored /\ validated
                       /\ application = durable.index
                       /\ commitIndex = durable.index
                       /\ appliedIndex = durable.index
                       /\ membership = durable.membership
                       /\ joining = (baseline.joining /\
                           ~(durable.index > 1 /\ IncludesSelf(durable)))
ReplyRequiresActivation == replyDone => phase = "Done" /\ mode = "Stream"
StoppedUnexposed == phase = "Stopped" => ~exposed /\ ~replyDone

Invariants == TypeOK /\ RejectionPreservesApplication /\ RejectionPreservesDurable
              /\ RejectionPreservesProgress /\ RejectionPreservesMembership
              /\ RejectionPreservesJoin /\ RecoveryExposureRequiresRestore
              /\ PublicationRequiresRestore /\ CompletedConsistent
              /\ ReplyRequiresActivation /\ StoppedUnexposed
=============================================================================
