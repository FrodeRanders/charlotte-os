------------------------------ MODULE CharlotteDMA ------------------------------
\* DMA memory pinning and Arm SMMUv3 domain teardown.  The model deliberately
\* separates translation revocation from pin release: frames must remain pinned
\* whenever hardware might still translate to them.

EXTENDS Naturals, FiniteSets

CONSTANTS Driver, Stream, Memory, Domain, NullDriver, NullStream

ASSUME NullDriver \notin Driver
ASSUME NullStream \notin Stream

DomainStates == {"Absent", "Active", "Destroying", "Revoked", "Quarantined"}
MemoryStates == {"Absent", "Live", "DestroyPending", "Freed"}
MapStates == {"None", "Pinned", "Mapped", "Revoked"}
MapModes == {"None", "Coherent", "Exclusive"}
LoanStates == {"None", "Read", "Write"}

VARIABLES driverOpen, domainState, domainOwner, domainStream,
          streamDomain, memoryState, memoryOwner, mappings, mappingMode,
          cpuMapped, loanState

vars == <<driverOpen, domainState, domainOwner, domainStream,
          streamDomain, memoryState, memoryOwner, mappings, mappingMode,
          cpuMapped, loanState>>

\* A stream uses the distinguished string "NoDomain" when it is unbound.
NoDomain == "NoDomain"

Init ==
    /\ driverOpen = [d \in Driver |-> TRUE]
    /\ domainState = [dom \in Domain |-> "Absent"]
    /\ domainOwner = [dom \in Domain |-> NullDriver]
    /\ domainStream = [dom \in Domain |-> NullStream]
    /\ streamDomain = [sid \in Stream |-> NoDomain]
    /\ memoryState = [mem \in Memory |-> "Absent"]
    /\ memoryOwner = [mem \in Memory |-> NullDriver]
    /\ mappings = [dom \in Domain |-> [mem \in Memory |-> "None"]]
    /\ mappingMode = [dom \in Domain |-> [mem \in Memory |-> "None"]]
    /\ cpuMapped = [mem \in Memory |-> FALSE]
    /\ loanState = [mem \in Memory |-> "None"]

CreateMemory(driver, mem) ==
    /\ driverOpen[driver]
    /\ memoryState[mem] = "Absent"
    /\ memoryState' = [memoryState EXCEPT ![mem] = "Live"]
    /\ memoryOwner' = [memoryOwner EXCEPT ![mem] = driver]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, mappings, mappingMode, cpuMapped, loanState>>

CreateDomain(driver, sid, dom) ==
    /\ driverOpen[driver]
    /\ domainState[dom] = "Absent"
    /\ streamDomain[sid] = NoDomain
    /\ domainState' = [domainState EXCEPT ![dom] = "Active"]
    /\ domainOwner' = [domainOwner EXCEPT ![dom] = driver]
    /\ domainStream' = [domainStream EXCEPT ![dom] = sid]
    /\ streamDomain' = [streamDomain EXCEPT ![sid] = dom]
    /\ UNCHANGED <<driverOpen, memoryState, memoryOwner, mappings, mappingMode,
                   cpuMapped, loanState>>

CpuMap(driver, mem) ==
    /\ driverOpen[driver]
    /\ memoryState[mem] = "Live"
    /\ memoryOwner[mem] = driver
    /\ ~cpuMapped[mem]
    /\ \A dom \in Domain : mappingMode[dom][mem] # "Exclusive"
    /\ cpuMapped' = [cpuMapped EXCEPT ![mem] = TRUE]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, memoryState, memoryOwner, mappings,
                   mappingMode, loanState>>

CpuUnmap(driver, mem) ==
    /\ memoryOwner[mem] = driver
    /\ cpuMapped[mem]
    /\ cpuMapped' = [cpuMapped EXCEPT ![mem] = FALSE]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, memoryState, memoryOwner, mappings,
                   mappingMode, loanState>>

BeginLoan(driver, mem, kind) ==
    /\ driverOpen[driver]
    /\ memoryState[mem] = "Live"
    /\ memoryOwner[mem] = driver
    /\ loanState[mem] = "None"
    /\ kind \in {"Read", "Write"}
    /\ \A dom \in Domain : mappings[dom][mem] = "None"
    /\ loanState' = [loanState EXCEPT ![mem] = kind]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, memoryState, memoryOwner, mappings,
                   mappingMode, cpuMapped>>

EndLoan(driver, mem) ==
    /\ memoryOwner[mem] = driver
    /\ loanState[mem] # "None"
    /\ loanState' = [loanState EXCEPT ![mem] = "None"]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, memoryState, memoryOwner, mappings,
                   mappingMode, cpuMapped>>

