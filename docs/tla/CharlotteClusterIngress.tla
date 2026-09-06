---------------------- MODULE CharlotteClusterIngress ----------------------
\* Joint cluster-control and DSR-ingress safety model.
\*
\* The existing Raft models establish how a command becomes committed. This
\* layer starts at that linearization point and checks what happens while
\* committed membership, placement, readiness, and drain state are projected
\* asynchronously into per-router ingress snapshots and retained flow epochs.
\*
\* Policy versions stand in for immutable BackendSnapshot epochs. The Rust
\* epoch is a deterministic 64-bit digest; the model uses the complete version
\* identity and therefore does not attempt to prove hash collision resistance.

EXTENDS Naturals, FiniteSets, Sequences

CONSTANTS Node, Flow, InitialVoters, InitialReplicas, MaxGeneration,
          MaxPolicyVersion, HistoryLimit, NoNode,
          AllowStaleNewFlow, AllowHistoryFallback, AllowStaleReadiness

ASSUME Node \subseteq Nat
ASSUME Node /= {}
ASSUME Flow /= {}
ASSUME InitialVoters \subseteq Node
ASSUME InitialVoters /= {}
ASSUME InitialReplicas \subseteq InitialVoters
ASSUME InitialReplicas /= {}
ASSUME MaxGeneration > 1
ASSUME MaxPolicyVersion > 3
ASSUME HistoryLimit > 0
ASSUME NoNode \notin Node

Generation == 0..MaxGeneration
PolicyVersion == 1..MaxPolicyVersion
MembershipPhase == {"Stable", "Joint"}

MinNode(nodes) == CHOOSE n \in nodes : \A other \in nodes : n <= other
MaxNode(nodes) == CHOOSE n \in nodes : \A other \in nodes : n >= other

IngressMembersFor(phaseValue, current, next) ==
    IF phaseValue = "Stable" THEN current ELSE current \cap next

DerivedEligible(members, generation, replicas, ready, draining) ==
    {n \in members :
        /\ n \in replicas
        /\ ready[n] = generation
        /\ n \notin draining}

MakePolicy(members, generation, replicas, ready, draining) ==
    LET eligible == DerivedEligible(members, generation, replicas, ready, draining)
        ingressMembers == members \ draining
    IN [members |-> members,
        generation |-> generation,
        replicas |-> replicas,
        ready |-> ready,
        draining |-> draining,
        eligible |-> eligible,
        advertiser |->
            IF eligible /= {} /\ ingressMembers /= {}
            THEN MinNode(ingressMembers)
            ELSE NoNode]

PolicyType ==
    [members: SUBSET Node,
     generation: 1..MaxGeneration,
     replicas: SUBSET Node,
     ready: [Node -> Generation],
     draining: SUBSET Node,
     eligible: SUBSET Node,
     advertiser: Node \cup {NoNode}]

SeqToSet(sequence) == {sequence[index] : index \in 1..Len(sequence)}

AppendBounded(sequence, version) ==
    IF Len(sequence) < HistoryLimit
    THEN Append(sequence, version)
    ELSE Append(Tail(sequence), version)

Winner(policy) == MaxNode(policy.eligible)

VARIABLES membershipPhase, currentVoters, nextVoters,
          deploymentGeneration, replicas, readyGeneration, draining,
          policyVersion, policies,
          knownRoutes, routerPolicy, history, leaseFresh,
          bindingPolicy, bindingBackend,
          staleNewFlowAdmitted, flowRemapped

vars == <<membershipPhase, currentVoters, nextVoters,
          deploymentGeneration, replicas, readyGeneration, draining,
          policyVersion, policies,
          knownRoutes, routerPolicy, history, leaseFresh,
          bindingPolicy, bindingBackend,
          staleNewFlowAdmitted, flowRemapped>>

InitialReady == [n \in Node |-> 0]
InitialPolicy ==
    MakePolicy(InitialVoters, 1, InitialReplicas, InitialReady, {})

Init ==
    /\ membershipPhase = "Stable"
    /\ currentVoters = InitialVoters
    /\ nextVoters = {}
    /\ deploymentGeneration = 1
    /\ replicas = InitialReplicas
    /\ readyGeneration = InitialReady
    /\ draining = {}
    /\ policyVersion = 1
    /\ policies = [version \in PolicyVersion |-> InitialPolicy]
    /\ knownRoutes = [router \in Node |-> {router}]
    /\ routerPolicy = [router \in Node |-> 0]
    /\ history = [router \in Node |-> <<>>]
    /\ leaseFresh = [router \in Node |-> FALSE]
    /\ bindingPolicy = [router \in Node |-> [flow \in Flow |-> 0]]
    /\ bindingBackend = [router \in Node |-> [flow \in Flow |-> NoNode]]
    /\ staleNewFlowAdmitted = FALSE
    /\ flowRemapped = FALSE

