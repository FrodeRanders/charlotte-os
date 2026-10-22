---------------------- MODULE CharlotteDeploymentInput ----------------------
\* Bounded input scheduling only. One healthy complete request competes with
\* incomplete readable inputs. A turn abstracts poll/observe/append/rearm; it
\* does not abstract controller, send, close, TCP or kernel scheduling latency.
EXTENDS Naturals, FiniteSets
CONSTANTS Clients, ByteBudget, Budget
ASSUME Clients > 1 /\ ByteBudget > Clients /\ Budget > 0
Slots == 1..Clients
Healthy == Clients
VARIABLES active, ready, bound, owner, buffered, cursor, now, deadline,
          healthyAge, phase
vars == <<active, ready, bound, owner, buffered, cursor, now, deadline,
          healthyAge, phase>>
TotalBytes == Cardinality(UNION {{i} \X (1..buffered[i]) : i \in Slots})
Distance(i) == (i + Clients - cursor) % Clients
Eligible == {i \in active : i \in ready \/ now >= deadline[i]}
First == CHOOSE i \in Eligible : \A j \in Eligible : Distance(i) <= Distance(j)

Init ==
    /\ active = Slots /\ owner = Slots /\ bound = Slots
    /\ ready = {1, Healthy}
    /\ buffered = [i \in Slots |-> 0]
    /\ deadline = [i \in Slots |-> Budget]
    /\ cursor = 1 /\ now = 0 /\ healthyAge = 0 /\ phase = "Running"

ProducerReady ==
    /\ phase \in {"Running", "Parked"}
    /\ \E i \in active \ ready :
        /\ ready' = ready \cup {i}
        /\ phase' = "Running"
    /\ UNCHANGED <<active, bound, owner, buffered, cursor, now, deadline, healthyAge>>

Turn(rotate, restart) ==
    /\ phase = "Running" /\ Eligible # {}
    /\ LET i == First
           terminal == i = Healthy \/ now >= deadline[i] \/ TotalBytes = ByteBudget
       IN /\ active' = IF terminal THEN active \ {i} ELSE active
          /\ owner' = active' /\ bound' = active'
          /\ ready' = IF terminal THEN ready \ {i} ELSE ready
          /\ buffered' = [buffered EXCEPT ![i] = IF terminal THEN 0 ELSE @ + 1]
          /\ cursor' = IF rotate THEN (i % Clients) + 1 ELSE cursor
          /\ deadline' = IF restart /\ ~terminal
                          THEN [deadline EXCEPT ![i] = now + Budget] ELSE deadline
          /\ healthyAge' = IF Healthy \in active /\ Healthy \in ready /\ i # Healthy
                            THEN healthyAge + 1 ELSE 0
    /\ UNCHANGED <<now, phase>>
Advance == /\ TRUE /\ Turn(TRUE, FALSE)
UnsafeKeepCursor == /\ TRUE /\ Turn(FALSE, FALSE)
UnsafeRestartBudget == /\ TRUE /\ Turn(TRUE, TRUE)

Tick ==
    /\ phase \in {"Running", "Parked"} /\ now < Budget
    /\ now' = now + 1 /\ phase' = "Running"
    /\ UNCHANGED <<active, ready, bound, owner, buffered, cursor, deadline, healthyAge>>

Park ==
    /\ phase = "Running" /\ active # {} /\ Eligible = {} /\ bound = active
    /\ phase' = "Parked"
    /\ UNCHANGED <<active, ready, bound, owner, buffered, cursor, now, deadline, healthyAge>>

RejectWait ==
    /\ phase = "Running" /\ Eligible = {} /\ active # {}
    /\ phase' = "Aborted"
    /\ UNCHANGED <<active, ready, bound, owner, buffered, cursor, now, deadline, healthyAge>>

Next == Advance \/ ProducerReady \/ Tick \/ Park \/ RejectWait
Spec == Init /\ [][Next]_vars
UnsafeCursorSpec == Init /\ [][Next \/ UnsafeKeepCursor]_vars
UnsafeDeadlineSpec == Init /\ [][Next \/ UnsafeRestartBudget]_vars

TypeOK ==
    /\ active \subseteq Slots /\ ready \subseteq active /\ bound = active /\ owner = active
    /\ buffered \in [Slots -> 0..ByteBudget]
    /\ cursor \in Slots /\ now \in 0..Budget
    /\ deadline \in [Slots -> 0..(2 * Budget)]
    /\ healthyAge \in 0..Clients
    /\ phase \in {"Running", "Parked", "Aborted"}
BoundedBytes == TotalBytes <= ByteBudget
OriginalBudget == \A i \in Slots : deadline[i] = Budget
HealthyTurnBound == healthyAge < Clients
ParkHasAllInterests == phase = "Parked" => bound = active
Invariants == TypeOK /\ BoundedBytes /\ OriginalBudget /\ HealthyTurnBound /\ ParkHasAllInterests
=============================================================================
