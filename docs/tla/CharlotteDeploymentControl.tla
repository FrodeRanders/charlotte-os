-------------------- MODULE CharlotteDeploymentControl --------------------
\* Already admitted controller calls. A ready healthy response must remain
\* observable while another call is pending. Receiver backing survives caller
\* timeout until the receiver separately releases it. No physical proof here.
EXTENDS Naturals, FiniteSets
CONSTANTS Clients, Budget
Slots == 1..Clients
VARIABLES active, bound, completed, owners, now, deadline, submissions,
          receiverBacking, terminal, phase, timeoutSeen
vars == <<active, bound, completed, owners, now, deadline, submissions,
          receiverBacking, terminal, phase, timeoutSeen>>
Init ==
    /\ active = Slots /\ bound = {} /\ completed = {} /\ owners = Slots
    /\ now = 0 /\ deadline = [i \in Slots |-> Budget]
    /\ submissions = [i \in Slots |-> 1]
    /\ receiverBacking = Slots /\ terminal = {} /\ phase = "Running"
    /\ timeoutSeen = {}
Bind ==
    /\ phase = "Running"
    /\ \E i \in active \ bound : bound' = bound \cup {i}
    /\ UNCHANGED <<active, completed, owners, now, deadline, submissions,
                   receiverBacking, terminal, phase, timeoutSeen>>
Complete ==
    /\ phase # "Aborted"
    /\ \E i \in active \ completed : completed' = completed \cup {i}
    /\ phase' = "Running"
    /\ UNCHANGED <<active, bound, owners, now, deadline, submissions,
                   receiverBacking, terminal, timeoutSeen>>
Observe ==
    /\ phase = "Running"
    /\ \E i \in active \cap completed :
        /\ active' = active \ {i} /\ bound' = bound \ {i} /\ owners' = owners \ {i}
        /\ completed' = completed \ {i} /\ terminal' = terminal \cup {i}
    /\ UNCHANGED <<now, deadline, submissions, receiverBacking, phase, timeoutSeen>>
Expire ==
    /\ phase = "Running"
    /\ \E i \in active :
        /\ now >= deadline[i]
        /\ active' = active \ {i} /\ bound' = bound \ {i} /\ owners' = owners \ {i}
        /\ completed' = completed \ {i} /\ terminal' = terminal \cup {i}
        /\ timeoutSeen' = timeoutSeen \cup {i}
    /\ UNCHANGED <<now, deadline, submissions, receiverBacking, phase>>
ReleaseReceiver ==
    /\ \E i \in receiverBacking : receiverBacking' = receiverBacking \ {i}
    /\ UNCHANGED <<active, bound, completed, owners, now, deadline,
                   submissions, terminal, phase, timeoutSeen>>
Park ==
    /\ phase = "Running" /\ active # {} /\ bound = active
    /\ completed = {} /\ \A i \in active : now < deadline[i]
    /\ phase' = "Parked"
    /\ UNCHANGED <<active, bound, completed, owners, now, deadline,
                   submissions, receiverBacking, terminal, timeoutSeen>>
Tick ==
    /\ phase # "Aborted" /\ now < Budget
    /\ now' = now + 1 /\ phase' = "Running"
    /\ UNCHANGED <<active, bound, completed, owners, deadline, submissions,
                   receiverBacking, terminal, timeoutSeen>>
RejectAdmission ==
    /\ phase = "Running" /\ active # {}
    /\ phase' = "Aborted"
    /\ UNCHANGED <<active, bound, completed, owners, now, deadline,
                   submissions, receiverBacking, terminal, timeoutSeen>>
UnsafeReplay ==
    /\ phase = "Running" /\ timeoutSeen # {}
    /\ \E i \in timeoutSeen : submissions' = [submissions EXCEPT ![i] = 2]
    /\ UNCHANGED <<active, bound, completed, owners, now, deadline,
                   receiverBacking, terminal, phase, timeoutSeen>>
UnsafeSerialWait ==
    /\ phase = "Running" /\ active = Slots /\ bound = active
    /\ completed # {} /\ completed # active
    /\ phase' = "Parked"
    /\ UNCHANGED <<active, bound, completed, owners, now, deadline,
                   submissions, receiverBacking, terminal, timeoutSeen>>
Next == Bind \/ Complete \/ Observe \/ Expire \/ ReleaseReceiver \/ Park \/ Tick \/ RejectAdmission
Spec == Init /\ [][Next]_vars
UnsafeReplaySpec == Init /\ [][Next \/ UnsafeReplay]_vars
UnsafeSerialSpec == Init /\ [][Next \/ UnsafeSerialWait]_vars
TypeOK ==
    /\ active \subseteq Slots /\ bound \subseteq active /\ completed \subseteq active
    /\ owners = active /\ now \in 0..Budget
    /\ deadline = [i \in Slots |-> Budget] /\ submissions \in [Slots -> 1..2]
    /\ receiverBacking \subseteq Slots /\ terminal = Slots \ active
    /\ timeoutSeen \subseteq terminal /\ phase \in {"Running", "Parked", "Aborted"}
SubmittedOnce == \A i \in Slots : submissions[i] = 1
ParkCoversAll == phase = "Parked" => bound = active /\ completed = {}
Invariants == TypeOK /\ SubmittedOnce /\ ParkCoversAll
=============================================================================