CurrentMembers ==
    IngressMembersFor(membershipPhase, currentVoters, nextVoters)

\* Every committed control-plane mutation records a complete immutable policy
\* projection. Routers may learn about that projection later.
PublishReady(node) ==
    /\ node \in replicas \cap CurrentMembers
    /\ node \notin draining
    /\ readyGeneration[node] /= deploymentGeneration
    /\ policyVersion < MaxPolicyVersion
    /\ LET nextReady == [readyGeneration EXCEPT ![node] = deploymentGeneration]
           nextVersion == policyVersion + 1
       IN /\ readyGeneration' = nextReady
          /\ policies' =
              [policies EXCEPT
                  ![nextVersion] = MakePolicy(CurrentMembers,
                                               deploymentGeneration,
                                               replicas,
                                               nextReady,
                                               draining)]
          /\ policyVersion' = nextVersion
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, draining,
                    knownRoutes, routerPolicy, history, leaseFresh,
                    bindingPolicy, bindingBackend,
                    staleNewFlowAdmitted, flowRemapped>>

WithdrawReady(node) ==
    /\ readyGeneration[node] /= 0
    /\ policyVersion < MaxPolicyVersion
    /\ LET nextReady == [readyGeneration EXCEPT ![node] = 0]
           nextVersion == policyVersion + 1
       IN /\ readyGeneration' = nextReady
          /\ policies' =
              [policies EXCEPT
                  ![nextVersion] = MakePolicy(CurrentMembers,
                                               deploymentGeneration,
                                               replicas,
                                               nextReady,
                                               draining)]
          /\ policyVersion' = nextVersion
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, draining,
                    knownRoutes, routerPolicy, history, leaseFresh,
                    bindingPolicy, bindingBackend,
                    staleNewFlowAdmitted, flowRemapped>>

ReplaceDeployment(nextReplicas) ==
    /\ nextReplicas \subseteq CurrentMembers
    /\ nextReplicas /= {}
    /\ nextReplicas /= replicas
    /\ deploymentGeneration < MaxGeneration
    /\ policyVersion < MaxPolicyVersion
    /\ LET nextGeneration == deploymentGeneration + 1
           nextVersion == policyVersion + 1
       IN /\ deploymentGeneration' = nextGeneration
          /\ replicas' = nextReplicas
          /\ policies' =
              [policies EXCEPT
                  ![nextVersion] = MakePolicy(CurrentMembers,
                                               nextGeneration,
                                               nextReplicas,
                                               readyGeneration,
                                               draining)]
          /\ policyVersion' = nextVersion
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    readyGeneration, draining,
                    knownRoutes, routerPolicy, history, leaseFresh,
                    bindingPolicy, bindingBackend,
                    staleNewFlowAdmitted, flowRemapped>>

CommitDrain(node) ==
    /\ node \in CurrentMembers
    /\ node \notin draining
    /\ policyVersion < MaxPolicyVersion
    /\ LET nextDraining == draining \cup {node}
           nextVersion == policyVersion + 1
       IN /\ draining' = nextDraining
          /\ policies' =
              [policies EXCEPT
                  ![nextVersion] = MakePolicy(CurrentMembers,
                                               deploymentGeneration,
                                               replicas,
                                               readyGeneration,
                                               nextDraining)]
          /\ policyVersion' = nextVersion
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, readyGeneration,
                    knownRoutes, routerPolicy, history, leaseFresh,
                    bindingPolicy, bindingBackend,
                    staleNewFlowAdmitted, flowRemapped>>

BeginJoint(nextMembers) ==
    /\ membershipPhase = "Stable"
    /\ nextMembers \subseteq Node
    /\ nextMembers /= {}
    /\ nextMembers /= currentVoters
    /\ policyVersion < MaxPolicyVersion
    /\ LET active == currentVoters \cap nextMembers
           nextVersion == policyVersion + 1
       IN /\ membershipPhase' = "Joint"
          /\ nextVoters' = nextMembers
          /\ policies' =
              [policies EXCEPT
                  ![nextVersion] = MakePolicy(active,
                                               deploymentGeneration,
                                               replicas,
                                               readyGeneration,
                                               draining)]
          /\ policyVersion' = nextVersion
    /\ UNCHANGED <<currentVoters, deploymentGeneration, replicas,
                    readyGeneration, draining,
                    knownRoutes, routerPolicy, history, leaseFresh,
                    bindingPolicy, bindingBackend,
                    staleNewFlowAdmitted, flowRemapped>>

