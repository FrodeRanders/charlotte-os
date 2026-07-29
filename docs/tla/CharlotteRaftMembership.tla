----------------------- MODULE CharlotteRaftMembership -----------------------
\* Replicated voter/learner membership, joint-consensus quorum changes,
\* automatic finalization, and decommissioning.

EXTENDS Naturals, FiniteSets

CONSTANTS Node, InitialVoters, MaxIndex, NoNode

ASSUME NoNode \notin Node
ASSUME InitialVoters \subseteq Node
ASSUME InitialVoters # {}
ASSUME MaxIndex >= 2

Index == 0..MaxIndex
Phase == {"Stable", "Joint", "Finalizing"}

VARIABLES up,
          leader,
          currentVoters,
          currentLearners,
          nextVoters,
          nextLearners,
          proposedVoters,
          proposedLearners,
          phase,
          lastIndex,
          commitIndex,
          jointIndex,
          finalizeIndex,
          matchIndex,
          decommissioned

vars == <<up, leader,
          currentVoters, currentLearners,
          nextVoters, nextLearners,
          proposedVoters, proposedLearners,
          phase, lastIndex, commitIndex, jointIndex, finalizeIndex,
          matchIndex, decommissioned>>

Members(voters, learners) == voters \cup learners

Majority(voters, acknowledgers) ==
    Cardinality(voters \cap acknowledgers) * 2 > Cardinality(voters)

ActiveMembers ==
    IF phase = "Stable"
    THEN Members(currentVoters, currentLearners)
    ELSE Members(currentVoters, currentLearners)
         \cup Members(nextVoters, nextLearners)

Acknowledged(index) ==
    {n \in Node : up[n] /\ matchIndex[n] >= index}

CurrentQuorum(index) == Majority(currentVoters, Acknowledged(index))

JointQuorum(index) ==
    /\ Majority(currentVoters, Acknowledged(index))
    /\ Majority(nextVoters, Acknowledged(index))

Init ==
    /\ up = [n \in Node |-> TRUE]
    /\ leader = NoNode
    /\ currentVoters = InitialVoters
    /\ currentLearners = {}
    /\ nextVoters = {}
    /\ nextLearners = {}
    /\ proposedVoters = {}
    /\ proposedLearners = {}
    /\ phase = "Stable"
    /\ lastIndex = 0
    /\ commitIndex = 0
    /\ jointIndex = 0
    /\ finalizeIndex = 0
    /\ matchIndex = [n \in Node |-> 0]
    /\ decommissioned =
        [n \in Node |-> n \notin Members(InitialVoters, {})]

\* Election is abstracted to the membership admission decision. The separate
\* CharlotteRaft model checks durable voting and one leader per term.
Elect(n, votes) ==
    /\ leader = NoNode
    /\ up[n]
    /\ n \in currentVoters
    /\ votes \subseteq Node
    /\ n \in votes
    /\ Majority(currentVoters, votes)
    /\ (phase = "Stable" \/ Majority(nextVoters, votes))
    /\ leader' = n
    /\ UNCHANGED <<up,
                    currentVoters, currentLearners,
                    nextVoters, nextLearners,
                    proposedVoters, proposedLearners,
                    phase, lastIndex, commitIndex, jointIndex, finalizeIndex,
                    matchIndex, decommissioned>>

\* A JOINT command is first appended to the leader's old-configuration log.
\* New peers do not become active replication targets until that entry commits.
SubmitJoint(voters, learners) ==
    /\ leader \in currentVoters
    /\ up[leader]
    /\ phase = "Stable"
    /\ lastIndex < MaxIndex
    /\ voters \subseteq Node
    /\ voters # {}
    /\ learners \subseteq Node
    /\ voters \cap learners = {}
    /\ proposedVoters' = voters
    /\ proposedLearners' = learners
    /\ jointIndex' = lastIndex + 1
    /\ lastIndex' = lastIndex + 1
    /\ matchIndex' = [matchIndex EXCEPT ![leader] = lastIndex + 1]
    /\ UNCHANGED <<up, leader,
                    currentVoters, currentLearners,
                    nextVoters, nextLearners,
                    phase, commitIndex, finalizeIndex, decommissioned>>

Replicate(n, index) ==
    /\ leader \in Node
    /\ up[leader] /\ up[n]
    /\ n # leader
    /\ n \in ActiveMembers
    /\ index \in (matchIndex[n] + 1)..lastIndex
    /\ matchIndex' = [matchIndex EXCEPT ![n] = index]
    /\ UNCHANGED <<up, leader,
                    currentVoters, currentLearners,
                    nextVoters, nextLearners,
                    proposedVoters, proposedLearners,
                    phase, lastIndex, commitIndex, jointIndex, finalizeIndex,
                    decommissioned>>

\* The JOINT entry is committed using the configuration that preceded it.
CommitJoint ==
    /\ phase = "Stable"
    /\ jointIndex > commitIndex
    /\ jointIndex <= lastIndex
    /\ CurrentQuorum(jointIndex)
    /\ commitIndex' = jointIndex
    /\ nextVoters' = proposedVoters
    /\ nextLearners' = proposedLearners
    /\ phase' = "Joint"
    \* During joint consensus, old and new members remain active. A learner is
    \* active but is not a voter and therefore is not decommissioned.
    /\ decommissioned' =
        [n \in Node |->
            n \notin (Members(currentVoters, currentLearners)
                      \cup Members(proposedVoters, proposedLearners))]
    /\ UNCHANGED <<up, leader,
                    currentVoters, currentLearners,
                    proposedVoters, proposedLearners,
                    lastIndex, jointIndex, finalizeIndex, matchIndex>>

