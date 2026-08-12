----------------------- MODULE CharlotteAuthorization -----------------------
\* Target authorization contract for capability issuance. The policy engine
\* and name catalog are logically separate even when one process implements
\* both. A grant is bound to a subject, service generation, rights set, and
\* policy version, and it can be redeemed exactly once.

EXTENDS Naturals, FiniteSets

CONSTANTS Principals, Subjects, PolicyAdmins, ServiceManagers,
          Services, Rights, MaxGeneration, MaxPolicyVersion,
          MaxTickets, MaxCapabilities, NullPrincipal, NullService

ASSUME Subjects \subseteq Principals
ASSUME PolicyAdmins \subseteq Principals
ASSUME ServiceManagers \subseteq Principals
ASSUME NullPrincipal \notin Principals
ASSUME NullService \notin Services
ASSUME Services # {}
ASSUME Rights # {}
ASSUME MaxGeneration > 1
ASSUME MaxPolicyVersion > 1
ASSUME MaxTickets > 1
ASSUME MaxCapabilities > 1

TicketId == 1..MaxTickets
CapabilityId == 1..MaxCapabilities
TicketStates == {"Free", "Issued", "Redeemed", "Cancelled"}
CapabilityStates == {"Free", "Live", "Closed"}

Ticket == [
    state         : TicketStates,
    principal     : Subjects \cup {NullPrincipal},
    service       : Services \cup {NullService},
    generation    : 0..MaxGeneration,
    rights        : SUBSET Rights,
    policyVersion : 0..MaxPolicyVersion,
    allowedAtIssue: SUBSET Rights
]

NoTicket == [state |-> "Free", principal |-> NullPrincipal,
             service |-> NullService, generation |-> 0, rights |-> {},
             policyVersion |-> 0, allowedAtIssue |-> {}]

Capability == [
    state               : CapabilityStates,
    owner               : Principals \cup {NullPrincipal},
    service             : Services \cup {NullService},
    rights              : SUBSET Rights,
    ticket              : 0..MaxTickets,
    ticketGeneration    : 0..MaxGeneration,
    redeemGeneration    : 0..MaxGeneration,
    ticketPolicyVersion : 0..MaxPolicyVersion,
    redeemPolicyVersion : 0..MaxPolicyVersion,
    allowedAtRedeem     : SUBSET Rights
]

NoCapability == [state |-> "Free", owner |-> NullPrincipal,
                 service |-> NullService, rights |-> {}, ticket |-> 0,
                 ticketGeneration |-> 0, redeemGeneration |-> 0,
                 ticketPolicyVersion |-> 0, redeemPolicyVersion |-> 0,
                 allowedAtRedeem |-> {}]

VARIABLES published, serviceGeneration, serviceCeiling,
          policy, policyVersion, tickets, capabilities,
          lastPolicyActor, lastPublicationActor

vars == <<published, serviceGeneration, serviceCeiling,
          policy, policyVersion, tickets, capabilities,
          lastPolicyActor, lastPublicationActor>>

Init ==
    /\ published = [s \in Services |-> FALSE]
    /\ serviceGeneration = [s \in Services |-> 0]
    /\ serviceCeiling = [s \in Services |-> {}]
    /\ policy = [p \in Subjects |-> [s \in Services |-> {}]]
    /\ policyVersion = [p \in Subjects |-> [s \in Services |-> 0]]
    /\ tickets = [t \in TicketId |-> NoTicket]
    /\ capabilities = [c \in CapabilityId |-> NoCapability]
    /\ lastPolicyActor = NullPrincipal
    /\ lastPublicationActor = NullPrincipal

\* The service manager authenticates publication and declares the maximum
\* rights the resolver may derive from the retained service connection.
PublishService(actor, service, ceiling) ==
    /\ actor \in ServiceManagers
    /\ ~published[service]
    /\ serviceGeneration[service] < MaxGeneration
    /\ ceiling \in SUBSET Rights
    /\ ceiling # {}
    /\ published' = [published EXCEPT ![service] = TRUE]
    /\ serviceGeneration' = [serviceGeneration EXCEPT ![service] = @ + 1]
    /\ serviceCeiling' = [serviceCeiling EXCEPT ![service] = ceiling]
    /\ lastPublicationActor' = actor
    /\ UNCHANGED <<policy, policyVersion, tickets, capabilities,
                    lastPolicyActor>>

