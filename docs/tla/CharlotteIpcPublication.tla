----------------------- MODULE CharlotteIpcPublication -----------------------
\* One owning-memory plus delegated-connection transaction. Shared authority
\* publication and payload delivery are separate boundaries. Namespace identity
\* is an account/generation pair even when numeric handles are reused.

EXTENDS Naturals, FiniteSets

CONSTANTS MaxSerial, NamespaceLimit, NodeLimit
ASSUME MaxSerial >= 4
ASSUME NamespaceLimit >= 3
ASSUME NodeLimit >= 6

Roles == {"Sender", "Receiver"}
Generations == 1..2
Namespaces == Roles \X Generations
Caps == 1..MaxSerial
Items == {"Memory", "Connection"}
Authority == {"Absent", "Staged", "Live", "Escrow"}
Payloads == {"None", "SourceMemory", "ConnectionSource", "Memory",
             "Connection", "Reply", "Filler"}
Deliveries == {"None", "Pending", "Visible"}
Phases == {"Idle", "Preparing", "Rejected", "RolledBack", "HiddenQueue",
           "HiddenResult", "Receiving", "Delivered", "Observed",
           "Cancelled", "Abandoned"}

VARIABLE st
vars == <<st>>

InitialSource == <<"Sender", 1>>
InitialTarget == <<"Receiver", 1>>
EmptyAuthority == [c \in Caps |-> "Absent"]
EmptyPayload == [c \in Caps |-> "None"]
EmptyDelivery == [c \in Caps |-> "None"]

Init == st = [
    phase |-> "Idle", path |-> "Queue", mode |-> "Move",
    current |-> [role \in Roles |-> 1],
    namespace |-> [ns \in Namespaces |-> IF ns[2] = 1 THEN "Live" ELSE "Unused"],
    serial |-> [ns \in Namespaces |-> IF ns = InitialSource THEN 3 ELSE 1],
    issued |-> [ns \in Namespaces |-> IF ns = InitialSource THEN {1, 2} ELSE {}],
    authority |-> [ns \in Namespaces |->
        IF ns = InitialSource THEN [EmptyAuthority EXCEPT ![1] = "Live", ![2] = "Live"]
        ELSE EmptyAuthority],
    payload |-> [ns \in Namespaces |->
        IF ns = InitialSource THEN [EmptyPayload EXCEPT ![1] = "SourceMemory", ![2] = "ConnectionSource"]
        ELSE EmptyPayload],
    delivery |-> [ns \in Namespaces |->
        IF ns = InitialSource THEN [EmptyDelivery EXCEPT ![1] = "Visible", ![2] = "Visible"]
        ELSE EmptyDelivery],
    charged |-> [ns \in Namespaces |-> IF ns = InitialSource THEN {1, 2} ELSE {}],
    retired |-> [ns \in Namespaces |-> {}],
    source |-> InitialSource, target |-> InitialTarget,
    reservation |-> [item \in Items |-> 0],
    storage |-> {}, prepared |-> {}, pin |-> FALSE, privateCopy |-> FALSE,
    committed |-> FALSE, delivered |-> FALSE, readyHint |-> FALSE,
    reply |-> 0, receiveRejected |-> FALSE, receiveFailed |-> FALSE,
    repeated |-> FALSE, used |-> {}, closed |-> {}
]

Current(role) == <<role, st.current[role]>>
ExactLive(ns) == ns = Current(ns[1]) /\ st.namespace[ns] = "Live"
NodeUsed == Cardinality({pair \in Namespaces \X Caps : pair[2] \in st.charged[pair[1]]})
CanReserve(ns) == ExactLive(ns) /\ st.serial[ns] \in Caps
    /\ Cardinality(st.charged[ns]) < NamespaceLimit /\ NodeUsed < NodeLimit