FinalizeJoint ==
    /\ membershipPhase = "Joint"
    /\ policyVersion < MaxPolicyVersion
    /\ LET nextMembers == nextVoters
           nextVersion == policyVersion + 1
       IN /\ membershipPhase' = "Stable"
          /\ currentVoters' = nextMembers
          /\ nextVoters' = {}
          /\ policies' =
              [policies EXCEPT
                  ![nextVersion] = MakePolicy(nextMembers,
                                               deploymentGeneration,
                                               replicas,
                                               readyGeneration,
                                               draining)]
          /\ policyVersion' = nextVersion
    /\ UNCHANGED <<deploymentGeneration, replicas, readyGeneration, draining,
                    knownRoutes, routerPolicy, history, leaseFresh,
                    bindingPolicy, bindingBackend,
                    staleNewFlowAdmitted, flowRemapped>>

LearnRoute(router, node) ==
    /\ node \notin knownRoutes[router]
    /\ knownRoutes' = [knownRoutes EXCEPT ![router] = @ \cup {node}]
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, readyGeneration, draining,
                    policyVersion, policies, routerPolicy, history, leaseFresh,
                    bindingPolicy, bindingBackend,
                    staleNewFlowAdmitted, flowRemapped>>

ForgetRoute(router, node) ==
    /\ node \in knownRoutes[router]
    /\ node /= router
    /\ knownRoutes' = [knownRoutes EXCEPT ![router] = @ \ {node}]
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, readyGeneration, draining,
                    policyVersion, policies, routerPolicy, history, leaseFresh,
                    bindingPolicy, bindingBackend,
                    staleNewFlowAdmitted, flowRemapped>>

\* Installing a snapshot is all-or-nothing: every admitted member must have a
\* route. The action abstracts the concrete DNS source-freshness gate (leader
\* quorum contact or a recent successful follower log match). A flow may
\* continue naming an evicted policy, but no safe packet action interprets
\* that binding through a different snapshot.
InstallSnapshot(router, version) ==
    /\ version \in 1..policyVersion
    /\ version > routerPolicy[router]
    /\ policies[version].members \subseteq knownRoutes[router]
    /\ LET nextHistory == AppendBounded(history[router], version)
       IN /\ history' = [history EXCEPT ![router] = nextHistory]
          /\ routerPolicy' = [routerPolicy EXCEPT ![router] = version]
          /\ leaseFresh' = [leaseFresh EXCEPT ![router] = TRUE]
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, readyGeneration, draining,
                    policyVersion, policies, knownRoutes,
                    bindingPolicy, bindingBackend,
                    staleNewFlowAdmitted, flowRemapped>>

\* A successful materialization grants only a bounded local lease. The weak
\* fairness condition in Spec ensures that a lease cannot remain fresh forever
\* without another successful InstallSnapshot refresh.
ExpireLease(router) ==
    /\ leaseFresh[router]
    /\ leaseFresh' = [leaseFresh EXCEPT ![router] = FALSE]
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, readyGeneration, draining,
                    policyVersion, policies, knownRoutes, routerPolicy, history,
                    bindingPolicy, bindingBackend,
                    staleNewFlowAdmitted, flowRemapped>>

StartNewFlow(router, flow) ==
    /\ bindingPolicy[router][flow] = 0
    /\ leaseFresh[router]
    /\ routerPolicy[router] /= 0
    /\ policies[routerPolicy[router]].advertiser = router
    /\ policies[routerPolicy[router]].eligible /= {}
    /\ bindingPolicy' =
        [bindingPolicy EXCEPT ![router][flow] = routerPolicy[router]]
    /\ bindingBackend' =
        [bindingBackend EXCEPT
            ![router][flow] = Winner(policies[routerPolicy[router]])]
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, readyGeneration, draining,
                    policyVersion, policies, knownRoutes, routerPolicy, history,
                    leaseFresh,
                    staleNewFlowAdmitted, flowRemapped>>

ExistingFlowPacket(router, flow) ==
    /\ bindingPolicy[router][flow] /= 0
    /\ bindingPolicy[router][flow] \in SeqToSet(history[router])
    /\ UNCHANGED vars