\* pin_for_dma succeeds before the SMMU lock is acquired.
BeginMap(driver, dom, mem, mode) ==
    /\ driverOpen[driver]
    /\ domainState[dom] = "Active"
    /\ domainOwner[dom] = driver
    /\ memoryState[mem] = "Live"
    /\ memoryOwner[mem] = driver
    /\ mappings[dom][mem] = "None"
    /\ mode \in {"Coherent", "Exclusive"}
    /\ \A other \in Domain : mappingMode[other][mem] # "Exclusive"
    /\ (mode = "Exclusive" =>
            /\ ~cpuMapped[mem]
            /\ loanState[mem] = "None"
            /\ \A other \in Domain : mappings[other][mem] = "None")
    /\ mappings' = [mappings EXCEPT ![dom][mem] = "Pinned"]
    /\ mappingMode' = [mappingMode EXCEPT ![dom][mem] = mode]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, memoryState, memoryOwner, cpuMapped, loanState>>

\* Page-table installation plus a successful SMMU invalidation publishes the
\* IOVA to userspace.
CommitMap(dom, mem) ==
    /\ domainState[dom] = "Active"
    /\ mappings[dom][mem] = "Pinned"
    /\ mappings' = [mappings EXCEPT ![dom][mem] = "Mapped"]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, memoryState, memoryOwner, mappingMode,
                   cpuMapped, loanState>>

\* If PTE installation completed but invalidation timed out, hardware state is
\* uncertain. The unpublished mapping remains pinned until domain destruction.
QuarantineMap(dom, mem) ==
    /\ domainState[dom] = "Active"
    /\ mappings[dom][mem] = "Pinned"
    /\ mappings' = [mappings EXCEPT ![dom][mem] = "Mapped"]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, memoryState, memoryOwner, mappingMode,
                   cpuMapped, loanState>>

\* Errors before complete PTE installation remove partial PTEs and release the
\* pin.
FailMap(dom, mem) ==
    /\ mappings[dom][mem] = "Pinned"
    /\ mappings' = [mappings EXCEPT ![dom][mem] = "None"]
    /\ mappingMode' = [mappingMode EXCEPT ![dom][mem] = "None"]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, memoryState, memoryOwner, cpuMapped, loanState>>

\* Successful invalidation removes hardware access but intentionally retains
\* the pin until the mapping record has been consumed.
RevokeMap(dom, mem) ==
    /\ mappings[dom][mem] = "Mapped"
    /\ mappings' = [mappings EXCEPT ![dom][mem] = "Revoked"]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, memoryState, memoryOwner, mappingMode,
                   cpuMapped, loanState>>

ReleasePin(dom, mem) ==
    /\ mappings[dom][mem] = "Revoked"
    /\ mappings' = [mappings EXCEPT ![dom][mem] = "None"]
    /\ mappingMode' = [mappingMode EXCEPT ![dom][mem] = "None"]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, memoryState, memoryOwner, cpuMapped, loanState>>

BeginDestroy(dom) ==
    /\ domainState[dom] = "Active"
    /\ domainState' = [domainState EXCEPT ![dom] = "Destroying"]
    /\ UNCHANGED <<driverOpen, domainOwner, domainStream, streamDomain,
                   memoryState, memoryOwner, mappings, mappingMode,
                   cpuMapped, loanState>>

\* Only an acknowledged aborting stream-table entry permits translation
\* records to become Revoked and their pins to be released.
AcknowledgeDestroy(dom) ==
    /\ domainState[dom] = "Destroying"
    /\ domainState' = [domainState EXCEPT ![dom] = "Revoked"]
    /\ streamDomain' = [streamDomain EXCEPT ![domainStream[dom]] = NoDomain]
    /\ mappings' =
        [mappings EXCEPT ![dom] =
            [mem \in Memory |->
                IF @[mem] = "Mapped" THEN "Revoked" ELSE @[mem]]]
    /\ UNCHANGED <<driverOpen, domainOwner, domainStream,
                   memoryState, memoryOwner, mappingMode, cpuMapped, loanState>>

\* A timeout leaves the domain and all pins quarantined.  It is safe to leak
\* them; it is not safe to pretend that hardware stopped translating.
QuarantineDestroy(dom) ==
    /\ domainState[dom] = "Destroying"
    /\ domainState' = [domainState EXCEPT ![dom] = "Quarantined"]
    /\ UNCHANGED <<driverOpen, domainOwner, domainStream, streamDomain,
                   memoryState, memoryOwner, mappings, mappingMode,
                   cpuMapped, loanState>>