Reserved == {st.reservation[item] : item \in Items} \ {0}
Present(ns) == {c \in Caps : st.authority[ns][c] # "Absent"}

Begin(path, mode) ==
    /\ st.phase = "Idle"
    /\ path \in {"Queue", "Result"} /\ mode \in {"Move", "Copy"}
    /\ (path = "Result" => mode = "Move")
    /\ ExactLive(InitialSource) /\ ExactLive(InitialTarget)
    /\ st' = [st EXCEPT !.phase = "Preparing", !.path = path, !.mode = mode]

\* Storage preparation changes no handle or admission counter.
PrepareRecord(item) ==
    /\ st.phase = "Preparing" /\ item \notin st.storage
    /\ st' = [st EXCEPT !.storage = @ \union {item}]

RejectStorage(item) ==
    /\ st.phase = "Preparing" /\ item \notin st.storage
    /\ st' = [st EXCEPT !.phase = "Rejected"]

Reserve(item) ==
    /\ st.phase = "Preparing" /\ item \in st.storage
    /\ st.reservation[item] = 0 /\ CanReserve(st.target)
    /\ LET ns == st.target c == st.serial[ns] IN
        st' = [st EXCEPT !.authority[ns][c] = "Staged",
            !.charged[ns] = @ \union {c}, !.issued[ns] = @ \union {c},
            !.serial[ns] = @ + 1, !.reservation[item] = c]

RejectReservation(item) ==
    /\ st.phase = "Preparing" /\ item \in st.storage
    /\ st.reservation[item] = 0 /\ ~CanReserve(st.target)
    /\ st' = [st EXCEPT !.phase = "Rejected"]

PrepareMemory ==
    /\ st.phase = "Preparing" /\ st.reservation["Memory"] # 0
    /\ "Memory" \notin st.prepared /\ ExactLive(st.source)
    /\ st.authority[st.source][1] = "Live"
    /\ st.payload[st.source][1] = "SourceMemory"
    /\ st' = [st EXCEPT !.prepared = @ \union {"Memory"}, !.pin = TRUE,
        !.privateCopy = (st.mode = "Copy"),
        !.authority[st.source][1] = IF st.mode = "Move" THEN "Escrow" ELSE @]

PrepareConnection ==
    /\ st.phase = "Preparing" /\ st.reservation["Connection"] # 0
    /\ "Connection" \notin st.prepared /\ ExactLive(st.source)
    /\ st.authority[st.source][2] = "Live"
    /\ st.payload[st.source][2] = "ConnectionSource"
    /\ st' = [st EXCEPT !.prepared = @ \union {"Connection"}]

CanCommit ==
    /\ st.prepared = Items /\ st.pin
    /\ ExactLive(st.source) /\ ExactLive(st.target)
    /\ \A item \in Items : st.reservation[item] \in Caps /\
            st.authority[st.target][st.reservation[item]] = "Staged"
    /\ st.authority[st.source][1] = IF st.mode = "Move" THEN "Escrow" ELSE "Live"
    /\ st.payload[st.source][1] = "SourceMemory"
    /\ st.authority[st.source][2] = "Live"
    /\ st.payload[st.source][2] = "ConnectionSource"

\* The batch validates all captured identities before any authority changes.
\* The payload registries serialize the remaining allocation-free updates.
Commit ==
    /\ st.phase = "Preparing" /\ CanCommit
    /\ LET src == st.source dst == st.target
           mem == st.reservation["Memory"] conn == st.reservation["Connection"] IN
        st' = [st EXCEPT
            !.phase = IF st.path = "Queue" THEN "HiddenQueue" ELSE "HiddenResult",
            !.committed = TRUE, !.privateCopy = FALSE,
            !.authority[dst][mem] = "Live", !.authority[dst][conn] = "Live",
            !.payload[dst][mem] = "Memory", !.payload[dst][conn] = "Connection",
            !.delivery[dst][mem] = "Pending", !.delivery[dst][conn] = "Pending",
            !.authority[src][1] = IF st.mode = "Move" THEN "Absent" ELSE @,
            !.payload[src][1] = IF st.mode = "Move" THEN "None" ELSE @,
            !.delivery[src][1] = IF st.mode = "Move" THEN "None" ELSE @,
            !.retired[src] = IF st.mode = "Move" THEN @ \union {1} ELSE @]

RejectCommit ==
    /\ st.phase = "Preparing" /\ st.prepared = Items /\ ~CanCommit
    /\ st' = [st EXCEPT !.phase = "Rejected"]

FinishPreparation ==
    /\ st.phase \in {"HiddenQueue", "HiddenResult"} /\ st.pin
    /\ st' = [st EXCEPT !.pin = FALSE, !.storage = {}]