\* A router policy cannot admit an unbound flow after its freshness lease
\* expires. Existing bindings remain eligible for their retained epoch.
DropStaleNewFlow(router, flow) ==
    /\ bindingPolicy[router][flow] = 0
    /\ routerPolicy[router] /= 0
    /\ ~leaseFresh[router]
    /\ UNCHANGED vars

\* If bounded history no longer contains a flow's pinned epoch, the packet is
\* rejected. The binding remains as a tombstone and is never reinterpreted.
DropUnretainedFlow(router, flow) ==
    /\ bindingPolicy[router][flow] /= 0
    /\ bindingPolicy[router][flow] \notin SeqToSet(history[router])
    /\ UNCHANGED vars

EndFlow(router, flow) ==
    /\ bindingPolicy[router][flow] /= 0
    /\ bindingPolicy' = [bindingPolicy EXCEPT ![router][flow] = 0]
    /\ bindingBackend' = [bindingBackend EXCEPT ![router][flow] = NoNode]
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, readyGeneration, draining,
                    policyVersion, policies, knownRoutes, routerPolicy, history,
                    leaseFresh,
                    staleNewFlowAdmitted, flowRemapped>>

\* Negative regression: the router admits a new SYN after the local snapshot
\* lease has expired. The extra policy checks produce a concrete withdrawn
\* backend counterexample rather than merely setting the witness bit.
UnsafeStartStaleFlow(router, flow) ==
    /\ AllowStaleNewFlow
    /\ bindingPolicy[router][flow] = 0
    /\ ~leaseFresh[router]
    /\ routerPolicy[router] \in 1..(policyVersion - 1)
    /\ policies[routerPolicy[router]].advertiser = router
    /\ policies[routerPolicy[router]].eligible /= {}
    /\ Winner(policies[routerPolicy[router]])
        \notin policies[policyVersion].eligible
    /\ bindingPolicy' =
        [bindingPolicy EXCEPT ![router][flow] = routerPolicy[router]]
    /\ bindingBackend' =
        [bindingBackend EXCEPT
            ![router][flow] = Winner(policies[routerPolicy[router]])]
    /\ staleNewFlowAdmitted' = TRUE
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, readyGeneration, draining,
                    policyVersion, policies, knownRoutes, routerPolicy, history,
                    leaseFresh,
                    flowRemapped>>

\* Negative regression: the former fallback interprets a binding whose epoch
\* has left history through the newest snapshot and can change its backend.
UnsafeFallbackExistingPacket(router, flow) ==
    /\ AllowHistoryFallback
    /\ bindingPolicy[router][flow] /= 0
    /\ bindingPolicy[router][flow] \notin SeqToSet(history[router])
    /\ routerPolicy[router] /= 0
    /\ policies[routerPolicy[router]].eligible /= {}
    /\ Winner(policies[routerPolicy[router]]) /= bindingBackend[router][flow]
    /\ bindingPolicy' =
        [bindingPolicy EXCEPT ![router][flow] = routerPolicy[router]]
    /\ bindingBackend' =
        [bindingBackend EXCEPT
            ![router][flow] = Winner(policies[routerPolicy[router]])]
    /\ flowRemapped' = TRUE
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, readyGeneration, draining,
                    policyVersion, policies, knownRoutes, routerPolicy, history,
                    leaseFresh,
                    staleNewFlowAdmitted>>

\* Negative regression: an old readiness proof is treated as if it belonged
\* to the replacement generation when constructing a policy snapshot.
UnsafeAdmitStaleReadiness(node) ==
    /\ AllowStaleReadiness
    /\ deploymentGeneration > 1
    /\ node \in replicas \cap CurrentMembers
    /\ readyGeneration[node] > 0
    /\ readyGeneration[node] < deploymentGeneration
    /\ node \notin draining
    /\ policyVersion < MaxPolicyVersion
    /\ LET nextVersion == policyVersion + 1
           safePolicy == MakePolicy(CurrentMembers,
                                    deploymentGeneration,
                                    replicas,
                                    readyGeneration,
                                    draining)
           unsafeEligible == safePolicy.eligible \cup {node}
           unsafePolicy ==
               [safePolicy EXCEPT
                   !.eligible = unsafeEligible,
                   !.advertiser = MinNode(CurrentMembers \ draining)]
       IN /\ policies' = [policies EXCEPT ![nextVersion] = unsafePolicy]
          /\ policyVersion' = nextVersion
    /\ UNCHANGED <<membershipPhase, currentVoters, nextVoters,
                    deploymentGeneration, replicas, readyGeneration, draining,
                    knownRoutes, routerPolicy, history, leaseFresh,
                    bindingPolicy, bindingBackend,
                    staleNewFlowAdmitted, flowRemapped>>

