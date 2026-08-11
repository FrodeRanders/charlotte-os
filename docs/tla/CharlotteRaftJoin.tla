--------------------------- MODULE CharlotteRaftJoin ---------------------------
\* Pre-membership Raft admission: a joiner selects one authoritative anchor,
\* a JOIN entry commits under the old configuration, the joiner catches up to
\* that fence, and only then may the leader submit/commit the joint change.
\* Admission state is durable: crash erases the volatile posture, restart
\* restores it, and the fence is cleared only with durable joint membership.

EXTENDS Naturals, FiniteSets

CONSTANTS Node, InitialVoters, MaxIndex, NoNode

ASSUME NoNode \notin Node
ASSUME InitialVoters \subseteq Node
ASSUME InitialVoters # {}
ASSUME MaxIndex >= 2

Index == 0..MaxIndex

VARIABLES members, leader, joining, selectedAnchor, joinIndex, joinCommitted,
          replicatedIndex, jointIndex, jointCommitted, lastIndex,
          unauthorizedAccepted, running, durableJoining, durableAnchor,
          campaignEligible, restartForgotAdmission

vars == <<members, leader, joining, selectedAnchor, joinIndex, joinCommitted,
          replicatedIndex, jointIndex, jointCommitted, lastIndex,
          unauthorizedAccepted, running, durableJoining, durableAnchor,
          campaignEligible, restartForgotAdmission>>

Init ==
    /\ members = InitialVoters
    /\ leader = NoNode
    /\ joining = {}
    /\ selectedAnchor = [n \in Node |-> NoNode]
    /\ joinIndex = [n \in Node |-> 0]
    /\ joinCommitted = {}
    /\ replicatedIndex = [n \in Node |-> 0]
    /\ jointIndex = [n \in Node |-> 0]
    /\ jointCommitted = {}
    /\ lastIndex = 0
    /\ unauthorizedAccepted = FALSE
    /\ running = Node
    /\ durableJoining = {}
    /\ durableAnchor = [n \in Node |-> NoNode]
    /\ campaignEligible = Node
    /\ restartForgotAdmission = FALSE

Elect(n) ==
    /\ leader = NoNode
    /\ n \in running
    /\ n \in members
    /\ n \notin joining
    /\ n \in campaignEligible
    /\ leader' = n
    /\ UNCHANGED <<members, joining, selectedAnchor, joinIndex,
                    joinCommitted, replicatedIndex, jointIndex,
                    jointCommitted, lastIndex, unauthorizedAccepted, running,
                    durableJoining, durableAnchor, campaignEligible,
                    restartForgotAdmission>>

BeginJoining(j, anchor) ==
    /\ j \notin members
    /\ j \in running
    /\ j \notin joining
    /\ anchor = leader
    /\ anchor \in members
    /\ joining' = joining \cup {j}
    /\ selectedAnchor' = [selectedAnchor EXCEPT ![j] = anchor]
    /\ durableJoining' = durableJoining \cup {j}
    /\ durableAnchor' = [durableAnchor EXCEPT ![j] = anchor]
    /\ campaignEligible' = campaignEligible \ {j}
    /\ UNCHANGED <<members, leader, joinIndex, joinCommitted,
                    replicatedIndex, jointIndex, jointCommitted, lastIndex,
                    unauthorizedAccepted, running, restartForgotAdmission>>

Crash(j) ==
    /\ j \in running
    /\ running' = running \ {j}
    /\ joining' = joining \ {j}
    /\ selectedAnchor' = [selectedAnchor EXCEPT ![j] = NoNode]
    /\ UNCHANGED <<members, leader, joinIndex, joinCommitted,
                    replicatedIndex, jointIndex, jointCommitted, lastIndex,
                    unauthorizedAccepted, durableJoining, durableAnchor,
                    campaignEligible, restartForgotAdmission>>