\* Ordinary unstarted cancellation restores only an existing captured source,
\* including a retiring namespace. Removed namespaces are never recreated.
AbortPreparation ==
    /\ st.phase \in {"Preparing", "Rejected"}
    /\ LET src == st.source dst == st.target
           staged == {c \in Reserved : st.authority[dst][c] = "Staged"} IN
        st' = [st EXCEPT !.phase = "RolledBack", !.pin = FALSE,
            !.privateCopy = FALSE, !.storage = {},
            !.authority[dst] = [c \in Caps |-> IF c \in staged THEN "Absent" ELSE st.authority[dst][c]],
            !.retired[dst] = @ \union staged,
            !.authority[src][1] = IF st.namespace[src] # "Removed" /\
                st.authority[src][1] = "Escrow" THEN "Live" ELSE @]

Readiness ==
    /\ st.phase \in {"HiddenQueue", "HiddenResult"} /\ ~st.readyHint
    /\ st' = [st EXCEPT !.readyHint = TRUE]

PrepareReceive ==
    /\ st.phase = "HiddenQueue" /\ ~st.pin /\ CanReserve(st.target)
    /\ LET ns == st.target c == st.serial[ns] IN
        st' = [st EXCEPT !.phase = "Receiving", !.reply = c,
            !.authority[ns][c] = "Staged", !.charged[ns] = @ \union {c},
            !.issued[ns] = @ \union {c}, !.serial[ns] = @ + 1]

RejectReceive ==
    /\ st.phase = "HiddenQueue" /\ ~st.pin /\ ~CanReserve(st.target)
    /\ ~st.receiveRejected
    /\ st' = [st EXCEPT !.receiveRejected = TRUE]

ReceiveWriteFailed ==
    /\ st.phase = "Receiving"
    /\ st' = [st EXCEPT !.phase = "HiddenQueue", !.receiveFailed = TRUE,
        !.authority[st.target][st.reply] = "Absent",
        !.retired[st.target] = @ \union {st.reply}, !.reply = 0]

CommitReceive ==
    /\ st.phase = "Receiving" /\ ExactLive(st.target)
    /\ LET dst == st.target IN
        st' = [st EXCEPT !.phase = "Delivered", !.delivered = TRUE,
            !.delivery[dst][st.reservation["Memory"]] = "Visible",
            !.delivery[dst][st.reservation["Connection"]] = "Visible",
            !.authority[dst][st.reply] = "Live", !.payload[dst][st.reply] = "Reply",
            !.delivery[dst][st.reply] = "Visible"]

ObserveReply ==
    /\ st.phase = "HiddenResult" /\ ~st.pin /\ ExactLive(st.target)
    /\ st' = [st EXCEPT !.phase = "Observed", !.delivered = TRUE,
        !.delivery[st.target][st.reservation["Memory"]] = "Visible",
        !.delivery[st.target][st.reservation["Connection"]] = "Visible"]

RepeatObservation ==
    /\ st.phase = "Observed" /\ st.closed # {} /\ ~st.repeated
    /\ st' = [st EXCEPT !.repeated = TRUE]

Use(item) ==
    /\ st.reservation[item] \in Caps /\ ExactLive(st.target)
    /\ st.authority[st.target][st.reservation[item]] = "Live"
    /\ st.delivery[st.target][st.reservation[item]] = "Visible"
    /\ item \notin st.used
    /\ st' = [st EXCEPT !.used = @ \union {item}]

ApplicationClose(item) ==
    /\ st.phase \in {"Delivered", "Observed"} /\ ExactLive(st.target)
    /\ item \notin st.closed
    /\ LET ns == st.target c == st.reservation[item] IN
        /\ st.authority[ns][c] = "Live" /\ st.delivery[ns][c] = "Visible"
        /\ st' = [st EXCEPT !.authority[ns][c] = "Absent",
            !.payload[ns][c] = "None", !.delivery[ns][c] = "None",
            !.retired[ns] = @ \union {c}, !.closed = @ \union {item}]

CancelHidden ==
    /\ st.phase \in {"HiddenQueue", "HiddenResult"} /\ ~st.pin
    /\ LET ns == st.target
           live == {c \in Reserved : st.authority[ns][c] = "Live"} IN
        st' = [st EXCEPT !.phase = "Cancelled",
            !.authority[ns] = [c \in Caps |-> IF c \in live THEN "Absent" ELSE st.authority[ns][c]],
            !.payload[ns] = [c \in Caps |-> IF c \in live THEN "None" ELSE st.payload[ns][c]],
            !.delivery[ns] = [c \in Caps |-> IF c \in live THEN "None" ELSE st.delivery[ns][c]],
            !.retired[ns] = @ \union live]

