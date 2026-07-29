--------------------------- MODULE CharlotteRaftLog ---------------------------
\* Bounded Raft log replication, conflict repair, commit, and durable restart.

EXTENDS Naturals, FiniteSets, Sequences

CONSTANTS Node, Value, MaxTerm, MaxIndex, NoNode

ASSUME NoNode \notin Node
ASSUME MaxTerm > 0
ASSUME MaxIndex > 0

Terms == 0..MaxTerm
Entry == [term : 1..MaxTerm, value : Value]

VARIABLES up, currentTerm, leader, logs, commitIndex

vars == <<up, currentTerm, leader, logs, commitIndex>>

Init ==
    /\ up = [n \in Node |-> TRUE]
    /\ currentTerm = [n \in Node |-> 0]
    /\ leader = NoNode
    /\ logs = [n \in Node |-> <<>>]
    /\ commitIndex = [n \in Node |-> 0]

LastTerm(log) == IF Len(log) = 0 THEN 0 ELSE log[Len(log)].term

AtLeastAsUpToDate(candidate, voter) ==
    \/ LastTerm(logs[candidate]) > LastTerm(logs[voter])
    \/ /\ LastTerm(logs[candidate]) = LastTerm(logs[voter])
       /\ Len(logs[candidate]) >= Len(logs[voter])

ElectionQuorum(candidate) ==
    {voter \in Node : up[voter] /\ AtLeastAsUpToDate(candidate, voter)}

Elect(candidate, newTerm) ==
    /\ up[candidate]
    /\ newTerm \in 1..MaxTerm
    \* CharlotteRaft supplies election safety: a replacement leader is from a
    \* strictly newer term than every leader epoch already represented here.
    /\ \A n \in Node : newTerm > currentTerm[n]
    /\ Cardinality(ElectionQuorum(candidate)) * 2 > Cardinality(Node)
    /\ leader' = candidate
    /\ currentTerm' =
        [n \in Node |-> IF n = candidate THEN newTerm ELSE currentTerm[n]]
    /\ UNCHANGED <<up, logs, commitIndex>>

AppendLeader(value) ==
    /\ leader \in Node
    /\ up[leader]
    /\ currentTerm[leader] > 0
    /\ Len(logs[leader]) < MaxIndex
    /\ logs' =
        [logs EXCEPT ![leader] =
            Append(@, [term |-> currentTerm[leader], value |-> value])]
    /\ UNCHANGED <<up, currentTerm, leader, commitIndex>>

\* One-entry AppendEntries projection. A matching prefix is retained, a
\* conflicting suffix is truncated, and the leader entry is appended.
ReplicateOne(follower, index) ==
    /\ leader \in Node
    /\ follower # leader
    /\ up[leader] /\ up[follower]
    /\ index \in 1..Len(logs[leader])
    /\ index <= MaxIndex
    /\ index = 1
       \/ /\ index > 1
          /\ Len(logs[follower]) >= index - 1
          /\ SubSeq(logs[follower], 1, index - 1) =
             SubSeq(logs[leader], 1, index - 1)
    /\ LET prefix == IF index = 1 THEN <<>>
                     ELSE SubSeq(logs[follower], 1, index - 1) IN
       logs' =
           IF /\ Len(logs[follower]) >= index
              /\ logs[follower][index] = logs[leader][index]
           THEN logs
           ELSE [logs EXCEPT
                    ![follower] = Append(prefix, logs[leader][index])]
    /\ currentTerm' =
        [currentTerm EXCEPT ![follower] =
            IF currentTerm[follower] > currentTerm[leader]
            THEN currentTerm[follower] ELSE currentTerm[leader]]
    /\ UNCHANGED <<up, leader, commitIndex>>

ReplicatedAt(index) ==
    {n \in Node :
        Len(logs[n]) >= index
        /\ logs[n][index] = logs[leader][index]}

CommitLeader(index) ==
    /\ leader \in Node
    /\ up[leader]
    /\ index \in (commitIndex[leader] + 1)..Len(logs[leader])
    \* Raft only advances by counting an entry from the leader's current term.
    /\ logs[leader][index].term = currentTerm[leader]
    /\ Cardinality(ReplicatedAt(index)) * 2 > Cardinality(Node)
    /\ commitIndex' = [commitIndex EXCEPT ![leader] = index]
    /\ UNCHANGED <<up, currentTerm, leader, logs>>

PropagateCommit(follower) ==
    /\ leader \in Node
    /\ follower # leader
    /\ up[leader] /\ up[follower]
    /\ commitIndex[follower] < commitIndex[leader]
    /\ LET next == IF commitIndex[leader] < Len(logs[follower])
                   THEN commitIndex[leader] ELSE Len(logs[follower]) IN
       /\ next > commitIndex[follower]
       /\ SubSeq(logs[follower], 1, next) = SubSeq(logs[leader], 1, next)
       /\ commitIndex' = [commitIndex EXCEPT ![follower] = next]
    /\ UNCHANGED <<up, currentTerm, leader, logs>>

Crash(n) ==
    /\ up[n]
    /\ up' = [up EXCEPT ![n] = FALSE]
    /\ leader' = IF leader = n THEN NoNode ELSE leader
    \* LogStore mutations are durable before their Rust calls return.
    /\ UNCHANGED <<currentTerm, logs, commitIndex>>

Restart(n) ==
    /\ ~up[n]
    /\ up' = [up EXCEPT ![n] = TRUE]
    /\ UNCHANGED <<currentTerm, leader, logs, commitIndex>>

Next ==
    \/ \E candidate \in Node, newTerm \in 1..MaxTerm : Elect(candidate, newTerm)
    \/ \E value \in Value : AppendLeader(value)
    \/ \E follower \in Node, index \in 1..MaxIndex : ReplicateOne(follower, index)
    \/ \E index \in 1..MaxIndex : CommitLeader(index)
    \/ \E follower \in Node : PropagateCommit(follower)
    \/ \E n \in Node : Crash(n)
    \/ \E n \in Node : Restart(n)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ up \in [Node -> BOOLEAN]
    /\ currentTerm \in [Node -> Terms]
    /\ leader \in Node \cup {NoNode}
    /\ logs \in [Node -> Seq(Entry)]
    /\ \A n \in Node : Len(logs[n]) <= MaxIndex
    /\ commitIndex \in [Node -> 0..MaxIndex]

CommitWithinLog ==
    \A n \in Node : commitIndex[n] <= Len(logs[n])

LogMatching ==
    \A first, second \in Node, index \in 1..MaxIndex :
        /\ Len(logs[first]) >= index
        /\ Len(logs[second]) >= index
        /\ logs[first][index].term = logs[second][index].term
        => SubSeq(logs[first], 1, index) = SubSeq(logs[second], 1, index)

CommittedAgreement ==
    \A first, second \in Node, index \in 1..MaxIndex :
        /\ commitIndex[first] >= index
        /\ commitIndex[second] >= index
        => logs[first][index] = logs[second][index]

LeaderCompleteness ==
    \A n \in Node, index \in 1..MaxIndex :
        /\ leader = n
        /\ \E prior \in Node : commitIndex[prior] >= index
        => /\ Len(logs[n]) >= index
           /\ \A prior \in Node :
                commitIndex[prior] >= index =>
                    logs[n][index] = logs[prior][index]

Invariants ==
    /\ TypeOK
    /\ CommitWithinLog
    /\ LogMatching
    /\ CommittedAgreement
    /\ LeaderCompleteness

=============================================================================
