------------------------ MODULE CharlotteRaftSnapshot ------------------------
\* Crash consistency of chunked snapshot reception, atomic durable replacement,
\* suffix retention, activation, and restart recovery.

EXTENDS Naturals, Sequences

CONSTANTS Value, NoValue, MaxIndex, MaxChunks

ASSUME NoValue \notin Value
ASSUME MaxIndex > 0
ASSUME MaxChunks > 0

Index == 0..MaxIndex
SnapshotValue == Value \cup {NoValue}
Entry == [index : 1..MaxIndex, term : 1..MaxIndex, value : Value]

VARIABLES up,
          log,
          commitIndex,
          pendingIndex,
          pendingTerm,
          pendingValue,
          pendingChunks,
          durableIndex,
          durableTerm,
          durableValue,
          appliedIndex,
          appliedValue

vars == <<up, log, commitIndex,
          pendingIndex, pendingTerm, pendingValue, pendingChunks,
          durableIndex, durableTerm, durableValue,
          appliedIndex, appliedValue>>

Init ==
    /\ up = TRUE
    /\ log = <<>>
    /\ commitIndex = 0
    /\ pendingIndex = 0
    /\ pendingTerm = 0
    /\ pendingValue = NoValue
    /\ pendingChunks = 0
    /\ durableIndex = 0
    /\ durableTerm = 0
    /\ durableValue = NoValue
    /\ appliedIndex = 0
    /\ appliedValue = NoValue

LastIndex == IF Len(log) = 0 THEN durableIndex ELSE log[Len(log)].index

AppendLog(term, value) ==
    /\ up
    /\ LastIndex < MaxIndex
    /\ term \in 1..MaxIndex
    /\ value \in Value
    /\ log' = Append(log,
           [index |-> LastIndex + 1, term |-> term, value |-> value])
    /\ UNCHANGED <<commitIndex,
                    pendingIndex, pendingTerm, pendingValue, pendingChunks,
                    durableIndex, durableTerm, durableValue,
                    appliedIndex, appliedValue, up>>

Commit(index) ==
    /\ up
    /\ index \in (commitIndex + 1)..LastIndex
    /\ commitIndex' = index
    /\ UNCHANGED <<log,
                    pendingIndex, pendingTerm, pendingValue, pendingChunks,
                    durableIndex, durableTerm, durableValue,
                    appliedIndex, appliedValue, up>>

BeginReceive(index, term, value) ==
    /\ up
    \* Delayed snapshots at or below committed progress are acknowledged and
    \* discarded by Rust; only a newer snapshot enters the pending buffer.
    /\ index \in (commitIndex + 1)..MaxIndex
    /\ term \in 1..MaxIndex
    /\ value \in Value
    /\ pendingIndex' = index
    /\ pendingTerm' = term
    /\ pendingValue' = value
    /\ pendingChunks' = 0
    /\ UNCHANGED <<log, commitIndex,
                    durableIndex, durableTerm, durableValue,
                    appliedIndex, appliedValue, up>>

ReceiveChunk ==
    /\ up
    /\ pendingIndex > 0
    /\ pendingChunks < MaxChunks
    /\ pendingChunks' = pendingChunks + 1
    /\ UNCHANGED <<log, commitIndex,
                    pendingIndex, pendingTerm, pendingValue,
                    durableIndex, durableTerm, durableValue,
                    appliedIndex, appliedValue, up>>

MatchingBoundary ==
    \E offset \in 1..Len(log) :
        /\ log[offset].index = pendingIndex
        /\ log[offset].term = pendingTerm

SuffixAfterBoundary ==
    IF MatchingBoundary
    THEN LET offset == CHOOSE i \in 1..Len(log) :
                           /\ log[i].index = pendingIndex
                           /\ log[i].term = pendingTerm
         IN SubSeq(log, offset + 1, Len(log))
    ELSE <<>>