\* Pressure uses ordinary admitted records. It can fill the escrowed source's
\* account; rollback must restore that source without another reservation.
Fill(role) ==
    /\ st.phase \in {"Idle", "Preparing"}
    /\ LET ns == Current(role) c == st.serial[ns] IN
        /\ ns[2] = 1 /\ CanReserve(ns)
        /\ st' = [st EXCEPT !.authority[ns][c] = "Live", !.payload[ns][c] = "Filler",
            !.delivery[ns][c] = "Visible", !.charged[ns] = @ \union {c},
            !.issued[ns] = @ \union {c}, !.serial[ns] = @ + 1]

\* Receiving is the interval in which PreparedReceive exclusively borrows IPC.
RetireNamespace(role) ==
    /\ st.phase # "Receiving"
    /\ LET ns == Current(role) IN
        /\ st.namespace[ns] = "Live"
        /\ st' = [st EXCEPT !.namespace[ns] = "Retiring"]

DetachNamespace(role) ==
    /\ st.phase # "Receiving"
    /\ LET ns == Current(role) IN
        /\ st.namespace[ns] = "Retiring"
        /\ st' = [st EXCEPT !.namespace[ns] = "Removed",
            !.retired[ns] = @ \union Present(ns), !.authority[ns] = EmptyAuthority,
            !.payload[ns] = EmptyPayload, !.delivery[ns] = EmptyDelivery]

\* A successor deliberately reuses scalar handles 1 and 2. They are unrelated
\* sentinel payloads, protected from every old token's cleanup.
ReplaceNamespace(role) ==
    /\ st.phase # "Receiving" /\ st.current[role] = 1
    /\ st.namespace[Current(role)] = "Removed" /\ NodeUsed + 2 <= NodeLimit
    /\ LET ns == <<role, 2>> IN
        st' = [st EXCEPT !.current[role] = 2, !.namespace[ns] = "Live",
            !.authority[ns] = [EmptyAuthority EXCEPT ![1] = "Live", ![2] = "Live"],
            !.payload[ns] = [EmptyPayload EXCEPT ![1] = "Filler", ![2] = "Filler"],
            !.delivery[ns] = [EmptyDelivery EXCEPT ![1] = "Visible", ![2] = "Visible"],
            !.charged[ns] = {1, 2}, !.issued[ns] = {1, 2}, !.serial[ns] = 3]

\* Explicit post-guard disposal refunds the original account only after its
\* node is released. A move's retired source stays with its pin until finish.
ReleaseRetired(ns, c) ==
    /\ c \in st.retired[ns]
    /\ ~(ns = st.source /\ c = 1 /\ st.pin /\ st.committed)
    /\ st' = [st EXCEPT !.retired[ns] = @ \ {c}, !.charged[ns] = @ \ {c}]

Abandon ==
    /\ st.phase = "Preparing"
    /\ st' = [st EXCEPT !.phase = "Abandoned"]

Next ==
    \/ \E path \in {"Queue", "Result"}, mode \in {"Move", "Copy"} : Begin(path, mode)
    \/ \E item \in Items : PrepareRecord(item) \/ RejectStorage(item)
        \/ Reserve(item) \/ RejectReservation(item) \/ Use(item) \/ ApplicationClose(item)
    \/ PrepareMemory \/ PrepareConnection \/ Commit \/ RejectCommit
    \/ FinishPreparation \/ AbortPreparation \/ Readiness \/ PrepareReceive
    \/ RejectReceive \/ ReceiveWriteFailed \/ CommitReceive \/ ObserveReply
    \/ RepeatObservation \/ CancelHidden \/ Abandon
    \/ \E role \in Roles : Fill(role) \/ RetireNamespace(role)
        \/ DetachNamespace(role) \/ ReplaceNamespace(role)
    \/ \E ns \in Namespaces, c \in Caps : ReleaseRetired(ns, c)

Spec == Init /\ [][Next]_vars