\* Rust submits FINALIZE only after every proposed member has caught up to the
\* committed JOINT fence. This is stronger than merely requiring a new quorum.
SubmitFinalize ==
    /\ phase = "Joint"
    /\ leader \in currentVoters \cup nextVoters
    /\ up[leader]
    /\ lastIndex < MaxIndex
    /\ \A n \in Members(nextVoters, nextLearners) :
           matchIndex[n] >= jointIndex
    /\ finalizeIndex' = lastIndex + 1
    /\ lastIndex' = lastIndex + 1
    /\ matchIndex' = [matchIndex EXCEPT ![leader] = lastIndex + 1]
    /\ phase' = "Finalizing"
    /\ UNCHANGED <<up, leader,
                    currentVoters, currentLearners,
                    nextVoters, nextLearners,
                    proposedVoters, proposedLearners,
                    commitIndex, jointIndex, decommissioned>>

CommitFinalize ==
    /\ phase = "Finalizing"
    /\ finalizeIndex > commitIndex
    /\ finalizeIndex <= lastIndex
    /\ JointQuorum(finalizeIndex)
    /\ commitIndex' = finalizeIndex
    /\ currentVoters' = nextVoters
    /\ currentLearners' = nextLearners
    /\ nextVoters' = {}
    /\ nextLearners' = {}
    /\ proposedVoters' = {}
    /\ proposedLearners' = {}
    /\ phase' = "Stable"
    /\ jointIndex' = 0
    /\ finalizeIndex' = 0
    /\ decommissioned' =
        [n \in Node |->
            n \notin Members(nextVoters, nextLearners)]
    /\ leader' =
        IF leader \in nextVoters THEN leader ELSE NoNode
    /\ UNCHANGED <<up, lastIndex, matchIndex>>

Crash(n) ==
    /\ up[n]
    /\ up' = [up EXCEPT ![n] = FALSE]
    /\ leader' = IF leader = n THEN NoNode ELSE leader
    /\ UNCHANGED <<currentVoters, currentLearners,
                    nextVoters, nextLearners,
                    proposedVoters, proposedLearners,
                    phase, lastIndex, commitIndex, jointIndex, finalizeIndex,
                    matchIndex, decommissioned>>

Restart(n) ==
    /\ ~up[n]
    /\ up' = [up EXCEPT ![n] = TRUE]
    /\ UNCHANGED <<leader,
                    currentVoters, currentLearners,
                    nextVoters, nextLearners,
                    proposedVoters, proposedLearners,
                    phase, lastIndex, commitIndex, jointIndex, finalizeIndex,
                    matchIndex, decommissioned>>

Next ==
    \/ \E n \in Node, votes \in SUBSET Node : Elect(n, votes)
    \/ \E voters, learners \in SUBSET Node : SubmitJoint(voters, learners)
    \/ \E n \in Node, index \in 1..MaxIndex : Replicate(n, index)
    \/ CommitJoint
    \/ SubmitFinalize
    \/ CommitFinalize
    \/ \E n \in Node : Crash(n)
    \/ \E n \in Node : Restart(n)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ up \in [Node -> BOOLEAN]
    /\ leader \in Node \cup {NoNode}
    /\ currentVoters \subseteq Node
    /\ currentLearners \subseteq Node
    /\ nextVoters \subseteq Node
    /\ nextLearners \subseteq Node
    /\ proposedVoters \subseteq Node
    /\ proposedLearners \subseteq Node
    /\ phase \in Phase
    /\ lastIndex \in Index
    /\ commitIndex \in Index
    /\ jointIndex \in Index
    /\ finalizeIndex \in Index
    /\ matchIndex \in [Node -> Index]
    /\ decommissioned \in [Node -> BOOLEAN]

MembershipWellFormed ==
    /\ currentVoters # {}
    /\ currentVoters \cap currentLearners = {}
    /\ nextVoters \cap nextLearners = {}
    /\ proposedVoters \cap proposedLearners = {}
    /\ phase = "Stable" => nextVoters = {} /\ nextLearners = {}
    /\ phase # "Stable" => nextVoters # {}

ProgressWithinLog ==
    /\ commitIndex <= lastIndex
    /\ \A n \in Node : matchIndex[n] <= lastIndex
    /\ jointIndex <= lastIndex
    /\ finalizeIndex <= lastIndex

PhaseOrdering ==
    /\ phase = "Stable" =>
        /\ finalizeIndex = 0
        /\ (jointIndex = 0 \/ jointIndex > commitIndex)
    /\ phase = "Joint" =>
        /\ jointIndex = commitIndex
        /\ finalizeIndex = 0
    /\ phase = "Finalizing" =>
        /\ jointIndex <= commitIndex
        /\ finalizeIndex > commitIndex

DecommissioningMatchesMembership ==
    \A n \in Node : decommissioned[n] = (n \notin ActiveMembers)

LeaderIsEligible ==
    leader \in Node =>
        /\ up[leader]
        /\ leader \in currentVoters \cup nextVoters

FinalizationWasJoint ==
    phase = "Finalizing" =>
        /\ nextVoters # {}
        /\ \A n \in Members(nextVoters, nextLearners) :
               matchIndex[n] >= jointIndex

Invariants ==
    /\ TypeOK
    /\ MembershipWellFormed
    /\ ProgressWithinLog
    /\ PhaseOrdering
    /\ DecommissioningMatchesMembership
    /\ LeaderIsEligible
    /\ FinalizationWasJoint

=============================================================================
