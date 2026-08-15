---------------------- MODULE CharlotteServiceLifecycle ----------------------
\* Signed service loading plus the replicated two-phase name lifecycle,
\* generation-fenced removal, scheduler exit/reaping, and AS teardown.

EXTENDS Naturals, FiniteSets

CONSTANTS Domains, MaxGeneration, NullDomain

ASSUME NullDomain \notin Domains
ASSUME MaxGeneration > 1

Phase == {"Empty", "Loaded", "Running", "Prepared", "LocallyPublished",
          "Published", "Stopping", "Exited", "Reaped", "TornDown"}
ArtifactState == {"Unstaged", "Trusted", "Untrusted"}

VARIABLES phase, domainGeneration, artifactState, rejectedUntrusted,
          localPublished, catalogGeneration, catalogOwner, catalogActive,
          lookupOwner, lookupGeneration, reapedProof,
          staleActivateRejected, staleUnregisterRejected, replacementLost

vars == <<phase, domainGeneration, artifactState, rejectedUntrusted,
          localPublished, catalogGeneration, catalogOwner, catalogActive,
          lookupOwner, lookupGeneration, reapedProof,
          staleActivateRejected, staleUnregisterRejected, replacementLost>>

Init ==
    /\ phase = [d \in Domains |-> "Empty"]
    /\ domainGeneration = [d \in Domains |-> 0]
    /\ artifactState = [d \in Domains |-> "Unstaged"]
    /\ rejectedUntrusted = [d \in Domains |-> FALSE]
    /\ localPublished = [d \in Domains |-> FALSE]
    /\ catalogGeneration = 0
    /\ catalogOwner = NullDomain
    /\ catalogActive = FALSE
    /\ lookupOwner = NullDomain
    /\ lookupGeneration = 0
    /\ reapedProof = [d \in Domains |-> FALSE]
    /\ staleActivateRejected = FALSE
    /\ staleUnregisterRejected = FALSE
    /\ replacementLost = FALSE

StageTrusted(d) ==
    /\ phase[d] \in {"Empty", "TornDown"}
    /\ artifactState[d] # "Trusted"
    /\ artifactState' = [artifactState EXCEPT ![d] = "Trusted"]
    /\ rejectedUntrusted' = [rejectedUntrusted EXCEPT ![d] = FALSE]
    /\ UNCHANGED <<phase, domainGeneration, localPublished,
                    catalogGeneration, catalogOwner, catalogActive,
                    lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

StageUntrusted(d) ==
    /\ phase[d] \in {"Empty", "TornDown"}
    /\ artifactState[d] # "Untrusted"
    /\ artifactState' = [artifactState EXCEPT ![d] = "Untrusted"]
    /\ rejectedUntrusted' = [rejectedUntrusted EXCEPT ![d] = FALSE]
    /\ UNCHANGED <<phase, domainGeneration, localPublished,
                    catalogGeneration, catalogOwner, catalogActive,
                    lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

RejectUntrustedLoad(d) ==
    /\ phase[d] \in {"Empty", "TornDown"}
    /\ artifactState[d] = "Untrusted"
    /\ ~rejectedUntrusted[d]
    /\ rejectedUntrusted' = [rejectedUntrusted EXCEPT ![d] = TRUE]
    /\ UNCHANGED <<phase, domainGeneration, artifactState, localPublished,
                    catalogGeneration, catalogOwner, catalogActive,
                    lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