UnsafeUseMemory ==
    /\ st.phase \in {"HiddenQueue", "HiddenResult"} /\ ~st.delivered
    /\ st' = [st EXCEPT !.used = @ \union {"Memory"}]

UnsafeUseConnection ==
    /\ st.phase \in {"HiddenQueue", "HiddenResult"} /\ ~st.delivered
    /\ st' = [st EXCEPT !.used = @ \union {"Connection"}]

UnsafePublishPrefix ==
    /\ st.phase = "Preparing" /\ CanCommit
    /\ st' = [st EXCEPT !.authority[st.target][st.reservation["Memory"]] = "Live"]

UnsafeReuseSerial ==
    /\ st.phase = "RolledBack" /\ Reserved # {} /\ ExactLive(st.target)
    /\ \E c \in Reserved : st' = [st EXCEPT !.serial[st.target] = c]

UnsafeCancelSuccessor ==
    /\ st.phase \in {"Preparing", "Rejected"} /\ Reserved # {}
    /\ st.current["Receiver"] = 2 /\ st.namespace[st.target] = "Removed"
    /\ \E c \in Reserved \intersect {1, 2} :
        st' = [st EXCEPT !.authority[Current("Receiver")][c] = "Absent"]

UnsafeRestoreSuccessor ==
    /\ st.phase \in {"Preparing", "Rejected"} /\ st.mode = "Move" /\ st.pin
    /\ st.current["Sender"] = 2 /\ st.namespace[st.source] = "Removed"
    /\ st' = [st EXCEPT !.payload[Current("Sender")][1] = "SourceMemory"]

UnsafeRefundRetained(ns, c) ==
    /\ c \in st.retired[ns]
    /\ st' = [st EXCEPT !.charged[ns] = @ \ {c}]

UnsafeReadinessDelivers ==
    /\ st.phase \in {"HiddenQueue", "HiddenResult"} /\ st.readyHint
    /\ st' = [st EXCEPT !.delivery[st.target][st.reservation["Memory"]] = "Visible"]

UnsafeRepublishObserved ==
    /\ st.phase = "Observed" /\ "Memory" \in st.closed /\ ExactLive(st.target)
    /\ st' = [st EXCEPT !.authority[st.target][st.reservation["Memory"]] = "Live"]

\* A restoration that tries to reserve again can strand escrow at the limit.
UnsafeRollbackNeedsAdmission ==
    /\ st.phase = "Preparing" /\ st.mode = "Move" /\ st.pin
    /\ st.authority[st.source][1] = "Escrow" /\ ExactLive(st.source)
    /\ Cardinality(st.charged[st.source]) = NamespaceLimit
    /\ st' = [st EXCEPT !.phase = "RolledBack", !.pin = FALSE,
        !.privateCopy = FALSE, !.storage = {}]

UnsafeMemorySpec == Init /\ [][Next \/ UnsafeUseMemory]_vars
UnsafeConnectionSpec == Init /\ [][Next \/ UnsafeUseConnection]_vars
UnsafeBatchSpec == Init /\ [][Next \/ UnsafePublishPrefix]_vars
UnsafeSerialSpec == Init /\ [][Next \/ UnsafeReuseSerial]_vars
UnsafeCancelSpec == Init /\ [][Next \/ UnsafeCancelSuccessor]_vars
UnsafeRestoreSpec == Init /\ [][Next \/ UnsafeRestoreSuccessor]_vars
UnsafeRefundSpec == Init /\ [][Next \/ \E ns \in Namespaces, c \in Caps : UnsafeRefundRetained(ns, c)]_vars
UnsafeReadinessSpec == Init /\ [][Next \/ UnsafeReadinessDelivers]_vars
UnsafeObserveSpec == Init /\ [][Next \/ UnsafeRepublishObserved]_vars
UnsafeRollbackSpec == Init /\ [][Next \/ UnsafeRollbackNeedsAdmission]_vars