FinalizeDomain(dom) ==
    /\ domainState[dom] = "Revoked"
    /\ \A mem \in Memory : mappings[dom][mem] = "None"
    /\ domainState' = [domainState EXCEPT ![dom] = "Absent"]
    /\ domainOwner' = [domainOwner EXCEPT ![dom] = NullDriver]
    /\ domainStream' = [domainStream EXCEPT ![dom] = NullStream]
    /\ UNCHANGED <<driverOpen, streamDomain, memoryState, memoryOwner, mappings,
                   mappingMode, cpuMapped, loanState>>

CloseMemory(driver, mem) ==
    /\ memoryOwner[mem] = driver
    /\ memoryState[mem] = "Live"
    /\ \A dom \in Domain : mappings[dom][mem] = "None"
    /\ ~cpuMapped[mem]
    /\ loanState[mem] = "None"
    /\ memoryState' = [memoryState EXCEPT ![mem] = "Freed"]
    /\ memoryOwner' = [memoryOwner EXCEPT ![mem] = NullDriver]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, mappings, mappingMode, cpuMapped, loanState>>

\* Address-space teardown makes pinned objects destroy-pending and begins
\* revocation of every domain.  It never frees pinned memory directly.
ExitDriver(driver) ==
    /\ driverOpen[driver]
    /\ driverOpen' = [driverOpen EXCEPT ![driver] = FALSE]
    /\ domainState' =
        [dom \in Domain |->
            IF domainOwner[dom] = driver /\ domainState[dom] = "Active"
            THEN "Destroying" ELSE domainState[dom]]
    /\ memoryState' =
        [mem \in Memory |->
            IF memoryOwner[mem] = driver /\ memoryState[mem] = "Live"
            THEN "DestroyPending" ELSE memoryState[mem]]
    /\ cpuMapped' =
        [mem \in Memory |->
            IF memoryOwner[mem] = driver THEN FALSE ELSE cpuMapped[mem]]
    /\ loanState' =
        [mem \in Memory |->
            IF memoryOwner[mem] = driver THEN "None" ELSE loanState[mem]]
    /\ UNCHANGED <<domainOwner, domainStream, streamDomain, memoryOwner, mappings,
                   mappingMode>>

ReclaimMemory(mem) ==
    /\ memoryState[mem] = "DestroyPending"
    /\ \A dom \in Domain : mappings[dom][mem] = "None"
    /\ ~cpuMapped[mem]
    /\ loanState[mem] = "None"
    /\ memoryState' = [memoryState EXCEPT ![mem] = "Freed"]
    /\ memoryOwner' = [memoryOwner EXCEPT ![mem] = NullDriver]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, mappings, mappingMode, cpuMapped, loanState>>

Next ==
    \/ \E driver \in Driver, mem \in Memory : CreateMemory(driver, mem)
    \/ \E driver \in Driver, sid \in Stream, dom \in Domain :
        CreateDomain(driver, sid, dom)
    \/ \E driver \in Driver, mem \in Memory : CpuMap(driver, mem)
    \/ \E driver \in Driver, mem \in Memory : CpuUnmap(driver, mem)
    \/ \E driver \in Driver, mem \in Memory, kind \in {"Read", "Write"} :
        BeginLoan(driver, mem, kind)
    \/ \E driver \in Driver, mem \in Memory : EndLoan(driver, mem)
    \/ \E driver \in Driver, dom \in Domain, mem \in Memory,
          mode \in {"Coherent", "Exclusive"} : BeginMap(driver, dom, mem, mode)
    \/ \E dom \in Domain, mem \in Memory : CommitMap(dom, mem)
    \/ \E dom \in Domain, mem \in Memory : QuarantineMap(dom, mem)
    \/ \E dom \in Domain, mem \in Memory : FailMap(dom, mem)
    \/ \E dom \in Domain, mem \in Memory : RevokeMap(dom, mem)
    \/ \E dom \in Domain, mem \in Memory : ReleasePin(dom, mem)
    \/ \E dom \in Domain : BeginDestroy(dom)
    \/ \E dom \in Domain : AcknowledgeDestroy(dom)
    \/ \E dom \in Domain : QuarantineDestroy(dom)
    \/ \E dom \in Domain : FinalizeDomain(dom)
    \/ \E driver \in Driver, mem \in Memory : CloseMemory(driver, mem)
    \/ \E driver \in Driver : ExitDriver(driver)
    \/ \E mem \in Memory : ReclaimMemory(mem)

Spec == Init /\ [][Next]_vars