Next ==
    \/ \E node \in Node : PublishReady(node)
    \/ \E node \in Node : WithdrawReady(node)
    \/ \E nextReplicas \in SUBSET Node : ReplaceDeployment(nextReplicas)
    \/ \E node \in Node : CommitDrain(node)
    \/ \E nextMembers \in SUBSET Node : BeginJoint(nextMembers)
    \/ FinalizeJoint
    \/ \E router, node \in Node : LearnRoute(router, node)
    \/ \E router, node \in Node : ForgetRoute(router, node)
    \/ \E router \in Node, version \in PolicyVersion : InstallSnapshot(router, version)
    \/ \E router \in Node : ExpireLease(router)
    \/ \E router \in Node, flow \in Flow : StartNewFlow(router, flow)
    \/ \E router \in Node, flow \in Flow : ExistingFlowPacket(router, flow)
    \/ \E router \in Node, flow \in Flow : DropStaleNewFlow(router, flow)
    \/ \E router \in Node, flow \in Flow : DropUnretainedFlow(router, flow)
    \/ \E router \in Node, flow \in Flow : EndFlow(router, flow)
    \/ \E router \in Node, flow \in Flow : UnsafeStartStaleFlow(router, flow)
    \/ \E router \in Node, flow \in Flow : UnsafeFallbackExistingPacket(router, flow)
    \/ \E node \in Node : UnsafeAdmitStaleReadiness(node)

Spec == Init /\ [][Next]_vars
        /\ \A router \in Node : WF_vars(ExpireLease(router))

TypeOK ==
    /\ membershipPhase \in MembershipPhase
    /\ currentVoters \subseteq Node
    /\ currentVoters /= {}
    /\ nextVoters \subseteq Node
    /\ membershipPhase = "Stable" => nextVoters = {}
    /\ membershipPhase = "Joint" => nextVoters /= {}
    /\ deploymentGeneration \in 1..MaxGeneration
    /\ replicas \subseteq Node
    /\ replicas /= {}
    /\ readyGeneration \in [Node -> Generation]
    /\ draining \subseteq Node
    /\ policyVersion \in PolicyVersion
    /\ policies \in [PolicyVersion -> PolicyType]
    /\ knownRoutes \in [Node -> SUBSET Node]
    /\ routerPolicy \in [Node -> 0..MaxPolicyVersion]
    /\ history \in [Node -> Seq(PolicyVersion)]
    /\ leaseFresh \in [Node -> BOOLEAN]
    /\ \A router \in Node : Len(history[router]) <= HistoryLimit
    /\ bindingPolicy \in [Node -> [Flow -> 0..MaxPolicyVersion]]
    /\ bindingBackend \in [Node -> [Flow -> Node \cup {NoNode}]]
    /\ staleNewFlowAdmitted \in BOOLEAN
    /\ flowRemapped \in BOOLEAN

CommittedPoliciesAreDerived ==
    \A version \in 1..policyVersion :
        LET policy == policies[version]
        IN /\ policy.eligible =
                   DerivedEligible(policy.members,
                                   policy.generation,
                                   policy.replicas,
                                   policy.ready,
                                   policy.draining)
           /\ policy.eligible \subseteq policy.members
           /\ policy.advertiser =
                IF policy.eligible = {}
                THEN NoNode
                ELSE MinNode(policy.members \ policy.draining)

InstalledSnapshotIsRetained ==
    \A router \in Node :
        routerPolicy[router] /= 0 =>
            /\ routerPolicy[router] <= policyVersion
            /\ routerPolicy[router] \in SeqToSet(history[router])

BoundFlowRemainsPinned ==
    \A router \in Node, flow \in Flow :
        bindingPolicy[router][flow] /= 0 =>
            /\ bindingPolicy[router][flow] \in 1..policyVersion
            /\ bindingBackend[router][flow] = Winner(policies[bindingPolicy[router][flow]])

EmptyBindingHasNoBackend ==
    \A router \in Node, flow \in Flow :
        (bindingPolicy[router][flow] = 0) =
        (bindingBackend[router][flow] = NoNode)

NewFlowsRequireFreshLease == ~staleNewFlowAdmitted

FlowBackendStable == ~flowRemapped

Invariants ==
    /\ TypeOK
    /\ CommittedPoliciesAreDerived
    /\ InstalledSnapshotIsRetained
    /\ BoundFlowRemainsPinned
    /\ EmptyBindingHasNoBackend
    /\ NewFlowsRequireFreshLease
    /\ FlowBackendStable

=============================================================================
