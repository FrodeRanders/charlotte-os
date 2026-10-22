-------------------- MODULE CharlotteDeploymentClose --------------------
\* Logical-close ownership only. The model starts with already owned sockets.
\* ACK releases a client position, without granting physical cleanup authority.
EXTENDS Naturals, FiniteSets
CONSTANTS Clients, Budget, Style
Slots == 1..Clients
VARIABLES state, owners, confirmed, accepted, now, retryAt, phase, receipt, admissionTerminal, reentered
vars == <<state, owners, confirmed, accepted, now, retryAt, phase, receipt, admissionTerminal, reentered>>
Init ==
    /\ state = [i \in Slots |-> "Unsubmitted"] /\ owners = Slots /\ confirmed = {}
    /\ accepted = [i \in Slots |-> 0] /\ now = 0
    /\ retryAt = [i \in Slots |-> 0] /\ phase = "Running"
    /\ receipt = [i \in Slots |-> "None"]
    /\ admissionTerminal = FALSE /\ reentered = FALSE
RejectQueue ==
    /\ phase = "Running" /\ now < Budget
    /\ \E i \in owners :
        /\ state[i] = "Unsubmitted" /\ now >= retryAt[i]
        /\ retryAt' = [retryAt EXCEPT ![i] = now + 1]
    /\ UNCHANGED <<admissionTerminal, reentered, receipt, state, owners, confirmed, accepted, now, phase>>
Admit ==
    /\ phase = "Running" /\ now < Budget
    /\ \E i \in owners :
        /\ state[i] = "Unsubmitted" /\ now >= retryAt[i]
        /\ state' = [state EXCEPT ![i] = "Pending"]
        /\ accepted' = [accepted EXCEPT ![i] = @ + 1]
    /\ UNCHANGED <<owners, confirmed, now, retryAt, phase, receipt, admissionTerminal, reentered>>
ProducerAck ==
    /\ phase \notin {"Aborted", "Retained"}
    /\ \E i \in owners :
        /\ state[i] = "Pending" /\ state' = [state EXCEPT ![i] = "Ready"]
        /\ receipt' = [receipt EXCEPT ![i] = "Started"]
    /\ phase' = "Running"
    /\ UNCHANGED <<admissionTerminal, reentered, owners, confirmed, accepted, now, retryAt>>
ProducerUnavailable ==
    /\ phase \notin {"Aborted", "Retained"}
    /\ \E i \in owners :
        /\ state[i] = "Pending" /\ state' = [state EXCEPT ![i] = "Ready"]
        /\ receipt' = [receipt EXCEPT ![i] = "Unavailable"]
    /\ phase' = "Running"
    /\ UNCHANGED <<admissionTerminal, reentered, owners, confirmed, accepted, now, retryAt>>
ObserveAck ==
    /\ phase = "Running" /\ now < Budget
    /\ \E i \in owners :
        /\ state[i] = "Ready" /\ state' = [state EXCEPT ![i] = "Done"]
        /\ owners' = owners \ {i} /\ confirmed' = confirmed \cup {i}
    /\ UNCHANGED <<accepted, now, retryAt, phase, receipt, admissionTerminal, reentered>>
Expire ==
    /\ phase = "Running" /\ owners # {} /\ now = Budget
    /\ phase' = "Aborted"
    /\ UNCHANGED <<admissionTerminal, reentered, receipt, state, owners, confirmed, accepted, now, retryAt>>
RejectWaitOrObservation ==
    /\ phase = "Running" /\ owners # {}
    /\ phase' = "Aborted"
    /\ UNCHANGED <<admissionTerminal, reentered, receipt, state, owners, confirmed, accepted, now, retryAt>>
Park ==
    /\ phase = "Running" /\ owners # {} /\ now < Budget
    /\ \A i \in owners : state[i] = "Pending" \/
                           (state[i] = "Unsubmitted" /\ now < retryAt[i])
    /\ phase' = "Parked"
    /\ UNCHANGED <<admissionTerminal, reentered, receipt, state, owners, confirmed, accepted, now, retryAt>>
Wake ==
    /\ phase = "Parked" /\ phase' = "Running"
    /\ UNCHANGED <<admissionTerminal, reentered, receipt, state, owners, confirmed, accepted, now, retryAt>>
Tick ==
    /\ phase \notin {"Aborted", "Retained"} /\ now < Budget
    /\ now' = now + 1 /\ phase' = "Running"
    /\ UNCHANGED <<admissionTerminal, reentered, receipt, state, owners, confirmed, accepted, retryAt>>
UnsafeReplay ==
    /\ phase = "Running"
    /\ \E i \in owners :
        /\ state[i] \in {"Pending", "Ready"}
        /\ accepted' = [accepted EXCEPT ![i] = 2]
    /\ UNCHANGED <<state, owners, confirmed, now, retryAt, phase, receipt, admissionTerminal, reentered>>
UnsafeRelease ==
    /\ phase = "Running"
    /\ \E i \in owners :
        /\ state[i] = "Pending" /\ owners' = owners \ {i}
    /\ UNCHANGED <<state, confirmed, accepted, now, retryAt, phase, receipt, admissionTerminal, reentered>>
RetainAdmission ==
    /\ Style = "Blocking" /\ phase = "Running"
    /\ \E i \in owners : state[i] = "Unsubmitted"
    /\ phase' = "Retained" /\ admissionTerminal' = TRUE
    /\ UNCHANGED <<state, owners, confirmed, accepted, now, retryAt, receipt, reentered>>
UnsafeFallback ==
    /\ phase = "Retained" /\ admissionTerminal
    /\ phase' = "Running" /\ reentered' = TRUE
    /\ UNCHANGED <<state, owners, confirmed, accepted, now, retryAt, receipt, admissionTerminal>>
Next == RejectQueue \/ Admit \/ ProducerAck \/ ProducerUnavailable \/ ObserveAck \/ Expire \/ RejectWaitOrObservation \/ Park \/ Wake \/ Tick \/ RetainAdmission
Spec == Init /\ [][Next]_vars
ReplaySpec == Init /\ [][Next \/ UnsafeReplay]_vars
ReleaseSpec == Init /\ [][Next \/ UnsafeRelease]_vars
FallbackSpec == Init /\ [][Next \/ UnsafeFallback]_vars
TypeOK ==
    /\ admissionTerminal \in BOOLEAN /\ reentered \in BOOLEAN
    /\ receipt \in [Slots -> {"None", "Started", "Unavailable"}]
    /\ state \in [Slots -> {"Unsubmitted", "Pending", "Ready", "Done"}]
    /\ owners \subseteq Slots /\ confirmed \subseteq Slots
    /\ accepted \in [Slots -> 0..2] /\ now \in 0..Budget
    /\ retryAt \in [Slots -> 0..Budget] /\ phase \in {"Running", "Parked", "Aborted", "Retained"}
AcceptedOnce == \A i \in Slots : accepted[i] <= 1
OwnersUntilAck == owners = Slots \ confirmed
OnlyAckReleases == \A i \in confirmed : state[i] = "Done" /\ accepted[i] = 1 /\ receipt[i] # "None"
ParkCoversAll == phase = "Parked" => now < Budget /\
    \A i \in owners : state[i] = "Pending" \/ (state[i] = "Unsubmitted" /\ now < retryAt[i])
NoReentry == ~reentered
Invariants == NoReentry /\ TypeOK /\ AcceptedOnce /\ OwnersUntilAck /\ OnlyAckReleases /\ ParkCoversAll
=============================================================================
