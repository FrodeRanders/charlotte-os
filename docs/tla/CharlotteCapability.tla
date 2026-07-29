------------------------- MODULE CharlotteCapability -------------------------
\* Unified tagged, per-address-space object-capability namespace, including
\* fresh delegation and same-handle transactional rollback.

EXTENDS Naturals, FiniteSets

CONSTANTS ASID, MaxCaps, NullAsid

ASSUME NullAsid \notin ASID
ASSUME MaxCaps > 1

CapId == 1..MaxCaps
Kinds == {"Ipc", "Memory", "Completion", "Device", "Mailbox", "SystemObserver"}
CapKind == Kinds \cup {"Null"}

Txn == [
    active : BOOLEAN,
    source : ASID \cup {NullAsid},
    target : ASID \cup {NullAsid},
    cap    : 0..MaxCaps,
    kind   : CapKind
]

NoTxn == [active |-> FALSE, source |-> NullAsid, target |-> NullAsid,
          cap |-> 0, kind |-> "Null"]

VARIABLES caps, nextSerial, namespaceOpen, transaction

vars == <<caps, nextSerial, namespaceOpen, transaction>>

Init ==
    /\ caps = [as \in ASID |-> [c \in CapId |-> "Null"]]
    /\ nextSerial = [as \in ASID |-> 1]
    /\ namespaceOpen = [as \in ASID |-> TRUE]
    /\ transaction = NoTxn

CanAllocate(as) == nextSerial[as] \in CapId

Allocate(as, kind) ==
    /\ namespaceOpen[as]
    /\ CanAllocate(as)
    /\ caps' = [caps EXCEPT ![as][nextSerial[as]] = kind]
    /\ nextSerial' = [nextSerial EXCEPT ![as] = @ + 1]
    /\ UNCHANGED <<namespaceOpen, transaction>>

Remove(as, cap, expectedKind) ==
    /\ namespaceOpen[as]
    /\ caps[as][cap] = expectedKind
    /\ caps' = [caps EXCEPT ![as][cap] = "Null"]
    /\ UNCHANGED <<nextSerial, namespaceOpen, transaction>>

\* Public delegation allocates a fresh target handle; it never copies a
\* numeric handle between address spaces.
DelegateCopy(source, cap, target) ==
    /\ source /= target
    /\ namespaceOpen[source] /\ namespaceOpen[target]
    /\ caps[source][cap] \in Kinds
    /\ CanAllocate(target)
    /\ caps' = [caps EXCEPT
         ![target][nextSerial[target]] = caps[source][cap]]
    /\ nextSerial' = [nextSerial EXCEPT ![target] = @ + 1]
    /\ UNCHANGED <<namespaceOpen, transaction>>

\* A cross-registry move first removes authority from the source. The payload
\* subsystem either commits a freshly allocated target handle or restores the
\* exact source handle during rollback.
BeginMove(source, cap, target) ==
    /\ ~transaction.active
    /\ source /= target
    /\ namespaceOpen[source] /\ namespaceOpen[target]
    /\ caps[source][cap] \in Kinds
    /\ caps' = [caps EXCEPT ![source][cap] = "Null"]
    /\ transaction' = [active |-> TRUE, source |-> source, target |-> target,
                       cap |-> cap, kind |-> caps[source][cap]]
    /\ UNCHANGED <<nextSerial, namespaceOpen>>

CommitMove ==
    /\ transaction.active
    /\ namespaceOpen[transaction.target]
    /\ CanAllocate(transaction.target)
    /\ caps' = [caps EXCEPT
         ![transaction.target][nextSerial[transaction.target]] = transaction.kind]
    /\ nextSerial' = [nextSerial EXCEPT ![transaction.target] = @ + 1]
    /\ transaction' = NoTxn
    /\ UNCHANGED namespaceOpen

RollbackMove ==
    /\ transaction.active
    /\ namespaceOpen[transaction.source]
    /\ caps[transaction.source][transaction.cap] = "Null"
    /\ caps' = [caps EXCEPT
         ![transaction.source][transaction.cap] = transaction.kind]
    /\ transaction' = NoTxn
    /\ UNCHANGED <<nextSerial, namespaceOpen>>

CloseAddressSpace(as) ==
    /\ ~transaction.active
    /\ namespaceOpen[as]
    /\ \E c \in CapId : caps[as][c] /= "Null"
    /\ caps' = [caps EXCEPT ![as] = [c \in CapId |-> "Null"]]
    /\ namespaceOpen' = [namespaceOpen EXCEPT ![as] = FALSE]
    \* Serial numbers are deliberately not reset within one ASID lifetime.
    /\ UNCHANGED <<nextSerial, transaction>>

Next ==
    \/ \E as \in ASID, kind \in Kinds : Allocate(as, kind)
    \/ \E as \in ASID, cap \in CapId, kind \in Kinds : Remove(as, cap, kind)
    \/ \E source, target \in ASID, cap \in CapId : DelegateCopy(source, cap, target)
    \/ \E source, target \in ASID, cap \in CapId : BeginMove(source, cap, target)
    \/ CommitMove
    \/ RollbackMove
    \/ \E as \in ASID : CloseAddressSpace(as)

Spec == Init /\ [][Next]_vars

TypeOK ==
    /\ caps \in [ASID -> [CapId -> CapKind]]
    /\ nextSerial \in [ASID -> 1..(MaxCaps + 1)]
    /\ namespaceOpen \in [ASID -> BOOLEAN]
    /\ transaction \in Txn

NoFutureHandles ==
    \A as \in ASID, cap \in CapId :
        cap >= nextSerial[as] => caps[as][cap] = "Null"

TransactionSourceRevoked ==
    transaction.active =>
        /\ transaction.source /= transaction.target
        /\ transaction.kind \in Kinds
        /\ transaction.cap < nextSerial[transaction.source]
        /\ caps[transaction.source][transaction.cap] = "Null"

TagsAreAuthoritative ==
    \A as \in ASID, cap \in CapId :
        caps[as][cap] \in Kinds => cap < nextSerial[as]

ClosedNamespaceEmpty ==
    \A as \in ASID :
        ~namespaceOpen[as] => \A cap \in CapId : caps[as][cap] = "Null"

Invariants ==
    /\ TypeOK
    /\ NoFutureHandles
    /\ TransactionSourceRevoked
    /\ TagsAreAuthoritative
    /\ ClosedNamespaceEmpty

=============================================================================