Restart(j) ==
    /\ j \notin running
    /\ running' = running \cup {j}
    /\ joining' = IF j \in durableJoining THEN joining \cup {j} ELSE joining
    /\ selectedAnchor' = IF j \in durableJoining
                          THEN [selectedAnchor EXCEPT ![j] = durableAnchor[j]]
                          ELSE selectedAnchor
    /\ UNCHANGED <<members, leader, joinIndex, joinCommitted,
                    replicatedIndex, jointIndex, jointCommitted, lastIndex,
                    unauthorizedAccepted, durableJoining, durableAnchor,
                    campaignEligible, restartForgotAdmission>>

UnsafeRestartForgetsAdmission(j) ==
    /\ j \notin running
    /\ j \in durableJoining
    /\ running' = running \cup {j}
    /\ campaignEligible' = campaignEligible \cup {j}
    /\ restartForgotAdmission' = TRUE
    /\ UNCHANGED <<members, leader, joining, selectedAnchor, joinIndex,
                    joinCommitted, replicatedIndex, jointIndex,
                    jointCommitted, lastIndex, unauthorizedAccepted,
                    durableJoining, durableAnchor>>

SubmitJoin(j) ==
    /\ j \in joining
    /\ leader = selectedAnchor[j]
    /\ joinIndex[j] = 0
    /\ lastIndex < MaxIndex
    /\ lastIndex' = lastIndex + 1
    /\ joinIndex' = [joinIndex EXCEPT ![j] = lastIndex + 1]
    /\ UNCHANGED <<members, leader, joining, selectedAnchor, joinCommitted,
                    replicatedIndex, jointIndex, jointCommitted,
                    unauthorizedAccepted, running, durableJoining,
                    durableAnchor, campaignEligible,
                    restartForgotAdmission>>

CommitJoin(j) ==
    /\ j \in joining
    /\ joinIndex[j] > 0
    /\ j \notin joinCommitted
    /\ leader = selectedAnchor[j]
    /\ joinCommitted' = joinCommitted \cup {j}
    /\ UNCHANGED <<members, leader, joining, selectedAnchor, joinIndex,
                    replicatedIndex, jointIndex, jointCommitted, lastIndex,
                    unauthorizedAccepted, running, durableJoining,
                    durableAnchor, campaignEligible,
                    restartForgotAdmission>>

ReplicateToJoiner(j, source, index) ==
    /\ j \in joinCommitted
    /\ j \in running
    /\ j \in joining
    /\ source = selectedAnchor[j]
    /\ index \in (replicatedIndex[j] + 1)..lastIndex
    /\ replicatedIndex' = [replicatedIndex EXCEPT ![j] = index]
    /\ UNCHANGED <<members, leader, joining, selectedAnchor, joinIndex,
                    joinCommitted, jointIndex, jointCommitted, lastIndex,
                    unauthorizedAccepted, running, durableJoining,
                    durableAnchor, campaignEligible,
                    restartForgotAdmission>>

UnsafeReplicateToJoiner(j, source, index) ==
    /\ j \in joinCommitted
    /\ j \in running
    /\ j \in joining
    /\ source \in Node
    /\ source # selectedAnchor[j]
    /\ index \in (replicatedIndex[j] + 1)..lastIndex
    /\ replicatedIndex' = [replicatedIndex EXCEPT ![j] = index]
    /\ unauthorizedAccepted' = TRUE
    /\ UNCHANGED <<members, leader, joining, selectedAnchor, joinIndex,
                    joinCommitted, jointIndex, jointCommitted, lastIndex,
                    running, durableJoining, durableAnchor,
                    campaignEligible, restartForgotAdmission>>

SubmitJoint(j) ==
    /\ j \in joinCommitted
    /\ replicatedIndex[j] >= joinIndex[j]
    /\ jointIndex[j] = 0
    /\ leader = selectedAnchor[j]
    /\ lastIndex < MaxIndex
    /\ lastIndex' = lastIndex + 1
    /\ jointIndex' = [jointIndex EXCEPT ![j] = lastIndex + 1]
    /\ UNCHANGED <<members, leader, joining, selectedAnchor, joinIndex,
                    joinCommitted, replicatedIndex, jointCommitted,
                    unauthorizedAccepted, running, durableJoining,
                    durableAnchor, campaignEligible,
                    restartForgotAdmission>>

