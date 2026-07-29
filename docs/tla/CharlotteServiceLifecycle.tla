---------------------- MODULE CharlotteServiceLifecycle ----------------------
\* Supervisor/name-service lifecycle projected over scheduler exit and reaping.

EXTENDS Naturals, FiniteSets

CONSTANTS Domains, MaxGeneration, NullDomain

ASSUME NullDomain \notin Domains
ASSUME MaxGeneration > 1

Phase == {"Empty", "Loaded", "Running", "Published", "Stopping",
          "Exited", "Reaped", "TornDown"}

VARIABLES phase, generation, published, lookupGeneration, reapedProof

vars == <<phase, generation, published, lookupGeneration, reapedProof>>

Init ==
    /\ phase = [d \in Domains |-> "Empty"]
    /\ generation = [d \in Domains |-> 0]
    /\ published = NullDomain
    /\ lookupGeneration = 0
    /\ reapedProof = [d \in Domains |-> FALSE]

Load(d) ==
    /\ phase[d] \in {"Empty", "TornDown"}
    /\ generation[d] < MaxGeneration
    /\ phase' = [phase EXCEPT ![d] = "Loaded"]
    /\ generation' = [generation EXCEPT ![d] = @ + 1]
    /\ reapedProof' = [reapedProof EXCEPT ![d] = FALSE]
    /\ UNCHANGED <<published, lookupGeneration>>

Start(d) ==
    /\ phase[d] = "Loaded"
    /\ phase' = [phase EXCEPT ![d] = "Running"]
    /\ UNCHANGED <<generation, published, lookupGeneration, reapedProof>>

\* Name-service replacement is the publication linearization point. The old
\* generation becomes merely Running and may subsequently be stopped.
Publish(d) ==
    /\ phase[d] = "Running"
    /\ LET old == published
       IN /\ phase' =
                IF old = NullDomain
                THEN [phase EXCEPT ![d] = "Published"]
                ELSE [phase EXCEPT ![old] = "Running",
                                   ![d] = "Published"]
          /\ published' = d
          /\ lookupGeneration' = generation[d]
    /\ UNCHANGED <<generation, reapedProof>>

RequestStop(d) ==
    /\ phase[d] \in {"Running", "Published"}
    /\ phase' = [phase EXCEPT ![d] = "Stopping"]
    /\ published' = IF published = d THEN NullDomain ELSE published
    /\ lookupGeneration' = IF published = d THEN 0 ELSE lookupGeneration
    /\ UNCHANGED <<generation, reapedProof>>

Exit(d) ==
    /\ phase[d] = "Stopping"
    /\ phase' = [phase EXCEPT ![d] = "Exited"]
    /\ UNCHANGED <<generation, published, lookupGeneration, reapedProof>>

\* Separate scheduler transition: removed from master table and then dropped
\* from the per-LP DEAD_THREADS list.
Reap(d) ==
    /\ phase[d] = "Exited"
    /\ phase' = [phase EXCEPT ![d] = "Reaped"]
    /\ reapedProof' = [reapedProof EXCEPT ![d] = TRUE]
    /\ UNCHANGED <<generation, published, lookupGeneration>>

Teardown(d) ==
    /\ phase[d] = "Reaped"
    /\ phase' = [phase EXCEPT ![d] = "TornDown"]
    /\ UNCHANGED <<generation, published, lookupGeneration, reapedProof>>

Next ==
    \/ \E d \in Domains : Load(d)
    \/ \E d \in Domains : Start(d)
    \/ \E d \in Domains : Publish(d)
    \/ \E d \in Domains : RequestStop(d)
    \/ \E d \in Domains : Exit(d)
    \/ \E d \in Domains : Reap(d)
    \/ \E d \in Domains : Teardown(d)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ phase \in [Domains -> Phase]
    /\ generation \in [Domains -> 0..MaxGeneration]
    /\ published \in Domains \cup {NullDomain}
    /\ lookupGeneration \in 0..MaxGeneration
    /\ reapedProof \in [Domains -> BOOLEAN]

UniquePublication ==
    Cardinality({d \in Domains : phase[d] = "Published"}) <= 1

PublicationConsistent ==
    /\ (published = NullDomain) <=> lookupGeneration = 0
    /\ published /= NullDomain =>
         /\ phase[published] = "Published"
         /\ lookupGeneration = generation[published]

TeardownAfterReap ==
    \A d \in Domains :
        phase[d] = "TornDown" => reapedProof[d]

GenerationIsLive ==
    \A d \in Domains :
        phase[d] /= "Empty" => generation[d] > 0

Invariants ==
    /\ TypeOK
    /\ UniquePublication
    /\ PublicationConsistent
    /\ TeardownAfterReap
    /\ GenerationIsLive

=============================================================================
