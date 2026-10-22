------------------ MODULE CharlotteDeploymentListener ------------------
\* Fixed positions, one original setup budget and one accepted call per phase.
\* Replies/binding are abstract producer contracts. A completed position moves
\* to input/close; this projection grants no physical socket cleanup proof.
EXTENDS Naturals, FiniteSets
CONSTANTS Clients, Budget
Slots == 1..Clients
Stages == {"Open", "Listen", "Accept"}
VARIABLES stage, status, accepted, socket, owners, now, deadline, phase, retryAt
vars == <<stage, status, accepted, socket, owners, now, deadline, phase, retryAt>>
Init ==
    /\ stage = [i \in Slots |-> "Open"]
    /\ status = [i \in Slots |-> "Unsubmitted"]
    /\ accepted = [i \in Slots |-> [s \in Stages |-> 0]]
    /\ socket = {} /\ owners = Slots /\ now = 0
    /\ deadline = [i \in Slots |-> Budget] /\ phase = "Running"
    /\ retryAt = [i \in Slots |-> 0]
Admit ==
    /\ phase = "Running" /\ now < Budget
    /\ \E i \in owners :
        /\ status[i] = "Unsubmitted" /\ now >= retryAt[i]
        /\ status' = [status EXCEPT ![i] = "Pending"]
        /\ accepted' = [accepted EXCEPT ![i][stage[i]] = @ + 1]
    /\ UNCHANGED <<stage, socket, owners, now, deadline, phase, retryAt>>
RejectQueue ==
    /\ phase = "Running" /\ now < Budget
    /\ \E i \in owners :
        /\ status[i] = "Unsubmitted" /\ now >= retryAt[i]
        /\ retryAt' = [retryAt EXCEPT ![i] = now + 1]
    /\ UNCHANGED <<stage, status, accepted, socket, owners, now, deadline, phase>>
ProducerReply ==
    /\ phase # "Aborted"
    /\ \E i \in owners :
        /\ status[i] = "Pending"
        /\ status' = [status EXCEPT ![i] = "Ready"]
    /\ phase' = "Running"
    /\ UNCHANGED <<stage, accepted, socket, owners, now, deadline, retryAt>>
Observe ==
    /\ phase = "Running"
    /\ \E i \in owners :
        /\ status[i] = "Ready"
        /\ stage[i] = "Accept" \/ now < deadline[i]
        /\ socket' = IF stage[i] = "Open" THEN socket \cup {i}
                      ELSE IF stage[i] = "Accept" THEN socket \ {i} ELSE socket
        /\ owners' = IF stage[i] = "Accept" THEN owners \ {i} ELSE owners
        /\ status' = [status EXCEPT ![i] = IF stage[i] = "Accept" THEN "Done" ELSE "Unsubmitted"]
        /\ stage' = [stage EXCEPT ![i] = IF @ = "Open" THEN "Listen" ELSE "Accept"]
    /\ UNCHANGED <<accepted, now, deadline, phase, retryAt>>
RejectConfirmed ==
    /\ phase = "Running"
    /\ \E i \in owners :
        /\ status[i] = "Ready"
        /\ socket' = socket \ {i} /\ owners' = owners \ {i}
        /\ status' = [status EXCEPT ![i] = "Done"]
    /\ UNCHANGED <<stage, accepted, now, deadline, phase, retryAt>>
Abort ==
    /\ phase = "Running" /\ owners # {} /\ phase' = "Aborted"
    /\ UNCHANGED <<stage, status, accepted, socket, owners, now, deadline, retryAt>>
IdleAccept(i) == stage[i] = "Accept" /\ status[i] \in {"Pending", "Ready"}
CanPark ==
    /\ owners # {}
    /\ \A i \in owners :
        /\ status[i] # "Ready"
        /\ IdleAccept(i) \/ now < deadline[i]
        /\ status[i] = "Pending" \/ (status[i] = "Unsubmitted" /\ now < retryAt[i])
Park ==
    /\ phase = "Running" /\ CanPark /\ phase' = "Parked"
    /\ UNCHANGED <<stage, status, accepted, socket, owners, now, deadline, retryAt>>
Wake ==
    /\ phase = "Parked" /\ phase' = "Running"
    /\ UNCHANGED <<stage, status, accepted, socket, owners, now, deadline, retryAt>>
Tick ==
    /\ phase # "Aborted" /\ now < Budget /\ now' = now + 1
    /\ phase' = "Running"
    /\ UNCHANGED <<stage, status, accepted, socket, owners, deadline, retryAt>>
UnsafeReplay ==
    /\ phase = "Running"
    /\ \E i \in owners :
        /\ status[i] \in {"Pending", "Ready"}
        /\ accepted' = [accepted EXCEPT ![i][stage[i]] = 2]
    /\ UNCHANGED <<stage, status, socket, owners, now, deadline, phase, retryAt>>
UnsafeRestart ==
    /\ phase = "Running"
    /\ \E i \in owners :
        /\ stage[i] = "Listen" /\ now > 0
        /\ deadline' = [deadline EXCEPT ![i] = now + Budget]
    /\ UNCHANGED <<stage, status, accepted, socket, owners, now, phase, retryAt>>
UnsafeSerialPark ==
    /\ phase = "Running"
    /\ \E i \in owners : status[i] = "Pending"
    /\ \E j \in owners : status[j] = "Ready"
    /\ phase' = "Parked"
    /\ UNCHANGED <<stage, status, accepted, socket, owners, now, deadline, retryAt>>
Next == Admit \/ RejectQueue \/ ProducerReply \/ Observe \/ RejectConfirmed \/ Abort \/ Park \/ Wake \/ Tick
Spec == Init /\ [][Next]_vars
ReplaySpec == Init /\ [][Next \/ UnsafeReplay]_vars
RestartSpec == Init /\ [][Next \/ UnsafeRestart]_vars
SerialSpec == Init /\ [][Next \/ UnsafeSerialPark]_vars
TypeOK ==
    /\ stage \in [Slots -> Stages]
    /\ status \in [Slots -> {"Unsubmitted", "Pending", "Ready", "Done"}]
    /\ accepted \in [Slots -> [Stages -> 0..2]]
    /\ socket \subseteq owners /\ owners \subseteq Slots
    /\ now \in 0..Budget /\ retryAt \in [Slots -> 0..Budget]
    /\ deadline \in [Slots -> 0..(2 * Budget)]
    /\ phase \in {"Running", "Parked", "Aborted"}
AcceptedOnce == \A i \in Slots : \A s \in Stages : accepted[i][s] <= 1
OriginalBudget == \A i \in Slots : deadline[i] = Budget
SocketAfterObservation == \A i \in owners : (i \in socket) = (stage[i] # "Open")
OwnersUntilObserved == owners = {i \in Slots : status[i] # "Done"}
ParkCoversAll == phase = "Parked" => CanPark
Invariants == TypeOK /\ AcceptedOnce /\ OriginalBudget /\ SocketAfterObservation /\ OwnersUntilObserved /\ ParkCoversAll
=============================================================================