\* Regression action: the old DMA abstraction allowed an exclusive pin to be
\* created without first excluding CPU mappings, loans, and other DMA pins.
UnsafeBeginExclusiveMap(driver, dom, mem) ==
    /\ driverOpen[driver]
    /\ domainState[dom] = "Active"
    /\ domainOwner[dom] = driver
    /\ memoryState[mem] = "Live"
    /\ memoryOwner[mem] = driver
    /\ mappings[dom][mem] = "None"
    /\ mappings' = [mappings EXCEPT ![dom][mem] = "Pinned"]
    /\ mappingMode' = [mappingMode EXCEPT ![dom][mem] = "Exclusive"]
    /\ UNCHANGED <<driverOpen, domainState, domainOwner, domainStream,
                   streamDomain, memoryState, memoryOwner, cpuMapped, loanState>>

UnsafeNext == Next \/
    \E driver \in Driver, dom \in Domain, mem \in Memory :
        UnsafeBeginExclusiveMap(driver, dom, mem)

UnsafeSpec == Init /\ [][UnsafeNext]_vars

TypeOK ==
    /\ driverOpen \in [Driver -> BOOLEAN]
    /\ domainState \in [Domain -> DomainStates]
    /\ domainOwner \in [Domain -> Driver \cup {NullDriver}]
    /\ domainStream \in [Domain -> Stream \cup {NullStream}]
    /\ streamDomain \in [Stream -> Domain \cup {NoDomain}]
    /\ memoryState \in [Memory -> MemoryStates]
    /\ memoryOwner \in [Memory -> Driver \cup {NullDriver}]
    /\ mappings \in [Domain -> [Memory -> MapStates]]
    /\ mappingMode \in [Domain -> [Memory -> MapModes]]
    /\ cpuMapped \in [Memory -> BOOLEAN]
    /\ loanState \in [Memory -> LoanStates]

StreamUnique ==
    \A sid \in Stream, dom \in Domain :
        streamDomain[sid] = dom =>
            /\ domainStream[dom] = sid
            /\ domainState[dom] \in {"Active", "Destroying", "Quarantined"}

LiveDomainHasStream ==
    \A dom \in Domain :
        domainState[dom] \in {"Active", "Destroying", "Quarantined"} =>
            /\ domainOwner[dom] \in Driver
            /\ domainStream[dom] \in Stream
            /\ streamDomain[domainStream[dom]] = dom

MappedImpliesPossibleHardwareAccess ==
    \A dom \in Domain, mem \in Memory :
        mappings[dom][mem] = "Mapped" =>
            /\ domainState[dom] \in {"Active", "Destroying", "Quarantined"}
            /\ memoryState[mem] \in {"Live", "DestroyPending"}
            /\ memoryOwner[mem] = domainOwner[dom]

PinnedMemoryNotFreed ==
    \A dom \in Domain, mem \in Memory :
        mappings[dom][mem] # "None" =>
            memoryState[mem] \in {"Live", "DestroyPending"}

FreedMemoryUnreachable ==
    \A mem \in Memory :
        memoryState[mem] = "Freed" =>
            \A dom \in Domain : mappings[dom][mem] = "None"

NoCrossDomainMapping ==
    \A dom \in Domain, mem \in Memory :
        mappings[dom][mem] # "None" =>
            memoryOwner[mem] = domainOwner[dom]

ClosedDriverCannotCreateAuthority ==
    /\ \A dom \in Domain :
        domainState[dom] = "Active" => driverOpen[domainOwner[dom]]
    /\ \A mem \in Memory :
        memoryState[mem] = "Live" => driverOpen[memoryOwner[mem]]

MappingModeTracksPin ==
    \A dom \in Domain, mem \in Memory :
        (mappings[dom][mem] = "None") = (mappingMode[dom][mem] = "None")

ExclusiveDmaHasNoCpuAuthority ==
    \A dom \in Domain, mem \in Memory :
        mappingMode[dom][mem] = "Exclusive" =>
            /\ mappings[dom][mem] # "None"
            /\ ~cpuMapped[mem]
            /\ loanState[mem] = "None"
            /\ \A other \in Domain : other = dom \/ mappings[other][mem] = "None"

Invariants ==
    /\ TypeOK
    /\ StreamUnique
    /\ LiveDomainHasStream
    /\ MappedImpliesPossibleHardwareAccess
    /\ PinnedMemoryNotFreed
    /\ FreedMemoryUnreachable
    /\ NoCrossDomainMapping
    /\ ClosedDriverCannotCreateAuthority
    /\ MappingModeTracksPin
    /\ ExclusiveDmaHasNoCpuAuthority

=============================================================================
