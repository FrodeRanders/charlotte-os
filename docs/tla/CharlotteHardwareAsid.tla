---------------------- MODULE CharlotteHardwareAsid ------------------------
\* Hardware TLB-tag allocation, retirement, invalidation, and safe reuse.

EXTENDS FiniteSets

CONSTANTS Domains, Tags, NullDomain, NullTag

ASSUME NullDomain \notin Domains
ASSUME NullTag \notin Tags

VARIABLES owner, assigned, staleTranslations, reuseViolation

vars == <<owner, assigned, staleTranslations, reuseViolation>>

Init ==
    /\ owner = [t \in Tags |-> NullDomain]
    /\ assigned = [d \in Domains |-> NullTag]
    /\ staleTranslations = {}
    /\ reuseViolation = FALSE

Allocate(d, t) ==
    /\ assigned[d] = NullTag
    /\ owner[t] = NullDomain
    /\ t \notin staleTranslations
    /\ owner' = [owner EXCEPT ![t] = d]
    /\ assigned' = [assigned EXCEPT ![d] = t]
    /\ UNCHANGED <<staleTranslations, reuseViolation>>

CacheTranslation(d) ==
    /\ assigned[d] \in Tags
    /\ staleTranslations' = staleTranslations \cup {assigned[d]}
    /\ UNCHANGED <<owner, assigned, reuseViolation>>

Retire(d) ==
    /\ assigned[d] \in Tags
    /\ LET tag == assigned[d]
       IN /\ owner' = [owner EXCEPT ![tag] = NullDomain]
          /\ assigned' = [assigned EXCEPT ![d] = NullTag]
    /\ UNCHANGED <<staleTranslations, reuseViolation>>

Invalidate(t) ==
    /\ t \in staleTranslations
    /\ staleTranslations' = staleTranslations \ {t}
    /\ UNCHANGED <<owner, assigned, reuseViolation>>

\* The bug: recycle a free tag while translations from its old page-table
\* lifetime can still match in a TLB.
UnsafeAllocate(d, t) ==
    /\ assigned[d] = NullTag
    /\ owner[t] = NullDomain
    /\ owner' = [owner EXCEPT ![t] = d]
    /\ assigned' = [assigned EXCEPT ![d] = t]
    /\ reuseViolation' = (reuseViolation \/ (t \in staleTranslations))
    /\ UNCHANGED staleTranslations

SafeNext ==
    \/ \E d \in Domains, t \in Tags : Allocate(d, t)
    \/ \E d \in Domains : CacheTranslation(d)
    \/ \E d \in Domains : Retire(d)
    \/ \E t \in Tags : Invalidate(t)

UnsafeNext ==
    \/ \E d \in Domains, t \in Tags : UnsafeAllocate(d, t)
    \/ \E d \in Domains : CacheTranslation(d)
    \/ \E d \in Domains : Retire(d)
    \/ \E t \in Tags : Invalidate(t)

Spec == Init /\ [][SafeNext]_vars
UnsafeSpec == Init /\ [][UnsafeNext]_vars

TypeOK ==
    /\ owner \in [Tags -> Domains \cup {NullDomain}]
    /\ assigned \in [Domains -> Tags \cup {NullTag}]
    /\ staleTranslations \subseteq Tags
    /\ reuseViolation \in BOOLEAN

OwnershipConsistent ==
    \A d \in Domains :
        assigned[d] \in Tags => owner[assigned[d]] = d

NoDirtyTagReuse == ~reuseViolation

Invariants == TypeOK /\ OwnershipConsistent /\ NoDirtyTagReuse

=============================================================================
