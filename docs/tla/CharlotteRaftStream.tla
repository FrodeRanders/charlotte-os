------------------------- MODULE CharlotteRaftStream -------------------------
\* Snapshot continuation ownership. Boundary index/term are fixed and equal
\* across senders, so they cannot accidentally supply the missing identity.
EXTENDS Naturals, Sequences, FiniteSets

CONSTANTS Leader, NoLeader, AllowUnboundContinuation, AllowWrongOffset
ASSUME Cardinality(Leader) = 2
ASSUME NoLeader \notin Leader

Owner == [leader : Leader, term : 1..2]
Chunk == [owner : Owner, offset : 0..2]
NoOwner == [leader |-> NoLeader, term |-> 0]

VARIABLES term, leader, pendingOwner, buffered, installed,
          rejectedStale, rejectedLeader, rejectedOffset, rejectedBoundary,
          restarted
vars == <<term, leader, pendingOwner, buffered, installed,
          rejectedStale, rejectedLeader, rejectedOffset, rejectedBoundary,
          restarted>>

Init ==
    /\ term = 1
    /\ leader \in Leader
    /\ pendingOwner = NoOwner
    /\ buffered = <<>>
    /\ installed = <<>>
    /\ rejectedStale = FALSE
    /\ rejectedLeader = FALSE
    /\ rejectedOffset = FALSE
    /\ rejectedBoundary = FALSE
    /\ restarted = FALSE

CurrentOwner == [leader |-> leader, term |-> term]

BeginReceive ==
    /\ leader \in Leader
    /\ installed = <<>>
    /\ buffered = <<>>
    /\ pendingOwner' = CurrentOwner
    /\ buffered' = <<[owner |-> CurrentOwner, offset |-> 0]>>
    /\ UNCHANGED <<term, leader, installed, rejectedStale, rejectedLeader,
                    rejectedOffset, rejectedBoundary, restarted>>

RestartOffsetZero ==
    /\ leader \in Leader
    /\ Len(buffered) = 1
    /\ pendingOwner' = CurrentOwner
    /\ buffered' = <<[owner |-> CurrentOwner, offset |-> 0]>>
    /\ restarted' = TRUE
    /\ UNCHANGED <<term, leader, installed, rejectedStale, rejectedLeader,
                    rejectedOffset, rejectedBoundary>>

ChangeTerm(nextLeader) ==
    /\ term = 1
    /\ nextLeader \in Leader \cup {NoLeader}
    /\ term' = 2
    /\ leader' = nextLeader
    /\ IF AllowUnboundContinuation
       THEN UNCHANGED <<pendingOwner, buffered>>
       ELSE /\ pendingOwner' = NoOwner
            /\ buffered' = <<>>
    /\ UNCHANGED <<installed, rejectedStale, rejectedLeader,
                    rejectedOffset, rejectedBoundary, restarted>>

RecognizeLeader(nextLeader) ==
    /\ leader = NoLeader
    /\ nextLeader \in Leader
    /\ leader' = nextLeader
    /\ UNCHANGED <<term, pendingOwner, buffered, installed,
                    rejectedStale, rejectedLeader, rejectedOffset,
                    rejectedBoundary, restarted>>

ReceiveChunk(offset) ==
    /\ leader \in Leader
    /\ Len(buffered) = 1
    /\ AllowUnboundContinuation \/ pendingOwner = CurrentOwner
    /\ offset \in 1..2
    /\ AllowWrongOffset \/ offset = Len(buffered)
    /\ installed' = Append(buffered, [owner |-> CurrentOwner, offset |-> offset])
    /\ buffered' = <<>>
    /\ pendingOwner' = NoOwner
    /\ UNCHANGED <<term, leader, rejectedStale, rejectedLeader,
                    rejectedOffset, rejectedBoundary, restarted>>

CancelReceive ==
    /\ Len(buffered) = 1
    /\ buffered' = <<>>
    /\ pendingOwner' = NoOwner
    /\ UNCHANGED <<term, leader, installed, rejectedStale, rejectedLeader,
                    rejectedOffset, rejectedBoundary, restarted>>

\* Invalid traffic preserves the accepted stream. These separate observations
\* ensure rejection paths are exercised rather than hidden behind valid chunks.
RejectStale ==
    /\ term = 2
    /\ Len(buffered) = 1
    /\ rejectedStale' = TRUE
    /\ UNCHANGED <<term, leader, pendingOwner, buffered, installed,
                    rejectedLeader, rejectedOffset, rejectedBoundary, restarted>>

RejectLeader ==
    /\ leader \in Leader
    /\ Len(buffered) = 1
    /\ rejectedLeader' = TRUE
    /\ UNCHANGED <<term, leader, pendingOwner, buffered, installed,
                    rejectedStale, rejectedOffset, rejectedBoundary, restarted>>

RejectOffset ==
    /\ Len(buffered) = 1
    /\ rejectedOffset' = TRUE
    /\ UNCHANGED <<term, leader, pendingOwner, buffered, installed,
                    rejectedStale, rejectedLeader, rejectedBoundary, restarted>>

RejectBoundary ==
    /\ Len(buffered) = 1
    /\ rejectedBoundary' = TRUE
    /\ UNCHANGED <<term, leader, pendingOwner, buffered, installed,
                    rejectedStale, rejectedLeader, rejectedOffset, restarted>>

Next ==
    \/ BeginReceive
    \/ RestartOffsetZero
    \/ \E nextLeader \in Leader \cup {NoLeader} : ChangeTerm(nextLeader)
    \/ \E nextLeader \in Leader : RecognizeLeader(nextLeader)
    \/ \E offset \in 1..2 : ReceiveChunk(offset)
    \/ CancelReceive
    \/ RejectStale
    \/ RejectLeader
    \/ RejectOffset
    \/ RejectBoundary

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ term \in 1..2
    /\ leader \in Leader \cup {NoLeader}
    /\ pendingOwner \in Owner \cup {NoOwner}
    /\ buffered \in Seq(Chunk)
    /\ Len(buffered) <= 1
    /\ installed \in Seq(Chunk)
    /\ Len(installed) \in {0, 2}
    /\ rejectedStale \in BOOLEAN
    /\ rejectedLeader \in BOOLEAN
    /\ rejectedOffset \in BOOLEAN
    /\ rejectedBoundary \in BOOLEAN
    /\ restarted \in BOOLEAN

BufferBoundToLeader ==
    Len(buffered) = 1 =>
        /\ pendingOwner = CurrentOwner
        /\ buffered[1].owner = pendingOwner

InstalledSingleOwner ==
    Len(installed) = 2 => installed[1].owner = installed[2].owner

InstalledInOrder ==
    Len(installed) = 2 => <<installed[1].offset, installed[2].offset>> = <<0, 1>>

=============================================================================