ReplaceService(actor, service, ceiling) ==
    /\ actor \in ServiceManagers
    /\ published[service]
    /\ serviceGeneration[service] < MaxGeneration
    /\ ceiling \in SUBSET Rights
    /\ ceiling # {}
    /\ serviceGeneration' = [serviceGeneration EXCEPT ![service] = @ + 1]
    /\ serviceCeiling' = [serviceCeiling EXCEPT ![service] = ceiling]
    /\ lastPublicationActor' = actor
    /\ UNCHANGED <<published, policy, policyVersion, tickets, capabilities,
                    lastPolicyActor>>

UnpublishService(actor, service) ==
    /\ actor \in ServiceManagers
    /\ published[service]
    /\ published' = [published EXCEPT ![service] = FALSE]
    /\ serviceCeiling' = [serviceCeiling EXCEPT ![service] = {}]
    /\ lastPublicationActor' = actor
    /\ UNCHANGED <<serviceGeneration, policy, policyVersion, tickets,
                    capabilities, lastPolicyActor>>

\* An authenticated policy administrator replaces one subject/service rule.
\* Incrementing its version fences every outstanding decision on that rule.
SetPolicy(actor, principal, service, allowed) ==
    /\ actor \in PolicyAdmins
    /\ policyVersion[principal][service] < MaxPolicyVersion
    /\ allowed \in SUBSET Rights
    /\ policy' = [policy EXCEPT ![principal][service] = allowed]
    /\ policyVersion' = [policyVersion EXCEPT ![principal][service] = @ + 1]
    /\ lastPolicyActor' = actor
    /\ UNCHANGED <<published, serviceGeneration, serviceCeiling, tickets,
                    capabilities, lastPublicationActor>>

\* `principal` is the identity derived from the kernel-authenticated sender,
\* not a caller-supplied claim. A co-located resolver may make IssueTicket and
\* Redeem one atomic implementation step.
IssueTicket(principal, service, requested, ticket) ==
    /\ tickets[ticket].state = "Free"
    /\ published[service]
    /\ policyVersion[principal][service] > 0
    /\ requested \in SUBSET Rights
    /\ requested # {}
    /\ requested \subseteq (policy[principal][service] \cap serviceCeiling[service])
    /\ tickets' = [tickets EXCEPT ![ticket] =
        [state |-> "Issued", principal |-> principal, service |-> service,
         generation |-> serviceGeneration[service], rights |-> requested,
         policyVersion |-> policyVersion[principal][service],
         allowedAtIssue |-> policy[principal][service] \cap serviceCeiling[service]]]
    /\ UNCHANGED <<published, serviceGeneration, serviceCeiling, policy,
                    policyVersion, capabilities, lastPolicyActor,
                    lastPublicationActor>>

CancelTicket(principal, ticket) ==
    /\ tickets[ticket].state = "Issued"
    /\ tickets[ticket].principal = principal
    /\ tickets' = [tickets EXCEPT ![ticket].state = "Cancelled"]
    /\ UNCHANGED <<published, serviceGeneration, serviceCeiling, policy,
                    policyVersion, capabilities, lastPolicyActor,
                    lastPublicationActor>>

Redeem(principal, ticket, capability) ==
    LET decision == tickets[ticket]
        service == decision.service
        allowed == policy[principal][service] \cap serviceCeiling[service]
    IN
    /\ decision.state = "Issued"
    /\ decision.principal = principal
    /\ capabilities[capability].state = "Free"
    /\ published[service]
    /\ decision.generation = serviceGeneration[service]
    /\ decision.policyVersion = policyVersion[principal][service]
    /\ decision.rights \subseteq allowed
    /\ tickets' = [tickets EXCEPT ![ticket].state = "Redeemed"]
    /\ capabilities' = [capabilities EXCEPT ![capability] =
        [state |-> "Live", owner |-> principal, service |-> service,
         rights |-> decision.rights, ticket |-> ticket,
         ticketGeneration |-> decision.generation,
         redeemGeneration |-> serviceGeneration[service],
         ticketPolicyVersion |-> decision.policyVersion,
         redeemPolicyVersion |-> policyVersion[principal][service],
         allowedAtRedeem |-> allowed]]
    /\ UNCHANGED <<published, serviceGeneration, serviceCeiling, policy,
                    policyVersion, lastPolicyActor, lastPublicationActor>>

