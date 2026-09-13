------------------------ MODULE CharlotteStackGrowth -------------------------
\* Demand-grown protected-domain stacks.
\*
\* A fault below the committed stack extends it by one page while the domain's
\* budget (its signed or adaptively chosen maximum) allows and frames are free.
\* Exhausting the budget on a fault kills the domain, which must return every
\* committed frame exactly once. The model abstracts faults, growth, clean
\* exit, and kill; it is the resource-accounting core a recoverable EL0 guard
\* fault implements.

EXTENDS Naturals, FiniteSets

CONSTANTS Domains, MaxBudget, TotalFrames, ReserveFrames

ASSUME MaxBudget > 0
ASSUME TotalFrames >= 1
ASSUME ReserveFrames < TotalFrames
ASSUME IsFiniteSet(Domains)

VARIABLES alive, budget, committed, freeFrames, faults, kills

vars == <<alive, budget, committed, freeFrames, faults, kills>>

RECURSIVE DomainSum(_)
DomainSum(S) ==
    IF S = {} THEN 0
    ELSE LET d == CHOOSE x \in S: TRUE IN committed[d] + DomainSum(S \ {d})

Init ==
    /\ alive = [d \in Domains |-> TRUE]
    /\ budget = [d \in Domains |-> MaxBudget]
    /\ committed = [d \in Domains |-> 0]
    /\ freeFrames = TotalFrames
    /\ faults = [d \in Domains |-> 0]
    /\ kills = [d \in Domains |-> 0]

\* A guard-page fault arrives for the domain. It stays pending until a growth
\* or a kill resolves it.
Fault(d) ==
    /\ alive[d]
    /\ faults[d] < 3
    /\ faults' = [faults EXCEPT ![d] = @ + 1]
    /\ UNCHANGED <<alive, budget, committed, freeFrames, kills>>

\* The safe path: charge one page from the domain's budget and the free pool.
Grow(d) ==
    /\ alive[d]
    /\ faults[d] > 0
    /\ committed[d] < budget[d]
    /\ freeFrames > ReserveFrames
    /\ committed' = [committed EXCEPT ![d] = @ + 1]
    /\ freeFrames' = freeFrames - 1
    /\ faults' = [faults EXCEPT ![d] = @ - 1]
    /\ UNCHANGED <<alive, budget, kills>>

\* Fail closed when the budget is exhausted: the domain dies and every
\* committed frame returns to the pool.
Kill(d) ==
    /\ alive[d]
    /\ faults[d] > 0
    /\ committed[d] = budget[d]
    /\ alive' = [alive EXCEPT ![d] = FALSE]
    /\ freeFrames' = freeFrames + committed[d]
    /\ committed' = [committed EXCEPT ![d] = 0]
    /\ faults' = [faults EXCEPT ![d] = 0]
    /\ kills' = [kills EXCEPT ![d] = @ + 1]
    /\ UNCHANGED budget

\* A clean exit with no outstanding fault releases the stack.
Exit(d) ==
    /\ alive[d]
    /\ faults[d] = 0
    /\ alive' = [alive EXCEPT ![d] = FALSE]
    /\ freeFrames' = freeFrames + committed[d]
    /\ committed' = [committed EXCEPT ![d] = 0]
    /\ UNCHANGED <<budget, faults, kills>>

CommittedWithinBudget == \A d \in Domains: committed[d] <= budget[d]

FrameConservation == freeFrames + DomainSum(Domains) = TotalFrames

DeadDomainsReleaseFrames == \A d \in Domains: ~alive[d] => committed[d] = 0

FreeReservePreserved == freeFrames >= ReserveFrames

Invariants ==
    /\ CommittedWithinBudget
    /\ FrameConservation
    /\ DeadDomainsReleaseFrames
    /\ FreeReservePreserved

SafeNext ==
    \/ \E d \in Domains: Fault(d)
    \/ \E d \in Domains: Grow(d)
    \/ \E d \in Domains: Kill(d)
    \/ \E d \in Domains: Exit(d)

\* The bug: growth ignores the budget and consumes frames without bound.
UnsafeGrowOverBudget(d) ==
    /\ alive[d]
    /\ faults[d] > 0
    /\ committed[d] >= budget[d]
    /\ freeFrames > 0
    /\ committed' = [committed EXCEPT ![d] = @ + 1]
    /\ freeFrames' = freeFrames - 1
    /\ faults' = [faults EXCEPT ![d] = @ - 1]
    /\ UNCHANGED <<alive, budget, kills>>

\* The bug: a killed domain keeps its committed frames forever.
UnsafeKillWithoutRelease(d) ==
    /\ alive[d]
    /\ faults[d] > 0
    /\ committed[d] = budget[d]
    /\ alive' = [alive EXCEPT ![d] = FALSE]
    /\ kills' = [kills EXCEPT ![d] = @ + 1]
    /\ UNCHANGED <<budget, committed, freeFrames, faults>>

Spec == Init /\ [][SafeNext]_vars
UnsafeGrowSpec ==
    Init /\ [][SafeNext \/ (\E d \in Domains: UnsafeGrowOverBudget(d))]_vars
UnsafeLeakSpec ==
    Init /\ [][SafeNext \/ (\E d \in Domains: UnsafeKillWithoutRelease(d))]_vars

=============================================================================
