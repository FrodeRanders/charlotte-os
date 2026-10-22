-------------------- MODULE CharlotteDeploymentOutput --------------------
\* Two bounded output owners. A producer receipt acknowledges one enqueued
\* prefix; wakes alone cannot advance it. Cancellation leaves enqueue uncertain.
EXTENDS Naturals, FiniteSets
CONSTANTS Clients, Budget, Chunks
Slots == 1..Clients
Offsets == 0..(Chunks - 1)
VARIABLES active, ready, confirmed, attempts, now, deadline, phase
vars == <<active, ready, confirmed, attempts, now, deadline, phase>>
Init ==
    /\ active = Slots /\ ready = {}
    /\ confirmed = [i \in Slots |-> 0]
    /\ attempts = [i \in Slots |-> [j \in Offsets |-> IF j = 0 THEN 1 ELSE 0]]
    /\ now = 0 /\ deadline = [i \in Slots |-> Budget] /\ phase = "Running"
ProducerReceipt ==
    /\ phase # "Aborted"
    /\ \E i \in active \ ready : ready' = ready \cup {i}
    /\ phase' = "Running"
    /\ UNCHANGED <<active, confirmed, attempts, now, deadline>>
ObservePrefix ==
    /\ phase = "Running"
    /\ \E i \in active \cap ready :
        /\ now < deadline[i]
        /\ confirmed' = [confirmed EXCEPT ![i] = @ + 1]
        /\ ready' = ready \ {i}
        /\ active' = IF confirmed[i] + 1 = Chunks THEN active \ {i} ELSE active
        /\ attempts' = IF confirmed[i] + 1 = Chunks THEN attempts
                       ELSE [attempts EXCEPT ![i][confirmed[i] + 1] = @ + 1]
    /\ UNCHANGED <<now, deadline, phase>>
EndUncertain ==
    /\ phase = "Running"
    /\ \E i \in active : active' = active \ {i} /\ ready' = ready \ {i}
    /\ UNCHANGED <<confirmed, attempts, now, deadline, phase>>
Tick ==
    /\ phase # "Aborted" /\ now < Budget
    /\ now' = now + 1 /\ phase' = "Running"
    /\ UNCHANGED <<active, ready, confirmed, attempts, deadline>>
Park ==
    /\ phase = "Running" /\ active # {} /\ ready = {}
    /\ \A i \in active : now < deadline[i]
    /\ phase' = "Parked"
    /\ UNCHANGED <<active, ready, confirmed, attempts, now, deadline>>
Wake ==
    /\ phase = "Parked" /\ phase' = "Running"
    /\ UNCHANGED <<active, ready, confirmed, attempts, now, deadline>>
RejectWait ==
    /\ phase = "Running" /\ active # {} /\ phase' = "Aborted"
    /\ UNCHANGED <<active, ready, confirmed, attempts, now, deadline>>
UnsafeReplay ==
    /\ phase = "Running"
    /\ \E i \in Slots \ active :
        /\ confirmed[i] < Chunks
        /\ attempts' = [attempts EXCEPT ![i][confirmed[i]] = 2]
    /\ UNCHANGED <<active, ready, confirmed, now, deadline, phase>>
UnsafeRestartBudget ==
    /\ phase = "Running" /\ now > 0 /\ active # {}
    /\ \E i \in active : deadline' = [deadline EXCEPT ![i] = Budget + 1]
    /\ UNCHANGED <<active, ready, confirmed, attempts, now, phase>>
Next == ProducerReceipt \/ ObservePrefix \/ EndUncertain \/ Tick \/ Park \/ Wake \/ RejectWait
Spec == Init /\ [][Next]_vars
ReplaySpec == Init /\ [][Next \/ UnsafeReplay]_vars
BudgetSpec == Init /\ [][Next \/ UnsafeRestartBudget]_vars
TypeOK ==
    /\ active \subseteq Slots /\ ready \subseteq active
    /\ confirmed \in [Slots -> 0..Chunks]
    /\ attempts \in [Slots -> [Offsets -> 0..2]]
    /\ now \in 0..Budget /\ deadline \in [Slots -> Budget..(Budget + 1)]
    /\ phase \in {"Running", "Parked", "Aborted"}
PrefixesOnce == \A i \in Slots : \A j \in Offsets : attempts[i][j] <= 1
OriginalBudget == deadline = [i \in Slots |-> Budget]
ParkCoversAll == phase = "Parked" => ready = {} /\ \A i \in active : now < deadline[i]
Invariants == TypeOK /\ PrefixesOnce /\ OriginalBudget /\ ParkCoversAll
=============================================================================