CloseCapability(principal, capability) ==
    /\ capabilities[capability].state = "Live"
    /\ capabilities[capability].owner = principal
    /\ capabilities' = [capabilities EXCEPT ![capability].state = "Closed"]
    /\ UNCHANGED <<published, serviceGeneration, serviceCeiling, policy,
                    policyVersion, tickets, lastPolicyActor,
                    lastPublicationActor>>

Next ==
    \/ \E actor \in Principals, service \in Services,
          ceiling \in SUBSET Rights : PublishService(actor, service, ceiling)
    \/ \E actor \in Principals, service \in Services,
          ceiling \in SUBSET Rights : ReplaceService(actor, service, ceiling)
    \/ \E actor \in Principals, service \in Services :
          UnpublishService(actor, service)
    \/ \E actor \in Principals, principal \in Subjects, service \in Services,
          allowed \in SUBSET Rights : SetPolicy(actor, principal, service, allowed)
    \/ \E principal \in Subjects, service \in Services,
          requested \in SUBSET Rights, ticket \in TicketId :
          IssueTicket(principal, service, requested, ticket)
    \/ \E principal \in Subjects, ticket \in TicketId :
          CancelTicket(principal, ticket)
    \/ \E principal \in Subjects, ticket \in TicketId,
          capability \in CapabilityId : Redeem(principal, ticket, capability)
    \/ \E principal \in Principals, capability \in CapabilityId :
          CloseCapability(principal, capability)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ published \in [Services -> BOOLEAN]
    /\ serviceGeneration \in [Services -> 0..MaxGeneration]
    /\ serviceCeiling \in [Services -> SUBSET Rights]
    /\ policy \in [Subjects -> [Services -> SUBSET Rights]]
    /\ policyVersion \in [Subjects -> [Services -> 0..MaxPolicyVersion]]
    /\ tickets \in [TicketId -> Ticket]
    /\ capabilities \in [CapabilityId -> Capability]
    /\ lastPolicyActor \in Principals \cup {NullPrincipal}
    /\ lastPublicationActor \in Principals \cup {NullPrincipal}

PolicyMutationAuthorized ==
    lastPolicyActor = NullPrincipal \/ lastPolicyActor \in PolicyAdmins

PublicationAuthorized ==
    lastPublicationActor = NullPrincipal \/ lastPublicationActor \in ServiceManagers

TicketBoundedByDecision ==
    \A ticket \in TicketId :
        tickets[ticket].state # "Free" =>
            /\ tickets[ticket].principal \in Subjects
            /\ tickets[ticket].service \in Services
            /\ tickets[ticket].generation > 0
            /\ tickets[ticket].policyVersion > 0
            /\ tickets[ticket].rights # {}
            /\ tickets[ticket].rights \subseteq tickets[ticket].allowedAtIssue

CapabilityBackedByTicket ==
    \A capability \in CapabilityId :
        capabilities[capability].state # "Free" =>
            LET ticket == capabilities[capability].ticket IN
            /\ ticket \in TicketId
            /\ tickets[ticket].state = "Redeemed"
            /\ capabilities[capability].service = tickets[ticket].service

MintBoundToPrincipal ==
    \A capability \in CapabilityId :
        capabilities[capability].state # "Free" =>
            capabilities[capability].owner =
                tickets[capabilities[capability].ticket].principal

MintTargetsCurrentBinding ==
    \A capability \in CapabilityId :
        capabilities[capability].state # "Free" =>
            /\ capabilities[capability].ticketGeneration =
                   tickets[capabilities[capability].ticket].generation
            /\ capabilities[capability].redeemGeneration =
                   capabilities[capability].ticketGeneration