PersistSnapshot ==
    /\ up
    /\ pendingIndex > commitIndex
    /\ pendingChunks > 0
    \* DiskLogStore serializes this boundary, data, and retained suffix into
    \* one object-store copy-on-write replacement.
    /\ durableIndex' = pendingIndex
    /\ durableTerm' = pendingTerm
    /\ durableValue' = pendingValue
    /\ log' = SuffixAfterBoundary
    /\ commitIndex' = pendingIndex
    /\ UNCHANGED <<pendingIndex, pendingTerm, pendingValue, pendingChunks,
                    appliedIndex, appliedValue, up>>

ActivateSnapshot ==
    /\ up
    /\ durableIndex > appliedIndex
    /\ appliedIndex' = durableIndex
    /\ appliedValue' = durableValue
    /\ pendingIndex' = 0
    /\ pendingTerm' = 0
    /\ pendingValue' = NoValue
    /\ pendingChunks' = 0
    /\ UNCHANGED <<log, commitIndex,
                    durableIndex, durableTerm, durableValue, up>>

DiscardStale(index) ==
    /\ up
    /\ index \in 0..commitIndex
    /\ UNCHANGED vars

Crash ==
    /\ up
    /\ up' = FALSE
    \* The receive buffer and volatile state-machine image do not survive.
    /\ pendingIndex' = 0
    /\ pendingTerm' = 0
    /\ pendingValue' = NoValue
    /\ pendingChunks' = 0
    /\ appliedIndex' = 0
    /\ appliedValue' = NoValue
    /\ UNCHANGED <<log, commitIndex,
                    durableIndex, durableTerm, durableValue>>

Restart ==
    /\ ~up
    /\ up' = TRUE
    \* RaftNode::new restores the durable snapshot before declaring its index
    \* committed and applied.
    /\ commitIndex' = durableIndex
    /\ appliedIndex' = durableIndex
    /\ appliedValue' = durableValue
    /\ UNCHANGED <<log,
                    pendingIndex, pendingTerm, pendingValue, pendingChunks,
                    durableIndex, durableTerm, durableValue>>

Next ==
    \/ \E term \in 1..MaxIndex, value \in Value : AppendLog(term, value)
    \/ \E index \in 1..MaxIndex : Commit(index)
    \/ \E index \in 1..MaxIndex, term \in 1..MaxIndex, value \in Value :
           BeginReceive(index, term, value)
    \/ ReceiveChunk
    \/ PersistSnapshot
    \/ ActivateSnapshot
    \/ \E index \in 0..MaxIndex : DiscardStale(index)
    \/ Crash
    \/ Restart

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ up \in BOOLEAN
    /\ log \in Seq(Entry)
    /\ Len(log) <= MaxIndex
    /\ commitIndex \in Index
    /\ pendingIndex \in Index
    /\ pendingTerm \in Index
    /\ pendingValue \in SnapshotValue
    /\ pendingChunks \in 0..MaxChunks
    /\ durableIndex \in Index
    /\ durableTerm \in Index
    /\ durableValue \in SnapshotValue
    /\ appliedIndex \in Index
    /\ appliedValue \in SnapshotValue

LogAboveSnapshot ==
    \A offset \in 1..Len(log) :
        /\ log[offset].index = durableIndex + offset
        /\ log[offset].index <= MaxIndex

DurableSnapshotWellFormed ==
    /\ (durableIndex = 0) = (durableValue = NoValue)
    /\ durableIndex <= commitIndex

ApplicationMatchesDurable ==
    /\ appliedIndex <= durableIndex
    /\ appliedIndex = 0 => appliedValue = NoValue
    /\ appliedIndex = durableIndex => appliedValue = durableValue

RunningNodeRecovered ==
    /\ up
    /\ pendingIndex = 0
    /\ appliedIndex < durableIndex
    => commitIndex = durableIndex

Invariants ==
    /\ TypeOK
    /\ LogAboveSnapshot
    /\ DurableSnapshotWellFormed
    /\ ApplicationMatchesDurable
    /\ RunningNodeRecovered

=============================================================================
