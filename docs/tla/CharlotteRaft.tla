----------------------------- MODULE CharlotteRaft -----------------------------
\* Election safety and durable term/vote recovery for a fixed voter set.
\* Message loss is represented by simply not taking GrantVote; duplication is
\* represented by its idempotent same-candidate case.

EXTENDS Naturals, FiniteSets

CONSTANTS Node, MaxTerm, NoNode

ASSUME NoNode \notin Node
ASSUME MaxTerm > 0

Terms == 0..MaxTerm
Roles == {"Follower", "Candidate", "Leader"}

VARIABLES up, role, term, durableTerm, votedFor, durableVote,
          voteHistory, observedVotes

vars == <<up, role, term, durableTerm, votedFor, durableVote,
          voteHistory, observedVotes>>

Init ==
    /\ up = [n \in Node |-> TRUE]
    /\ role = [n \in Node |-> "Follower"]
    /\ term = [n \in Node |-> 0]
    /\ durableTerm = [n \in Node |-> 0]
    /\ votedFor = [n \in Node |-> NoNode]
    /\ durableVote = [n \in Node |-> NoNode]
    /\ voteHistory = [n \in Node |-> [t \in Terms |-> NoNode]]
    /\ observedVotes = [n \in Node |-> {}]

Majority(voters) == Cardinality(voters) * 2 > Cardinality(Node)

StartElection(n) ==
    /\ up[n]
    /\ role[n] # "Leader"
    /\ term[n] < MaxTerm
    /\ LET next == term[n] + 1 IN
       /\ role' = [role EXCEPT ![n] = "Candidate"]
       /\ term' = [term EXCEPT ![n] = next]
       \* Rust persists the term and self-vote before sending requests.
       /\ durableTerm' = [durableTerm EXCEPT ![n] = next]
       /\ votedFor' = [votedFor EXCEPT ![n] = n]
       /\ durableVote' = [durableVote EXCEPT ![n] = n]
       /\ voteHistory' = [voteHistory EXCEPT ![n][next] = n]
       /\ observedVotes' = [observedVotes EXCEPT ![n] = {n}]
    /\ UNCHANGED up

GrantVote(voter, candidate) ==
    /\ voter # candidate
    /\ up[voter] /\ up[candidate]
    /\ role[candidate] = "Candidate"
    /\ term[candidate] > 0
    /\ term[voter] <= term[candidate]
    /\ voteHistory[voter][term[candidate]] \in {NoNode, candidate}
    /\ term' = [term EXCEPT ![voter] = term[candidate]]
    /\ durableTerm' = [durableTerm EXCEPT ![voter] = term[candidate]]
    /\ role' = [role EXCEPT ![voter] = "Follower"]
    /\ votedFor' = [votedFor EXCEPT ![voter] = candidate]
    /\ durableVote' = [durableVote EXCEPT ![voter] = candidate]
    /\ voteHistory' =
        [voteHistory EXCEPT ![voter][term[candidate]] = candidate]
    /\ observedVotes' =
        [n \in Node |->
            IF n = candidate THEN observedVotes[n] \cup {voter}
            ELSE IF n = voter THEN {} ELSE observedVotes[n]]
    /\ UNCHANGED up

BecomeLeader(n) ==
    /\ up[n]
    /\ role[n] = "Candidate"
    /\ Majority(observedVotes[n])
    /\ role' = [role EXCEPT ![n] = "Leader"]
    /\ UNCHANGED <<up, term, durableTerm, votedFor, durableVote,
                   voteHistory, observedVotes>>

ObserveHigherTerm(n, higher) ==
    /\ up[n]
    /\ higher \in Terms
    /\ higher > term[n]
    /\ term' = [term EXCEPT ![n] = higher]
    /\ durableTerm' = [durableTerm EXCEPT ![n] = higher]
    /\ role' = [role EXCEPT ![n] = "Follower"]
    /\ votedFor' = [votedFor EXCEPT ![n] = NoNode]
    /\ durableVote' = [durableVote EXCEPT ![n] = NoNode]
    /\ observedVotes' = [observedVotes EXCEPT ![n] = {}]
    /\ UNCHANGED <<up, voteHistory>>

Crash(n) ==
    /\ up[n]
    /\ up' = [up EXCEPT ![n] = FALSE]
    /\ role' = [role EXCEPT ![n] = "Follower"]
    /\ observedVotes' = [observedVotes EXCEPT ![n] = {}]
    /\ UNCHANGED <<term, durableTerm, votedFor, durableVote, voteHistory>>

Restart(n) ==
    /\ ~up[n]
    /\ up' = [up EXCEPT ![n] = TRUE]
    /\ role' = [role EXCEPT ![n] = "Follower"]
    /\ term' = [term EXCEPT ![n] = durableTerm[n]]
    /\ votedFor' = [votedFor EXCEPT ![n] = durableVote[n]]
    /\ UNCHANGED <<durableTerm, durableVote, voteHistory, observedVotes>>

Next ==
    \/ \E n \in Node : StartElection(n)
    \/ \E voter, candidate \in Node : GrantVote(voter, candidate)
    \/ \E n \in Node : BecomeLeader(n)
    \/ \E n \in Node, higher \in Terms : ObserveHigherTerm(n, higher)
    \/ \E n \in Node : Crash(n)
    \/ \E n \in Node : Restart(n)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ up \in [Node -> BOOLEAN]
    /\ role \in [Node -> Roles]
    /\ term \in [Node -> Terms]
    /\ durableTerm \in [Node -> Terms]
    /\ votedFor \in [Node -> Node \cup {NoNode}]
    /\ durableVote \in [Node -> Node \cup {NoNode}]
    /\ voteHistory \in [Node -> [Terms -> Node \cup {NoNode}]]
    /\ observedVotes \in [Node -> SUBSET Node]

VolatileMatchesDurable ==
    \A n \in Node : up[n] =>
        /\ term[n] = durableTerm[n]
        /\ votedFor[n] = durableVote[n]

ObservedVotesWereDurable ==
    \A cand \in Node :
        \A voter \in observedVotes[cand] :
            voteHistory[voter][term[cand]] = cand

LeaderHasQuorum ==
    \A n \in Node : role[n] = "Leader" => Majority(observedVotes[n])

ElectionSafety ==
    \A first, second \in Node :
        /\ role[first] = "Leader"
        /\ role[second] = "Leader"
        /\ term[first] = term[second]
        => first = second

Invariants ==
    /\ TypeOK
    /\ VolatileMatchesDurable
    /\ ObservedVotesWereDurable
    /\ LeaderHasQuorum
    /\ ElectionSafety

=============================================================================