MintUsesCurrentPolicy ==
    \A capability \in CapabilityId :
        capabilities[capability].state # "Free" =>
            /\ capabilities[capability].ticketPolicyVersion =
                   tickets[capabilities[capability].ticket].policyVersion
            /\ capabilities[capability].redeemPolicyVersion =
                   capabilities[capability].ticketPolicyVersion

NoRightsAmplification ==
    \A capability \in CapabilityId :
        capabilities[capability].state # "Free" =>
            /\ capabilities[capability].rights \subseteq
                   tickets[capabilities[capability].ticket].rights
            /\ capabilities[capability].rights \subseteq
                   capabilities[capability].allowedAtRedeem

NoTicketReplay ==
    \A first, second \in CapabilityId :
        /\ capabilities[first].state # "Free"
        /\ capabilities[second].state # "Free"
        /\ capabilities[first].ticket = capabilities[second].ticket
        => first = second

Invariants ==
    /\ TypeOK
    /\ PolicyMutationAuthorized
    /\ PublicationAuthorized
    /\ TicketBoundedByDecision
    /\ CapabilityBackedByTicket
    /\ MintBoundToPrincipal
    /\ MintTargetsCurrentBinding
    /\ MintUsesCurrentPolicy
    /\ NoRightsAmplification
    /\ NoTicketReplay

\* Negative transitions retained to demonstrate that the corresponding
\* invariant and check configuration detect each missing fence.
UnsafeSetPolicy(actor, principal, service, allowed) ==
    /\ actor \in Principals \ PolicyAdmins
    /\ policyVersion[principal][service] < MaxPolicyVersion
    /\ allowed \in SUBSET Rights
    /\ policy' = [policy EXCEPT ![principal][service] = allowed]
    /\ policyVersion' = [policyVersion EXCEPT ![principal][service] = @ + 1]
    /\ lastPolicyActor' = actor
    /\ UNCHANGED <<published, serviceGeneration, serviceCeiling, tickets,
                    capabilities, lastPublicationActor>>

UnsafeRedeemOtherPrincipal(actor, ticket, capability) ==
    LET decision == tickets[ticket]
        service == decision.service
        allowed == policy[decision.principal][service] \cap serviceCeiling[service]
    IN
    /\ actor \in Principals
    /\ actor # decision.principal
    /\ decision.state = "Issued"
    /\ capabilities[capability].state = "Free"
    /\ published[service]
    /\ decision.generation = serviceGeneration[service]
    /\ decision.policyVersion = policyVersion[decision.principal][service]
    /\ decision.rights \subseteq allowed
    /\ tickets' = [tickets EXCEPT ![ticket].state = "Redeemed"]
    /\ capabilities' = [capabilities EXCEPT ![capability] =
        [state |-> "Live", owner |-> actor, service |-> service,
         rights |-> decision.rights, ticket |-> ticket,
         ticketGeneration |-> decision.generation,
         redeemGeneration |-> serviceGeneration[service],
         ticketPolicyVersion |-> decision.policyVersion,
         redeemPolicyVersion |-> policyVersion[decision.principal][service],
         allowedAtRedeem |-> allowed]]
    /\ UNCHANGED <<published, serviceGeneration, serviceCeiling, policy,
                    policyVersion, lastPolicyActor, lastPublicationActor>>

UnsafeRedeemStalePolicy(principal, ticket, capability) ==
    LET decision == tickets[ticket]
        service == decision.service
        allowed == policy[principal][service] \cap serviceCeiling[service]
    IN
    /\ decision.state = "Issued"
    /\ decision.principal = principal
    /\ capabilities[capability].state = "Free"
    /\ published[service]
    /\ decision.generation = serviceGeneration[service]
    /\ decision.policyVersion # policyVersion[principal][service]
    /\ tickets' = [tickets EXCEPT ![ticket].state = "Redeemed"]
    /\ capabilities' = [capabilities EXCEPT ![capability] =
        [state |-> "Live", owner |-> principal, service |-> service,
         rights |-> decision.rights, ticket |-> ticket,
         ticketGeneration |-> decision.generation,
         redeemGeneration |-> serviceGeneration[service],
         ticketPolicyVersion |-> decision.policyVersion,
         redeemPolicyVersion |-> policyVersion[principal][service],
         allowedAtRedeem |-> allowed]]
    /\ UNCHANGED <<published, serviceGeneration, serviceCeiling, policy,
                    policyVersion, lastPolicyActor, lastPublicationActor>>