TypeOK ==
    /\ st.phase \in Phases /\ st.path \in {"Queue", "Result"} /\ st.mode \in {"Move", "Copy"}
    /\ st.current \in [Roles -> Generations]
    /\ st.namespace \in [Namespaces -> {"Unused", "Live", "Retiring", "Removed"}]
    /\ st.serial \in [Namespaces -> 1..(MaxSerial + 1)]
    /\ st.issued \in [Namespaces -> SUBSET Caps]
    /\ st.authority \in [Namespaces -> [Caps -> Authority]]
    /\ st.payload \in [Namespaces -> [Caps -> Payloads]]
    /\ st.delivery \in [Namespaces -> [Caps -> Deliveries]]
    /\ st.charged \in [Namespaces -> SUBSET Caps] /\ st.retired \in [Namespaces -> SUBSET Caps]
    /\ st.source \in Namespaces /\ st.target \in Namespaces
    /\ st.reservation \in [Items -> 0..MaxSerial]
    /\ st.storage \subseteq Items /\ st.prepared \subseteq Items
    /\ st.pin \in BOOLEAN /\ st.privateCopy \in BOOLEAN
    /\ st.committed \in BOOLEAN /\ st.delivered \in BOOLEAN /\ st.readyHint \in BOOLEAN
    /\ st.reply \in 0..MaxSerial /\ st.receiveRejected \in BOOLEAN /\ st.receiveFailed \in BOOLEAN
    /\ st.repeated \in BOOLEAN /\ st.used \subseteq Items /\ st.closed \subseteq Items

UseRequiresDelivery == st.used # {} => st.delivered
VisibleRequiresDelivery ==
    \A c \in Reserved : st.delivery[st.target][c] = "Visible" => st.delivered
AtomicPublication ==
    ~st.committed => \A c \in Reserved : st.authority[st.target][c] # "Live"
SerialsConsumed == \A ns \in Namespaces : \A c \in st.issued[ns] : c < st.serial[ns]
ReplacementIntact ==
    \A role \in Roles :
        st.current[role] = 2 /\ st.namespace[Current(role)] # "Removed" =>
            \A c \in {1, 2} : st.authority[Current(role)][c] = "Live"
                /\ st.payload[Current(role)][c] = "Filler"
RetainedNodesCharged ==
    \A ns \in Namespaces : st.charged[ns] = Present(ns) \union st.retired[ns]
AdmissionBounded == NodeUsed <= NodeLimit /\
    \A ns \in Namespaces : Cardinality(st.charged[ns]) <= NamespaceLimit
ObservedCloseStaysClosed ==
    \A item \in st.closed : st.authority[st.target][st.reservation[item]] = "Absent"
MoveNeverRestoredAfterCommit == st.committed /\ st.mode = "Move" =>
    st.authority[st.source][1] = "Absent" /\ st.payload[st.source][1] = "None"
RemovedNamespaceEmpty ==
    \A ns \in Namespaces : st.namespace[ns] = "Removed" => Present(ns) = {}
PreparationRetainsPin ==
    st.phase \in {"Preparing", "Rejected", "Abandoned"} /\ "Memory" \in st.prepared => st.pin
RollbackRestoresCapturedSource ==
    st.phase = "RolledBack" /\ "Memory" \in st.prepared /\
        st.namespace[st.source] # "Removed" =>
            st.authority[st.source][1] = "Live" /\ st.payload[st.source][1] = "SourceMemory"
RollbackDisposesPreparation == st.phase = "RolledBack" =>
    ~st.pin /\ ~st.privateCopy /\ st.storage = {} /\
        \A c \in Reserved : st.authority[st.target][c] = "Absent"
HiddenContainerOwnsAttachments ==
    st.phase \in {"HiddenQueue", "HiddenResult", "Receiving"} /\
        st.namespace[st.target] # "Removed" =>
            \A c \in Reserved : st.authority[st.target][c] = "Live" /\
                st.delivery[st.target][c] = "Pending"
RetiredRecordsDetached ==
    \A ns \in Namespaces : Present(ns) \intersect st.retired[ns] = {}

Invariants == TypeOK /\ UseRequiresDelivery /\ VisibleRequiresDelivery
    /\ AtomicPublication /\ SerialsConsumed /\ ReplacementIntact
    /\ RetainedNodesCharged /\ AdmissionBounded /\ ObservedCloseStaysClosed
    /\ MoveNeverRestoredAfterCommit /\ RemovedNamespaceEmpty /\ PreparationRetainsPin
    /\ RollbackRestoresCapturedSource /\ RollbackDisposesPreparation
    /\ HiddenContainerOwnsAttachments /\ RetiredRecordsDetached

=============================================================================