CommitJoint(j) ==
    /\ j \in joining
    /\ jointIndex[j] > 0
    /\ j \notin jointCommitted
    /\ jointCommitted' = jointCommitted \cup {j}
    /\ members' = members \cup {j}
    /\ joining' = joining \ {j}
    /\ selectedAnchor' = [selectedAnchor EXCEPT ![j] = NoNode]
    /\ durableJoining' = durableJoining \ {j}
    /\ durableAnchor' = [durableAnchor EXCEPT ![j] = NoNode]
    /\ campaignEligible' = campaignEligible \cup {j}
    /\ UNCHANGED <<leader, joinIndex, joinCommitted, replicatedIndex,
                    jointIndex, lastIndex, unauthorizedAccepted, running,
                    restartForgotAdmission>>

SafeNext ==
    \/ \E n \in Node : Elect(n)
    \/ \E j, anchor \in Node : BeginJoining(j, anchor)
    \/ \E j \in Node : Crash(j) \/ Restart(j)
    \/ \E j \in Node : SubmitJoin(j) \/ CommitJoin(j)
    \/ \E j, source \in Node, index \in 1..MaxIndex :
           ReplicateToJoiner(j, source, index)
    \/ \E j \in Node : SubmitJoint(j) \/ CommitJoint(j)

UnsafeNext ==
    \/ SafeNext
    \/ \E j, source \in Node, index \in 1..MaxIndex :
           UnsafeReplicateToJoiner(j, source, index)
    \/ \E j \in Node : UnsafeRestartForgetsAdmission(j)

Spec == Init /\ [][SafeNext]_vars
UnsafeSpec == Init /\ [][UnsafeNext]_vars

TypeOK ==
    /\ members \subseteq Node
    /\ leader \in Node \cup {NoNode}
    /\ joining \subseteq Node
    /\ selectedAnchor \in [Node -> Node \cup {NoNode}]
    /\ joinIndex \in [Node -> Index]
    /\ joinCommitted \subseteq Node
    /\ replicatedIndex \in [Node -> Index]
    /\ jointIndex \in [Node -> Index]
    /\ jointCommitted \subseteq Node
    /\ lastIndex \in Index
    /\ unauthorizedAccepted \in BOOLEAN
    /\ running \subseteq Node
    /\ durableJoining \subseteq Node
    /\ durableAnchor \in [Node -> Node \cup {NoNode}]
    /\ campaignEligible \subseteq Node
    /\ restartForgotAdmission \in BOOLEAN

JoinAdmissionFenced ==
    /\ \A j \in Node : jointIndex[j] > 0 =>
           /\ j \in joinCommitted
           /\ replicatedIndex[j] >= joinIndex[j]
    /\ jointCommitted \subseteq joinCommitted

JoiningAcceptsOnlySelectedAnchor == ~unauthorizedAccepted

JoiningCannotCampaign ==
    /\ leader \notin joining
    /\ joining \cap campaignEligible = {}
    /\ durableJoining \cap campaignEligible = {}

RestartPreservesAdmission ==
    /\ ~restartForgotAdmission
    /\ \A j \in running \cap durableJoining :
           /\ j \in joining
           /\ selectedAnchor[j] = durableAnchor[j]

LeaderIsMember == leader \in Node => leader \in members

SelectedAnchorStable ==
    \A j \in durableJoining : durableAnchor[j] \in members

Invariants ==
    /\ TypeOK
    /\ JoinAdmissionFenced
    /\ JoiningAcceptsOnlySelectedAnchor
    /\ JoiningCannotCampaign
    /\ RestartPreservesAdmission
    /\ LeaderIsMember
    /\ SelectedAnchorStable

=============================================================================