Load(d) ==
    /\ phase[d] \in {"Empty", "TornDown"}
    /\ artifactState[d] = "Trusted"
    /\ domainGeneration[d] < MaxGeneration
    /\ (~catalogActive \/ catalogOwner # d)
    /\ phase' = [phase EXCEPT ![d] = "Loaded"]
    /\ domainGeneration' = [domainGeneration EXCEPT ![d] = @ + 1]
    /\ reapedProof' = [reapedProof EXCEPT ![d] = FALSE]
    /\ localPublished' = [localPublished EXCEPT ![d] = FALSE]
    /\ UNCHANGED <<artifactState, rejectedUntrusted,
                    catalogGeneration, catalogOwner, catalogActive,
                    lookupOwner, lookupGeneration,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

Start(d) ==
    /\ phase[d] = "Loaded"
    /\ phase' = [phase EXCEPT ![d] = "Running"]
    /\ UNCHANGED <<domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration, catalogOwner,
                    catalogActive, lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

\* Replicated prepare allocates the new monotonic catalog generation and
\* deliberately makes it invisible. This matches NameCatalog::CMD_REGISTER;
\* a replacement has a bounded unavailable interval until activation.
Prepare(d) ==
    /\ phase[d] = "Running"
    /\ catalogGeneration < MaxGeneration
    /\ phase' =
        IF catalogActive
        THEN [phase EXCEPT
                ![catalogOwner] = IF @ = "Published" THEN "Running" ELSE @,
                ![d] = "Prepared"]
        ELSE [phase EXCEPT ![d] = "Prepared"]
    /\ catalogGeneration' = catalogGeneration + 1
    /\ catalogOwner' = d
    /\ catalogActive' = FALSE
    /\ localPublished' = [localPublished EXCEPT ![d] = FALSE]
    /\ UNCHANGED <<domainGeneration, artifactState, rejectedUntrusted,
                    lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

PublishLocal(d) ==
    /\ phase[d] = "Prepared"
    /\ catalogOwner = d
    /\ ~catalogActive
    /\ phase' = [phase EXCEPT ![d] = "LocallyPublished"]
    /\ localPublished' = [localPublished EXCEPT ![d] = TRUE]
    /\ UNCHANGED <<domainGeneration, artifactState, rejectedUntrusted,
                    catalogGeneration, catalogOwner, catalogActive,
                    lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

Activate(d, observedGeneration) ==
    /\ phase[d] = "LocallyPublished"
    /\ catalogOwner = d
    /\ observedGeneration = catalogGeneration
    /\ ~catalogActive
    /\ phase' = [phase EXCEPT ![d] = "Published"]
    /\ catalogActive' = TRUE
    /\ UNCHANGED <<domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration, catalogOwner,
                    lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

RejectStaleActivate(d, observedGeneration) ==
    /\ observedGeneration \in 1..MaxGeneration
    /\ (catalogOwner # d \/ observedGeneration # catalogGeneration
        \/ phase[d] # "LocallyPublished")
    /\ ~staleActivateRejected
    /\ staleActivateRejected' = TRUE
    /\ UNCHANGED <<phase, domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration, catalogOwner,
                    catalogActive, lookupOwner, lookupGeneration, reapedProof,
                    staleUnregisterRejected, replacementLost>>

Lookup ==
    /\ catalogActive
    /\ lookupOwner = NullDomain
    /\ lookupOwner' = catalogOwner
    /\ lookupGeneration' = catalogGeneration
    /\ UNCHANGED <<phase, domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration, catalogOwner,
                    catalogActive, reapedProof, staleActivateRejected,
                    staleUnregisterRejected, replacementLost>>

ClearLookup ==
    /\ lookupOwner # NullDomain
    /\ lookupOwner' = NullDomain
    /\ lookupGeneration' = 0
    /\ UNCHANGED <<phase, domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration, catalogOwner,
                    catalogActive, reapedProof, staleActivateRejected,
                    staleUnregisterRejected, replacementLost>>

FencedUnregister(d, expectedGeneration) ==
    /\ catalogActive
    /\ catalogOwner = d
    /\ catalogGeneration = expectedGeneration
    /\ catalogActive' = FALSE
    /\ catalogOwner' = NullDomain
    /\ phase' = [phase EXCEPT
                    ![d] = IF @ = "Published" THEN "Running" ELSE @]
    /\ UNCHANGED <<domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration,
                    lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

RejectStaleUnregister(d, expectedGeneration) ==
    /\ expectedGeneration \in 1..MaxGeneration
    /\ catalogActive
    /\ (catalogOwner # d \/ catalogGeneration # expectedGeneration)
    /\ ~staleUnregisterRejected
    /\ staleUnregisterRejected' = TRUE
    /\ UNCHANGED <<phase, domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration, catalogOwner,
                    catalogActive, lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, replacementLost>>

\* Regression action: an owner/generation-unfenced unregister destroys the
\* replacement selected by the current catalog state.
UnsafeStaleUnregister(d, expectedGeneration) ==
    /\ catalogActive
    /\ expectedGeneration \in 1..MaxGeneration
    /\ (catalogOwner # d \/ catalogGeneration # expectedGeneration)
    /\ catalogActive' = FALSE
    /\ catalogOwner' = NullDomain
    /\ replacementLost' = TRUE
    /\ UNCHANGED <<phase, domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration,
                    lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected>>

CleanupLocal(d) ==
    /\ localPublished[d]
    /\ catalogOwner # d
    /\ localPublished' = [localPublished EXCEPT ![d] = FALSE]
    /\ UNCHANGED <<phase, domainGeneration, artifactState, rejectedUntrusted,
                    catalogGeneration, catalogOwner, catalogActive,
                    lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

RequestStop(d) ==
    /\ phase[d] \in {"Running", "Prepared", "LocallyPublished", "Published"}
    /\ phase' = [phase EXCEPT ![d] = "Stopping"]
    /\ UNCHANGED <<domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration, catalogOwner,
                    catalogActive, lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

Exit(d) ==
    /\ phase[d] = "Stopping"
    /\ phase' = [phase EXCEPT ![d] = "Exited"]
    /\ UNCHANGED <<domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration, catalogOwner,
                    catalogActive, lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

\* A panic, fatal EL0 fault, or explicit DOMAIN_ABORT bypasses cooperative
\* stopping. Scheduler-level retirement of every thread is abstracted into the
\* transition to Exited; Reap and Teardown retain their ordinary ordering.
DomainAbort(d) ==
    /\ phase[d] \in {"Running", "Prepared", "LocallyPublished", "Published"}
    /\ phase' = [phase EXCEPT ![d] = "Exited"]
    /\ UNCHANGED <<domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration, catalogOwner,
                    catalogActive, lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

Reap(d) ==
    /\ phase[d] = "Exited"
    /\ phase' = [phase EXCEPT ![d] = "Reaped"]
    /\ reapedProof' = [reapedProof EXCEPT ![d] = TRUE]
    /\ UNCHANGED <<domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration, catalogOwner,
                    catalogActive, lookupOwner, lookupGeneration,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

Teardown(d) ==
    /\ phase[d] = "Reaped"
    /\ phase' = [phase EXCEPT ![d] = "TornDown"]
    /\ UNCHANGED <<domainGeneration, artifactState, rejectedUntrusted,
                    localPublished, catalogGeneration, catalogOwner,
                    catalogActive, lookupOwner, lookupGeneration, reapedProof,
                    staleActivateRejected, staleUnregisterRejected,
                    replacementLost>>

SafeNext ==
    \/ \E d \in Domains : StageTrusted(d) \/ StageUntrusted(d)
    \/ \E d \in Domains : RejectUntrustedLoad(d) \/ Load(d) \/ Start(d)
    \/ \E d \in Domains : Prepare(d) \/ PublishLocal(d)
    \/ \E d \in Domains, g \in 1..MaxGeneration :
           Activate(d, g) \/ RejectStaleActivate(d, g)
    \/ Lookup \/ ClearLookup
    \/ \E d \in Domains, g \in 1..MaxGeneration :
           FencedUnregister(d, g) \/ RejectStaleUnregister(d, g)
    \/ \E d \in Domains : CleanupLocal(d) \/ RequestStop(d) \/ Exit(d) \/ DomainAbort(d)
                              \/ Reap(d) \/ Teardown(d)

UnsafeNext == SafeNext \/
    \E d \in Domains, g \in 1..MaxGeneration : UnsafeStaleUnregister(d, g)

Spec == Init /\ [][SafeNext]_vars
UnsafeSpec == Init /\ [][UnsafeNext]_vars

TypeOK ==
    /\ phase \in [Domains -> Phase]
    /\ domainGeneration \in [Domains -> 0..MaxGeneration]
    /\ artifactState \in [Domains -> ArtifactState]
    /\ rejectedUntrusted \in [Domains -> BOOLEAN]
    /\ localPublished \in [Domains -> BOOLEAN]
    /\ catalogGeneration \in 0..MaxGeneration
    /\ catalogOwner \in Domains \cup {NullDomain}
    /\ catalogActive \in BOOLEAN
    /\ lookupOwner \in Domains \cup {NullDomain}
    /\ lookupGeneration \in 0..MaxGeneration
    /\ reapedProof \in [Domains -> BOOLEAN]
    /\ staleActivateRejected \in BOOLEAN
    /\ staleUnregisterRejected \in BOOLEAN
    /\ replacementLost \in BOOLEAN

CatalogConsistent ==
    /\ catalogActive =>
        /\ catalogOwner \in Domains
        /\ catalogGeneration > 0
        /\ localPublished[catalogOwner]
    /\ catalogOwner = NullDomain => ~catalogActive

LookupCameFromACommittedActivation ==
    lookupOwner # NullDomain =>
        /\ lookupGeneration > 0
        /\ lookupGeneration <= catalogGeneration

UntrustedNeverLoaded ==
    \A d \in Domains :
        phase[d] \notin {"Empty", "TornDown"} => artifactState[d] = "Trusted"

TeardownAfterReap ==
    \A d \in Domains : phase[d] = "TornDown" => reapedProof[d]

GenerationIsLive ==
    \A d \in Domains :
        phase[d] # "Empty" => domainGeneration[d] > 0

ReplacementSurvivesStaleUnregister == ~replacementLost

Invariants ==
    /\ TypeOK
    /\ CatalogConsistent
    /\ LookupCameFromACommittedActivation
    /\ UntrustedNeverLoaded
    /\ TeardownAfterReap
    /\ GenerationIsLive
    /\ ReplacementSurvivesStaleUnregister

=============================================================================
