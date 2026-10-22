-------------------- MODULE CharlotteControllerRelay --------------------
\* Two already admitted server relays, one preflight/mutation and one query.
\* Binding and producer delivery are assumed. Reply classification abstracts
\* checked absence/newer-state evidence; errors never supply that evidence.
EXTENDS Naturals, FiniteSets
CONSTANT Budget
Slots == {1, 2}
Stages == {"Preflight", "Mutation", "Query"}
VARIABLES active, stage, ready, proof, sent, now, deadline, phase, backing
vars == <<active, stage, ready, proof, sent, now, deadline, phase, backing>>
Init ==
    /\ active = Slots /\ stage = [i \in Slots |-> IF i = 1 THEN "Preflight" ELSE "Query"]
    /\ ready = {} /\ proof = FALSE
    /\ sent = [s \in Stages |-> IF s = "Mutation" THEN 0 ELSE 1]
    /\ now = 0 /\ deadline = Budget /\ phase = "Running"
    /\ backing = {"Preflight", "Query"}
ProducerReply ==
    /\ phase # "Aborted" /\ \E i \in active \ ready : ready' = ready \cup {i}
    /\ phase' = "Running"
    /\ UNCHANGED <<active, stage, proof, sent, now, deadline, backing>>
CheckedPreflight ==
    /\ phase = "Running" /\ 1 \in active \cap ready /\ stage[1] = "Preflight" /\ now < deadline
    /\ proof' = TRUE /\ stage' = [stage EXCEPT ![1] = "Mutation"]
    /\ sent' = [sent EXCEPT !["Mutation"] = @ + 1]
    /\ backing' = backing \cup {"Mutation"} /\ ready' = ready \ {1}
    /\ UNCHANGED <<active, now, deadline, phase>>
RejectPreflight ==
    /\ phase = "Running" /\ 1 \in active \cap ready /\ stage[1] = "Preflight"
    /\ active' = active \ {1} /\ ready' = ready \ {1}
    /\ UNCHANGED <<stage, proof, sent, now, deadline, phase, backing>>
ObserveTerminal ==
    /\ phase = "Running"
    /\ \E i \in active \cap ready :
        /\ stage[i] # "Preflight" /\ active' = active \ {i} /\ ready' = ready \ {i}
    /\ UNCHANGED <<stage, proof, sent, now, deadline, phase, backing>>
Expire ==
    /\ phase = "Running" /\ now = deadline /\ active # {}
    /\ active' = {} /\ ready' = {}
    /\ UNCHANGED <<stage, proof, sent, now, deadline, phase, backing>>
ReleaseReceiver ==
    /\ \E s \in backing : backing' = backing \ {s}
    /\ UNCHANGED <<active, stage, ready, proof, sent, now, deadline, phase>>
Park ==
    /\ phase = "Running" /\ active # {} /\ ready = {} /\ now < deadline
    /\ phase' = "Parked"
    /\ UNCHANGED <<active, stage, ready, proof, sent, now, deadline, backing>>
Wake ==
    /\ phase = "Parked" /\ phase' = "Running"
    /\ UNCHANGED <<active, stage, ready, proof, sent, now, deadline, backing>>
Tick ==
    /\ phase # "Aborted" /\ now < Budget /\ now' = now + 1 /\ phase' = "Running"
    /\ UNCHANGED <<active, stage, ready, proof, sent, deadline, backing>>
AbortBindingOrWait ==
    /\ phase = "Running" /\ active # {} /\ phase' = "Aborted"
    /\ UNCHANGED <<active, stage, ready, proof, sent, now, deadline, backing>>
UnsafeUnknownAsMissing ==
    /\ phase = "Running" /\ 1 \in active \cap ready /\ stage[1] = "Preflight" /\ now < deadline
    /\ stage' = [stage EXCEPT ![1] = "Mutation"] /\ ready' = ready \ {1}
    /\ sent' = [sent EXCEPT !["Mutation"] = 1] /\ backing' = backing \cup {"Mutation"}
    /\ UNCHANGED <<active, proof, now, deadline, phase>>
UnsafeRestart ==
    /\ phase = "Running" /\ stage[1] = "Mutation" /\ now > 0
    /\ deadline' = now + Budget
    /\ UNCHANGED <<active, stage, ready, proof, sent, now, phase, backing>>
UnsafeSerialWait ==
    /\ phase = "Running" /\ active = Slots /\ ready # {} /\ ready # active
    /\ phase' = "Parked"
    /\ UNCHANGED <<active, stage, ready, proof, sent, now, deadline, backing>>
UnsafeReplay ==
    /\ phase = "Running" /\ sent["Mutation"] = 1
    /\ sent' = [sent EXCEPT !["Mutation"] = 2]
    /\ UNCHANGED <<active, stage, ready, proof, now, deadline, phase, backing>>
Next == ProducerReply \/ CheckedPreflight \/ RejectPreflight \/ ObserveTerminal \/ Expire \/ ReleaseReceiver \/ Park \/ Wake \/ Tick \/ AbortBindingOrWait
Spec == Init /\ [][Next]_vars
UnknownSpec == Init /\ [][Next \/ UnsafeUnknownAsMissing]_vars
RestartSpec == Init /\ [][Next \/ UnsafeRestart]_vars
SerialSpec == Init /\ [][Next \/ UnsafeSerialWait]_vars
ReplaySpec == Init /\ [][Next \/ UnsafeReplay]_vars
TypeOK ==
    /\ active \subseteq Slots /\ ready \subseteq active /\ stage \in [Slots -> Stages]
    /\ proof \in BOOLEAN /\ sent \in [Stages -> 0..2] /\ now \in 0..Budget
    /\ deadline \in 0..(2 * Budget) /\ phase \in {"Running", "Parked", "Aborted"}
    /\ backing \subseteq Stages
MutationRequiresCheckedQuery == sent["Mutation"] > 0 => proof
OriginalBudget == deadline = Budget
AcceptedOnce == \A s \in Stages : sent[s] <= 1
ParkCoversAll == phase = "Parked" => ready = {} /\ now < deadline
Invariants == TypeOK /\ MutationRequiresCheckedQuery /\ OriginalBudget /\ AcceptedOnce /\ ParkCoversAll
=============================================================================