UnsafeRedeemStaleBinding(principal, ticket, capability) ==
    LET decision == tickets[ticket]
        service == decision.service
        allowed == policy[principal][service] \cap serviceCeiling[service]
    IN
    /\ decision.state = "Issued"
    /\ decision.principal = principal
    /\ capabilities[capability].state = "Free"
    /\ published[service]
    /\ decision.generation # serviceGeneration[service]
    /\ decision.policyVersion = policyVersion[principal][service]
    /\ tickets' = [tickets EXCEPT ![ticket].state = "Redeemed"]
    /\ capabilities' = [capabilities EXCEPT ![capability] =
        [state |-> "Live", owner |-> principal, service |-> service,
         rights |-> decision.rights, ticket |-> ticket,
         ticketGeneration |-> decision.generation,
         redeemGeneration |-> serviceGeneration[service],
         ticketPolicyVersion |-> decision.policyVersion,
         redeemPolicyVersion |-> policyVersion[principal][service],
         allowedAtRedeem |-> allowed]]
    /\ UNCHANGED <<published, serviceGeneration, serviceCeiling, policy,
                    policyVersion, lastPolicyActor, lastPublicationActor>>

UnsafeAmplifyRights(principal, ticket, capability, minted) ==
    LET decision == tickets[ticket]
        service == decision.service
        allowed == policy[principal][service] \cap serviceCeiling[service]
    IN
    /\ decision.state = "Issued"
    /\ decision.principal = principal
    /\ capabilities[capability].state = "Free"
    /\ published[service]
    /\ decision.generation = serviceGeneration[service]
    /\ decision.policyVersion = policyVersion[principal][service]
    /\ minted \in SUBSET Rights
    /\ ~(minted \subseteq decision.rights)
    /\ tickets' = [tickets EXCEPT ![ticket].state = "Redeemed"]
    /\ capabilities' = [capabilities EXCEPT ![capability] =
        [state |-> "Live", owner |-> principal, service |-> service,
         rights |-> minted, ticket |-> ticket,
         ticketGeneration |-> decision.generation,
         redeemGeneration |-> serviceGeneration[service],
         ticketPolicyVersion |-> decision.policyVersion,
         redeemPolicyVersion |-> policyVersion[principal][service],
         allowedAtRedeem |-> allowed]]
    /\ UNCHANGED <<published, serviceGeneration, serviceCeiling, policy,
                    policyVersion, lastPolicyActor, lastPublicationActor>>

UnsafePolicySpec == Init /\ [][Next \/
    (\E actor \in Principals, principal \in Subjects, service \in Services,
        allowed \in SUBSET Rights :
        UnsafeSetPolicy(actor, principal, service, allowed))]_vars

UnsafePrincipalSpec == Init /\ [][Next \/
    (\E actor \in Principals, ticket \in TicketId,
        capability \in CapabilityId :
        UnsafeRedeemOtherPrincipal(actor, ticket, capability))]_vars

UnsafeStalePolicySpec == Init /\ [][Next \/
    (\E principal \in Subjects, ticket \in TicketId,
        capability \in CapabilityId :
        UnsafeRedeemStalePolicy(principal, ticket, capability))]_vars

UnsafeStaleBindingSpec == Init /\ [][Next \/
    (\E principal \in Subjects, ticket \in TicketId,
        capability \in CapabilityId :
        UnsafeRedeemStaleBinding(principal, ticket, capability))]_vars

UnsafeAmplificationSpec == Init /\ [][Next \/
    (\E principal \in Subjects, ticket \in TicketId,
        capability \in CapabilityId, minted \in SUBSET Rights :
        UnsafeAmplifyRights(principal, ticket, capability, minted))]_vars

=============================================================================
