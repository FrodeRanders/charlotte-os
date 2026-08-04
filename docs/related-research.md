# Related Operating-Systems Research

## Purpose

This note identifies operating-systems and distributed-systems research related to the designs described in:

- [`charlotte-networking-architecture.md`](charlotte-networking-architecture.md)
- [`charlotte-sitas-xous-architecture.md`](charlotte-sitas-xous-architecture.md)
- [`manual-v2/charlotte.pdf`](manual-v2/charlotte.pdf), especially Chapters 11 and 16, and Chapter 17 (the server-class cluster vision)

These do not descend from one research tradition and shamelessly combine good ideas from:

1. capability-based microkernels;
2. distributed operating systems organized around RPC and objects;
3. message-passing many-core operating systems;
4. ownership-oriented IPC;
5. isolated user-space services and drivers;
6. high-performance user-space networking and asynchronous I/O.

The closest overall characterization is:

> CharlotteOS combines Xous-style isolated IPC and memory lending, 
  Seastar-style shard-local execution, seL4/EROS-style authority, 
  Barrelfish-style explicit message passing, and Amoeba-style 
  distributed capability invocation.

Its most important unresolved research problem is preserving capability 
and ownership semantics across unreliable networks, retries, partial 
failure, and service restart.

This note distinguishes four kinds of answer:

- **Implemented** means the behavior exists in the current kernel or userspace
  services and has direct tests.
- **Specified and bounded-model-checked** means an executable TLA+ abstraction
  and a Rust conformance map exist, but there is no refinement proof.
- **Partial** means a local mechanism or lower layer exists, while the
  end-to-end policy or some failure modes remain unresolved.
- **Open** means the architecture may state an intention, but the repository
  does not yet provide an implementation-level contract.

The audit below finds that most questions about the **local** object model now
have implemented or bounded-model-checked answers: tagged capability lookup,
rights attenuation, reply-token linearity, memory transfer and lending,
completion retention and cancellation, lifecycle teardown, DMA isolation, and
Raft's internal safety mechanisms. The principal open areas are the **remote**
invocation/capability contract, derivation-based and distributed revocation,
whole-system information-flow proof, consensus-backed service authority, and
end-to-end admission/priority policy.

---

## 1. Architectural themes

The documents establish the following core ideas:

- capabilities denote authority;
- endpoints denote communication rendezvous;
- memory objects denote data and ownership;
- local inter-process communication uses bounded request/reply messaging;
- memory-bearing messages distinguish copying, moving, read-only lending, and mutable lending;
- kernel and device operations complete through retained completion records;
- notifications, readiness, completions, capabilities, and Rust `Waker`s are deliberately distinct;
- userspace services and drivers run in separate protection domains;
- one Sitas executor owns each shard’s mutable state;
- cross-shard coordination uses bounded typed messages;
- the native network interface is reliable, message-oriented capability invocation rather than sockets;
- TCP/IP remains an interoperability service;
- service names are discovery metadata, not authority;
- remote services are intended to be invoked through distributed object capabilities.

These ideas have strong historical precedents, but their particular 
combination is unusual.

---

## 2. Most directly relevant systems

### 2.1 Amoeba: distributed capability RPC

Amoeba is probably this OS’ closest historical relative.

Its model consists of:

- clients;
- userspace servers;
- objects managed by those servers;
- capabilities that identify and authorize access to objects;
- RPC as the native operation mechanism;
- location-independent communication.

This closely matches this OS’ proposed sequence:

```text
service lookup
    → capability
    → RPC invocation
    → reliable message transport
    → local or remote server
```

#### Recommended reading

- Andrew S. Tanenbaum, Sape J. Mullender, and Robbert van Renesse, [Using Sparse Capabilities in a Distributed Operating System](https://www.inf.fu-berlin.de/lehre/SS11/compsec/TanenbaumMR1986.pdf)
- Andrew S. Tanenbaum et al., [The Amoeba Distributed Operating System -- A Status Report](https://www.sciencedirect.com/science/article/pii/0140366491900589)
- Robbert van Renesse, Hans van Staveren, and Andrew S. Tanenbaum, [Performance of the world's fastest distributed operating system](https://dl.acm.org/doi/abs/10.1145/54289.54291)
- Sape J. Mullender, [The Amoeba Distributed Operating System](https://ir.cwi.nl/pub/18386/18386A.pdf)

#### Relevant questions

- Does a distributed capability identify an object, a server, or a particular server generation?
- How is it protected from forgery?
- How is authority attenuated?
- How are distributed capabilities revoked?
- What happens when a target service restarts?
- How are duplicated, delayed, or replayed requests detected?
- What delivery and execution guarantees does an invocation provide?

#### Present CharlotteOS answers

Most of these questions now have a local answer but not yet a distributed one:

| Question | Current answer |
|---|---|
| Capability identity | A local handle names a tagged kernel object in one address space. A connection names an endpoint; the name service separately returns a service generation so clients can reject stale instances. The representation of stable authority for a remote logical object remains open. |
| Forgery resistance | Local handles are opaque, monotonically allocated, non-reused table indices and are checked against the caller ASID, object-family tag, and subsystem registry. Cryptographic protection or proxy validation for a network-carried capability is not implemented. |
| Attenuation | Connection delegation intersects the requested `SEND`/`CALL` rights with available authority. Device, memory, observer, and bootstrap authority is explicitly delegated. General derivation trees and arbitrary distributed attenuation are not implemented. |
| Revocation | Endpoint closure, capability removal, address-space teardown, borrow cancellation, and service-generation replacement revoke local authority deterministically. Selective transitive and distributed revocation remain open. |
| Service restart | Implemented locally: old connections fail, a replacement registers a new generation, and clients re-resolve. A remote retry/re-resolution contract across partitions remains open. |
| Duplicate and replay detection | Local pending calls and reply tokens have unique identities and one-shot terminal transitions. Raft RPC handling has term/index and peer-identity checks. There is no general remote invocation ID, deduplication window, or replay cache yet. |
| Delivery/execution guarantee | Local IPC distinguishes a queued call cancelled before delivery from a delivered call whose reply authority is later invalidated. It does not claim transactional execution. A general remote at-most-once/at-least-once and uncertain-outcome contract remains open. |

Amoeba’s Fast Local Internet Protocol, or FLIP, is also directly relevant. It was designed 
to support location-independent RPC, group communication, and internetwork routing without 
making TCP streams the native abstraction.

---

### 2.2 EROS and KeyKOS: capabilities as authority

EROS and its predecessor KeyKOS are foundational comparisons for this OS’ capability model.

EROS demonstrates that:

- capabilities can be the uniform authorization mechanism;
- capability invocation can be efficient;
- services can be decomposed into confined components;
- capability transfer can express delegation;
- persistence and recovery can be integrated into the object model.

#### Recommended reading

- Jonathan S. Shapiro, Jonathan M. Smith, and David J. Farber, [EROS: A Fast Capability System](https://www.researchgate.net/publication/220910162_EROS_a_fast_capability_system)
- Norman Hardy, *The KeyKOS Architecture*
- Jonathan Shapiro, *EROS: A Capability System*
- CapROS and Coyotos design material, which continues this lineage

#### Relevance to this OS

The rule that a capability means authority -- not work, readiness, notification, or 
completion -- is consistent with this tradition.

CharlotteOS’s monotonic, non-reused handles prevent a stale handle from accidentally 
naming a later local object. That is useful, but it is not a complete solution to:

- capability derivation;
- transitive delegation;
- confinement;
- selective revocation;
- distributed revocation;
- authority recovery after service restart.

These systems provide a useful framework for deciding how much derivation information CharlotteOS must retain.

---

### 2.3 seL4: capabilities, endpoints, and kernel objects

seL4 is the strongest modern comparison for this OS’ local kernel model.

It combines:

- capabilities as the authorization mechanism;
- endpoints for IPC;
- explicit memory-management objects;
- user-space system services;
- a small kernel interface;
- rigorous specification and verification.

#### Recommended reading

- Gerwin Klein et al., [Comprehensive Formal Verification of an OS Microkernel](https://sel4.org/Research/pdfs/comprehensive-formal-verification-os-microkernel.pdf)
- [seL4 research and publications](https://sel4.org/Research/)
- Kevin Elphinstone and Gernot Heiser, *From L3 to seL4: What Have We Learnt in 20 Years of L4 Microkernels?*

#### Research questions suggested by seL4

- Can this OS express authority rules as invariants over capability tables?
- Should it maintain explicit capability-derivation information?
- Are memory ownership transitions atomic and formally specified?
- Can bootstrap authority be described declaratively?
- What are the exact invariants for reply tokens?
- Can cancellation, domain teardown, and lending revocation be modeled as state machines?
- Can access-control or information-flow properties be proven over the object model?

Several now have concrete, qualified answers:

| Question | Current answer |
|---|---|
| Authority invariants over capability tables | **Specified and bounded-model-checked.** `CharlotteCapability.tla` checks the unified tagged namespace, fresh handles, kind-correct removal, delegation, move rollback, and whole-AS teardown. `CharlotteIPC.tla` composes authority with endpoints, reply tokens, and memory transfers. |
| Explicit derivation information | **Answered negatively for the current design.** Delegation attenuates rights and creates a fresh handle, but the kernel does not retain a general capability-derivation tree. Consequently selective transitive revocation and confinement proofs remain open. |
| Atomic, formally specified memory ownership transitions | **Partially answered.** Copy, move, read/write borrow, cancellation, close, and teardown are executable TLA+ actions with mapped Rust linearization points. Concrete multi-registry atomicity is reviewed in the conformance map, but no refinement proof establishes equivalence to Rust. |
| Declarative bootstrap authority | **Partially implemented.** A typed launch manifest and typed bootstrap capability vector describe values and explicitly delegated name-service, device, state, endpoint, and system-observer authority. Launch policy is still assembled procedurally by the supervisor rather than derived from a complete declarative authority specification. |
| Reply-token invariants | **Implemented and model-checked in a bounded abstraction.** A token belongs to one pending call, becomes receiver-visible only on delivery, is consumable once, and is invalidated by cancellation, caller death, or endpoint teardown. |
| Cancellation, teardown, and lending state machines | **Yes, at the abstract safety level.** `CharlotteIPC`, `CharlotteCQ`, `CharlotteServiceLifecycle`, and `CharlotteDMA` cover these paths; the conformance document maps their actions to Rust. |
| Access-control or information-flow proof | **Open.** Kind/owner/rights checks and an explicitly delegated system-observer capability provide useful enforcement mechanisms, but there is no noninterference or whole-system access-control proof. |

The executable specifications live in [`tla/`](tla/README.md). They do not
constitute a refinement proof of the Rust kernel; the remaining verification
boundary and the identified linearization points are documented in
[`tla/CONFORMANCE.md`](tla/CONFORMANCE.md).

The most relevant seL4 concepts include:

- capability spaces and CNodes;
- capability derivation and revocation;
- endpoint badges;
- reply capabilities;
- untyped-memory retyping;
- capability-controlled interrupt and device access;
- declarative system initialization.

---

### 2.4 Barrelfish: the multikernel and per-core state

This OS, with the help of Sitas’s shard-per-core execution model, is 
closely related to Barrelfish’s multikernel thesis.

Barrelfish treats a multicore computer as a distributed system:

- OS state is partitioned or replicated;
- cores communicate explicitly through messages;
- shared-memory synchronization is minimized;
- hardware topology and heterogeneity are visible;
- placement is treated as an explicit policy decision.

#### Recommended reading

- Andrew Baumann et al., *The Multikernel: A New OS Architecture for Scalable Multicore Systems*, available from the [Barrelfish publication archive](https://barrelfish-test.systems.ethz.ch/documentation.html)
- Simon Gerber et al., [Not Your Parents’ Physical Address Space](https://www.usenix.org/system/files/conference/hotos15/hotos15-paper-gerber.pdf)
- Barrelfish/DC material in the [OSDI ’14 proceedings](https://www.usenix.org/sites/default/files/osdi14_full_proceedings.pdf)

#### Relationship to [`Sitas`](https://github.com/FrodeRanders/sitas) (and thus Seastar)

Both designs favor:

- explicit ownership;
- per-core mutable state;
- message-based coordination;
- reduced cache-coherence traffic;
- topology-aware placement.

The concepts should nevertheless remain distinct:

- a Sitas shard is an application-level ownership and execution domain;
- a logical processor is a schedulable hardware context;
- a kernel execution context is a thread;
- a Barrelfish-style OS node includes per-core operating-system state.

This OS correctly avoids making shard identity synonymous with processor identity.

---

### 2.5 Singularity: typed channels and ownership-moving IPC

Singularity is one of the closest precedents for this OS' memory messages.

Processes communicated through typed asynchronous channels. Messages were 
allocated in an exchange heap, and ownership moved from sender to receiver 
without copying. Static verification prevented both processes from accessing 
the same transferred object simultaneously.

#### Recommended reading

- Manuel Fähndrich et al., [Language Support for Fast and Reliable Message-Based Communication in Singularity OS](https://www.researchgate.net/publication/234761830_Language_support_for_fast_and_reliable_message-based_communication_in_singularity_OS)
- Galen Hunt and James Larus, [Singularity Design Motivation](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/tr-2004-105.pdf)
- Galen Hunt et al., [An Overview of the Singularity Project](https://www.microsoft.com/en-us/research/wp-content/uploads/2005/10/tr-2005-135.pdf)

#### Direct correspondence

| This OS                    | Singularity |
|----------------------------|---|
| Memory object              | Exchange-heap object |
| `Move`                     | Ownership-moving message |
| Typed userspace protocol   | Channel contract |
| Isolated protection domain | Software-isolated process |
| Bounded message path       | Typed channel |
| MMU-enforced exclusivity   | Language-enforced ownership |

This OS uses capabilities and page mappings rather than a managed-language 
verifier. It must therefore address several cases explicitly:

- aliases and overlapping mappings;
- partial-page transfers;
- sender or receiver death;
- reply-bound restoration;
- cancellation races;
- DMA pins;
- device ownership;
- rollback after incomplete operations.

---

### 2.6 Xous

Xous is already an explicit source for the architecture and remains 
the closest implementation-level comparison for:

- userspace servers;
- connection identifiers;
- scalar and memory messages;
- `Send`, `Lend`, and mutable lending;
- user-space name resolution;
- isolated drivers and services.

This OS extends this model with:

- first-class object capabilities;
- explicit memory objects;
- completion queues;
- shard-local async executors;
- distributed invocation;
- a stronger distinction between notification, readiness, completion, and authority.

The Xous comparison is therefore most valuable for validating the local 
IPC ABI and memory-message lifecycle.

---

## 3. A critical counterargument: transparent distribution

The networking document states:

> Remote IPC should be indistinguishable from local IPC.

This is attractive as a common API, but dangerous as a semantic promise.

The canonical critique is:

- Jim Waldo, Geoff Wyant, Ann Wollrath, and Sam Kendall, [A Note on Distributed Computing](https://waldo.scholars.harvard.edu/publications/note-distributed-computing)

Local and remote calls differ fundamentally in:

- latency;
- concurrency;
- memory semantics;
- independent lifecycle;
- observability;
- partial failure.

A local call either returns or its process fails. With a remote call, a 
client can lose contact after the server has executed the request but before 
the reply arrives. No abstraction layer can always determine which occurred.

### Recommended refinement

A safer architectural principle is:

> Local and remote services use a common capability-oriented protocol model, 
  while remote invocation explicitly exposes latency, cancellation, retry, 
  and partial-failure semantics.

The programming model should make the following concepts available:

- deadlines;
- cancellation tokens;
- idempotency keys;
- retry policies;
- duplicate detection;
- logical service identity;
- server-instance generation;
- uncertain outcomes;
- reconnect and re-resolution;
- protocol-specific recovery.

Remote invocation should not silently inherit assumptions from MMU-enforced 
local memory lending. In particular, a network partition cannot synchronously 
revoke memory or prove that a remote operation stopped executing.

---

## 4. RPC, binding, and invocation semantics

The foundational RPC reference is:

- Andrew D. Birrell and Bruce Jay Nelson, [Implementing Remote Procedure Calls](https://www.cs.cmu.edu/~15712/papers/birrell84.pdf)

It addresses:

- interface stubs;
- client/server binding;
- call identifiers;
- retransmission;
- duplicate suppression;
- server load;
- exception handling;
- transport-level optimization.

This OS should preserve distinct identities for:

```text
human-readable service name
    → logical service identity
    → service instance and generation
    → connection capability
    → individual invocation identifier
```

Collapsing these identities makes restart, replication, retry, and audit 
behavior difficult to specify.

### Other relevant IPC research

- Brian Bershad et al., *Lightweight Remote Procedure Call* -- optimized same-machine cross-domain calls.
- David Cheriton, *The V Distributed System* -- message-oriented distributed OS design.
- Thomas Anderson et al., *Scheduler Activations* -- coordination between kernel events and user-level scheduling.
- Thorsten von Eicken et al., *Active Messages* -- message-driven execution.
- Liedtke’s L4 papers -- minimal IPC mechanisms and fast paths.
- Mach IPC and ports -- transferable endpoint rights and message-based kernel services.

---

## 5. Asynchronous execution and completion queues

This OS deliberately separates two data paths:

1. endpoint IPC for invoking another protection domain;
2. submission/completion for work owned by the kernel or a device.

This is a sound distinction. Relevant research and production interfaces include:

- `io_uring`;
- Windows I/O completion ports;
- Solaris event ports;
- Mach port sets;
- BSD `kqueue`;
- Linux `epoll`;
- scheduler activations;
- user-level thread packages;
- asynchronous device queues.

The most important semantic distinction is:

```text
notification
    “state may have changed”

readiness
    “an operation would currently make progress”

completion
    “a particular submitted operation reached a terminal state”

capability
    “the holder has authority”

Rust Waker
    “poll this userspace future again”
```

This OS should retain these distinctions even when they share an aggregated 
waiting mechanism.

### Research questions

- Are terminal completion records non-lossy?
- Can a ring overflow without losing operation results?
- Is wakeup coalescing race-free?
- Does every operation have exactly one terminal transition?
- Is buffer ownership returned exactly once?
- Can an operation complete concurrently with cancellation?
- Who owns cleanup after the submitting process dies?
- Which operations require separately delegable operation capabilities?

### Present CharlotteOS answers

| Question | Current answer |
|---|---|
| Non-lossy terminal records and ring overflow | **Implemented.** A full shared ring spills entries into a kernel backlog; draining the ring flushes the backlog in order. Submission capacity provides earlier backpressure, although memory exhaustion of the backlog is not modeled as a recoverable condition. |
| Wakeup coalescing | **Implemented and bounded-model-checked.** Queue work generations, observer registration, and a post-registration recheck close the lost-wake window. Notifications may coalesce because the retained record or generation, rather than the wake itself, is authoritative. |
| Exactly one terminal transition | **Implemented and bounded-model-checked.** `InFlight → Completed` or `InFlight → CancelPending → Completed`; duplicate completion is idempotent and does not post a second CQ entry. |
| Buffer ownership returned once | **Implemented for completion-owned buffers.** The buffer stays inside the operation through cancellation and is taken only by `Completed → Observed`. IPC memory ownership and lending use separate modeled state machines. |
| Completion racing cancellation | **Defined.** Both transitions serialize on the operation state lock. Completion first yields its result and later cancellation reports `AlreadyComplete`; cancellation first forces the eventual terminal result to `Cancelled`. |
| Submitter death cleanup | **Partially answered.** Closing an address space tears down its completion namespace; IPC, service, device, DMA, and borrow teardown have explicit reconciliation paths. Operations with effects outside those managers still require resource-specific cleanup, and drain-or-leak behavior under irrecoverable hardware uncertainty remains intentional. |
| Separately delegable operation capabilities | **An initial policy exists.** Individually waited, polled, cancelled, or buffer-owning work receives a completion capability. High-rate work may use a capability-free operation ID delivered only through a selected CQ. General cross-domain delegation of an in-flight operation is not yet a supported authority pattern. |

The relevant implementation is in `completion`, with boot self-tests for
completion state, CQ integration, overflow/backlog delivery, cancellation, and
blocking waits. `CharlotteCQ.tla` checks the corresponding bounded safety
projection.

---

## 6. Userspace services, drivers, and recovery

### 6.1 What CharlotteOS already provides

The CharlotteOS manual substantially narrows the open questions in this
area. Chapters 11 and 16 describe two related but distinct lifecycle paths:

1. **Crash restart**, which starts a fresh service instance without inheriting
   the failed instance's authority or volatile state.
2. **Restart-with-state**, a graceful live-upgrade prototype that transfers
   explicitly serialized state to a replacement instance.

The documented crash-restart sequence is:

```text
service or driver failure
    → protection-domain teardown
    → capability and endpoint invalidation
    → outstanding borrow revocation
    → waiter and pending-call termination
    → device reset and grant recovery, where applicable
    → operation and DMA-buffer reconciliation
    → fresh domain and bootstrap capabilities
    → new service registration and generation
    → client re-lookup
```

The following behaviour is implemented and exercised:

- destroying a domain invalidates its capability table and client connections;
- active memory borrows are revoked and lender access is restored;
- reply tokens from the dead instance cannot be inherited;
- blocked clients receive terminal errors instead of hanging;
- a driver restart resets the device before resources are re-granted;
- submitted operations, borrowed memory, and DMA buffers are reconciled;
- the replacement receives freshly minted bootstrap capabilities;
- name-service generation tracking makes stale connections fail
  deterministically;
- clients can re-look up the name to obtain a connection to the new instance.

The manual uses `ServiceRestarted` for the service-level stale-generation
condition and, in the live-upgrade chapter, describes the lower-level outcomes
as `EndpointClosed` for undelivered queued calls and `Cancelled` for delivered
calls whose server exits before replying. The API should document precisely
where these lower-level results are translated into the service-level error.

#### Restart-with-state

The tested live-upgrade prototype adds four mechanisms:

- memory-object ownership transfer for serialized service state;
- generation tracking and fresh endpoint registration;
- deterministic endpoint closure and call cancellation;
- bootstrap delivery of the transferred state to the new domain.

The old service handles an explicit handoff request, stops accepting work,
drains what it can, serializes mutable state into memory objects, moves those
objects to the supervisor, and exits. The supervisor owns the state during the
transition and supplies it to the replacement through an extended bootstrap
contract.

This is deliberately **not zero-downtime**. Calls in the handoff window fail
and clients must re-resolve and retry. The manual identifies atomic endpoint
queue migration or a sidecar period with multiple endpoint owners as possible
future approaches to zero-downtime replacement.

The reference path currently demonstrates one embedded service image and one
state object. General image capabilities, multiple state objects,
manager-owned teardown, arbitrary-service state compatibility, bounded
handoff time, and production crash consistency remain future work.

### 6.2 What remains a protocol-level problem

CharlotteOS therefore already defines local resource reclamation and stale
connection behavior. The remaining research issue is not simply "how to
restart a service." It is how each service protocol recovers its externally
observable state.

Examples include:

- whether a delivered request changed persistent state before the server died;
- whether retrying an interrupted request is safe;
- how a filesystem recovers journal and cache state;
- how a network stack reconstructs connection state;
- how a replicated service restores its durable log and rejoins its group;
- how an upgraded implementation interprets state written by an older
  version;
- how effects already issued to external devices or remote machines are
  reconciled.

Generation tracking prevents a client from continuing to use the dead
instance. It cannot, by itself, determine the outcome of a request that crossed
the delivery boundary before failure. Protocols still need idempotency,
deduplication, transactional state, logging, or explicit uncertain-outcome
reporting where appropriate.

### 6.3 MINIX 3

MINIX 3 is highly relevant to:

- user-space device drivers;
- service supervision;
- driver restart;
- failure containment;
- reincarnation and reconnection.

Its central lesson reinforces the distinction already present in CharlotteOS:
mechanical component restart can be generic, while application-specific state
recovery cannot.

### 6.4 CuriOS

- Francis M. David et al., [CuriOS: Improving Reliability through Operating System Structure](https://www.usenix.org/legacy/event/osdi08/tech/full_papers/david/david_html/)

CuriOS studies fault containment and recovery for operating-system services. 
It is directly relevant to CharlotteOS’s supervisor and service-generation model.

### 6.5 Pebble

- Eran Gabber et al., [The Pebble Component-Based Operating System](https://www.usenix.org/legacy/publications/library/proceedings/usenix99/full_papers/gabber/gabber_html/index.html)

Pebble runs minimally privileged, replaceable system components -- including 
drivers -- in separate protection domains and optimizes transfers between them.

### 6.6 Contrasting approaches

Nooks and SafeDrive isolate or recover kernel extensions without fully moving 
them to independent user processes. They are useful comparisons for:

- performance overhead;
- fault containment;
- recovery complexity;
- compatibility with existing drivers;
- trusted computing base size.

### Remaining recovery questions

The manual already gives deterministic local rules for capabilities,
connections, loans, reply tokens, device reset, and operation reconciliation.
Further specification and testing should concentrate on:

- exact error translation among `EndpointClosed`, `Cancelled`, and
  `ServiceRestarted`;
- persistent side effects of delivered but unanswered requests;
- unacknowledged device effects that survive reset;
- externally visible network operations;
- durable recovery and state-version compatibility;
- retry and deduplication rules;
- state-transfer failure after the old instance has exited;
- rollback or fallback when the replacement cannot start;
- admission control during restart storms;
- bounded restart and handoff latency.

The documented supervisor-owned handoff state provides a sound basis for
fallback because the state is not lost with either service instance. A complete
fallback protocol, however, is still described as future work.

---

## 7. High-performance networking and zero-copy I/O

### 7.1 IX

- Adam Belay et al., [IX: A Protected Dataplane Operating System](https://www.usenix.org/system/files/conference/osdi14/osdi14-paper-belay.pdf)

IX combines:

- protected user-level networking;
- run-to-completion execution;
- per-core data paths;
- batching;
- zero-copy interfaces;
- minimal shared state.

This is highly relevant to Sitas executors, NIC buffer pools, interrupt 
moderation, and batching.

### 7.2 Arrakis

- Simon Peter et al., [Arrakis: The Operating System Is the Control Plane](https://www.usenix.org/node/186141%E2%80%AC)

Arrakis gives applications direct access to virtualized I/O while 
retaining kernel-enforced control-plane policy.

CharlotteOS takes a different approach: it delegates devices to isolated 
driver services rather than directly to ordinary applications. Arrakis is 
nevertheless valuable for:

- IOMMU design;
- virtualized device delegation;
- control/data-plane separation;
- minimizing kernel involvement on fast paths;
- storage and network protection.

### 7.3 Other useful systems

- netmap -- shared packet rings and amortized system calls;
- DPDK -- polled userspace packet processing and explicit buffer pools;
- mTCP -- per-core userspace TCP;
- Demikernel -- coroutine-oriented library OS I/O;
- Exokernel -- protected hardware multiplexing with application-defined abstractions;
- Shinjuku -- low-latency dispatch and preemption;
- Shenango and Caladan -- user-level scheduling and resource management;
- eRPC -- efficient datacenter RPC;
- RDMA research -- registered memory, remote access authority, and revocation.

### Zero-copy caution

Copy elimination moves complexity into:

- buffer ownership;
- memory registration;
- lifetime tracking;
- queue capacity;
- flow control;
- cancellation;
- DMA safety;
- recovery.

An early analysis of these tradeoffs is:

- B. Murphy, S. Zeadally, and C. J. Adams, [An Analysis of Process and Memory Models to Support High-Speed Networking in a UNIX Environment](https://www.usenix.org/conference/usenix-1996-annual-technical-conference/analysis-process-and-memory-models-support-high)

CharlotteOS’s explicit transfer modes provide a promising way to manage 
that complexity, provided their state transitions remain precise across 
failure paths.

---

## 8. Backpressure and queueing

Bounded queues appear throughout the CharlotteOS design:

- endpoint queues;
- shard mailboxes;
- completion rings;
- driver queues;
- packet pools;
- reliable-message windows.

Relevant work includes:

- SEDA, the staged event-driven architecture;
- Click modular router;
- network calculus;
- queueing theory for overload control;
- overload behavior in RPC systems;
- credit-based flow control;
- structured concurrency and cancellation.

Backpressure must propagate across abstraction boundaries:

```text
NIC descriptors
    → driver buffer pool
    → driver endpoint
    → protocol service
    → reliable-message window
    → RPC dispatch
    → application mailbox
```

If one layer converts a bounded resource into an unbounded queue, the 
architecture loses its overload guarantees.

Useful questions include:

- Where is admission control performed?
- Are control-plane messages protected from data-plane saturation?
- Is capacity reserved for cancellation, teardown, and priority traffic?
- Can request/reply cycles deadlock when all bounded queues are full?
- How are priority and backpressure propagated through chains of services?

### Present CharlotteOS answers

| Question | Current answer |
|---|---|
| Admission control | **Implemented at individual boundaries.** Endpoint enqueue returns queue-full, completion submission returns `WouldBlock`, per-LP IPI and shard-mailbox submission has a bounded `try_*` path, and device services bound their outstanding work by queue or hardware capacity. There is no node-wide admission controller. |
| Control-plane isolation | **Partial.** Some kernel-critical IPI traffic has a must-not-drop path, while ordinary cross-LP work receives backpressure. This is not a general control-plane/data-plane class system, and force-eviction in the exceptional IPI fallback is not a proof of safe overload behavior. |
| Reserved cancellation/teardown capacity | **Mostly open as an end-to-end policy.** Many cancellation and teardown transitions mutate authoritative state directly rather than enqueueing ordinary work, which avoids sharing an endpoint slot. The system does not consistently reserve queue credits for cancellation, teardown, or priority traffic across every service boundary. |
| Full-queue request/reply deadlock | **Locally tested, not globally excluded.** Endpoint queue-full behavior, FIFO preservation, cancellation, and closure are tested and modeled. No wait-for graph, lock-order proof, or end-to-end model excludes cyclic service dependencies under saturation. |
| Chained priority/backpressure | **Partial.** `QueueFull`/`WouldBlock` is explicit at several hops and queues are bounded, but priority classes and automatic propagation through a chain of services are not implemented as one policy. Each service must currently translate upstream pressure deliberately. |

The architecture can therefore claim bounded local mechanisms and explicit
failure signals. It cannot yet claim a complete overload-control theorem for
the composed system.

---

## 9. Capability security and compatibility

### 9.1 Capsicum

- Robert N. M. Watson et al., [Capsicum: Practical Capabilities for UNIX](https://research.google/pubs/capsicum-practical-capabilities-for-unix/)

Capsicum shows how capability-oriented confinement can coexist with UNIX compatibility.

It is particularly relevant to CharlotteOS’s decision to provide POSIX and 
TCP/IP as compatibility services rather than native authority models.

The major risk is reintroducing ambient authority behind capability-looking 
APIs. A POSIX personality should avoid giving a process unrestricted access 
to global filesystems, process tables, network namespaces, or device 
namespaces merely because legacy APIs expect them.

### 9.2 CHERI

CHERI provides hardware-enforced memory capabilities. It is complementary 
to CharlotteOS rather than a replacement for its object capabilities:

- CharlotteOS object capabilities authorize kernel-mediated operations;
- CHERI capabilities constrain memory references.

The combination could reduce trusted unsafe code around:

- message parsing;
- shared completion rings;
- userspace drivers;
- DMA buffers;
- protocol serialization;
- memory-object views.

### 9.3 Foundational capability research

- Jack Dennis and Earl Van Horn, *Programming Semantics for Multiprogrammed Computations*
- Henry Levy, *Capability-Based Computer Systems*
- Mark Miller, *Robust Composition*
- KeyKOS, EROS, CapROS, and Coyotos
- seL4
- Capsicum and CloudABI

---

## 10. Namespaces and discovery

CharlotteOS separates naming from authority:

```text
name
    → policy-controlled lookup
    → delegated connection capability
```

This is a strong design choice. Possessing a string should not confer authority.

Relevant systems include:

- Plan 9 per-process namespaces and 9P;
- Inferno and Styx;
- Grapevine;
- Xerox Clearinghouse;
- Sprite;
- Chorus;
- Mach ports;
- CORBA object references;
- Jini discovery and leasing.

The main design dimensions are:

- private versus global namespaces;
- logical service identity versus current instance identity;
- leases for transient registrations;
- policy-controlled resolution;
- replication-aware lookup;
- restart generations;
- caching and invalidation;
- bootstrap trust.

CharlotteOS’s node-local name service and attenuated discovery connections 
are consistent with capability discipline. A future distributed registry 
must preserve that discipline rather than turning a globally visible name 
into ambient authority.

---

## 11. Distributed consistency and replicated services

The networking document proposes Raft as a capability service. Raft is an 
appropriate starting point for replicated metadata and directory services:

- Diego Ongaro and John Ousterhout, [In Search of an Understandable Consensus Algorithm](https://raft.github.io/raft.pdf)

Additional relevant work includes:

- Paxos and Multi-Paxos;
- Viewstamped Replication;
- Zab;
- virtual synchrony;
- state-machine replication;
- leases;
- linearizability;
- fencing tokens;
- failure detectors.

Capabilities do not remove distributed consistency problems. A replicated 
capability service must still define:

- which operations are linearizable;
- how leader changes affect outstanding calls;
- whether an issued capability remains valid after rollback or reconfiguration;
- how authority survives or is withdrawn during membership changes;
- whether service generations are consensus-backed;
- how stale replicas are prevented from authorizing operations.

Distributed locks and mutable leases should use fencing tokens so that a 
former holder cannot continue operating after its lease has expired.

### Present CharlotteOS answers

CharlotteOS now contains a `no_std` adaptation of the separately tested Graft
Raft implementation rather than only an architectural proposal. It implements
durable term/vote/log state, leader no-op entries, current-term commit rules,
learners, joint consensus, automatic finalization after a catch-up fence,
linearizable-read barriers, chunked snapshots carrying membership, and
persistent object-store recovery. The boot suite exercises local multi-node
election and explicit persistent restart. The TLA+ suite separately checks
bounded election, log, membership, and snapshot safety models.

That answers the Raft-mechanism part, but not yet the distributed name-service
policy:

| Question | Current answer |
|---|---|
| Which operations are linearizable? | The Graft core defines committed client commands and quorum-contact read barriers. The distributed name service now exposes client commands (`dns::OP_REGISTER`/`OP_LOOKUP`/`OP_CALL`) that submit to the leader and replicate the `name -> node` catalog across two guests; a per-operation external linearizability contract and general query service remain future work. |
| Leader changes and outstanding calls | The core rejects non-leader commands, tracks a known leader, and requires current-term/quorum conditions. Redirect, retry, idempotency, and uncertain-outcome behavior are not yet a complete external client protocol. |
| Capabilities after rollback/reconfiguration | Raft membership authority is configuration-indexed and removed peers are decommissioned. Application capabilities issued from replicated directory state do not yet have consensus-backed epochs or rollback rules. |
| Authority during membership changes | Peer voting and leadership authority follows stable/joint voter sets; learners replicate without voting. This protects the consensus group itself, not arbitrary capabilities stored in its state machine. |
| Consensus-backed service generations | **Open.** Current name-service generations are node-local lifecycle generations. The planned clustered registry must decide whether generation allocation is a replicated command. |
| Stale replicas authorizing operations | The Raft core accepts leader RPCs only from configured voters and linearizable reads only after a quorum-contact barrier. A distributed capability issuer still needs an epoch/fencing rule so a stale service replica cannot authorize external effects. |

---

## 12. Recommended research agenda

### 12.1 Formalize the local object model

This agenda item is substantially underway. The executable models and
conformance map now cover:

- capability-table invariants;
- rights attenuation;
- endpoint lifecycle;
- reply-token linearity;
- memory transfer states;
- cancellation;
- domain death;
- driver teardown;
- completion retention.

Within their finite configurations, the state machines make double return,
use-after-move, stale reply, lost completion, stale scheduler wakeup, unsafe
DMA unpinning, and inconsistent Raft recovery invariant violations.

The remaining work is qualitatively different: add omitted concrete failure
steps, identify all Rust linearization points, construct refinement mappings,
and eventually prove that the implementation refines the abstract state
machines. Access-control and information-flow properties also remain beyond
the present safety models.

### 12.2 Define a remote invocation contract

Decide explicitly:

- at-most-once versus at-least-once transmission;
- duplicate suppression scope;
- retry lifetime;
- call-ID uniqueness;
- deadline interpretation;
- idempotency requirements;
- behavior after reconnect;
- outcome reporting when execution is uncertain.

“Reliable message” is not by itself a complete RPC contract.

### 12.3 Separate stable authority from transient routing

Consider distinct representations for:

- logical service authority;
- a resolved server instance;
- a transport route;
- a connection session;
- an individual request.

This would allow routing and server placement to change without 
silently changing authority.

### 12.4 Model distributed capability lifecycle

Study:

- cryptographically protected capabilities;
- indirection through local proxies;
- leases;
- revocation lists;
- epochs and generations;
- issuer-mediated validation;
- attenuation and delegation chains;
- confinement;
- auditability.

### 12.5 Evaluate fast paths experimentally

Measure:

- scalar IPC latency;
- page move and lending cost;
- TLB shootdown cost;
- cross-core wakeup rate;
- completion-ring overflow behavior;
- batching versus tail latency;
- driver isolation overhead;
- packet-buffer ownership transitions;
- remote RPC under loss and duplication;
- recovery time after service and driver failure.

### 12.6 Test adversarial lifecycle races

Important cases include:

- reply racing cancellation — **implemented in the state machine and exercised
  by cancellation tests; broader schedule fuzzing remains useful**;
- caller death during a mutable loan — **covered by IPC teardown tests and the
  bounded IPC model**;
- server death after consuming a moved object — **local teardown has defined
  ownership behavior; persistent/external effects remain protocol-specific**;
- device reset with DMA in flight — **driver teardown and SMMU/DMA quarantine
  paths exist and are modeled; real-hardware fault injection remains future
  work**;
- completion-ring saturation — **tested with non-lossy backlog delivery and
  modeled**;
- endpoint closure with queued calls — **tested and modeled**;
- service restart during name lookup — **waitable lookup and generation-based
  restart are exercised by the service lifecycle suite**;
- duplicate remote request after leader change — **open**;
- network partition during authority revocation — **open**.

---

## 13. Prioritized reading list

For architecture refinement, the most useful reading order is:

1. **Waldo et al., _A Note on Distributed Computing_**  
   Tests the central local/remote-transparency claim.

2. **Amoeba’s capability and RPC papers**  
   The closest precedent for distributed capability invocation.

3. **seL4’s kernel and capability research**  
   The best comparison for capabilities, endpoints, memory objects, and authority invariants.

4. **Singularity’s message-passing work**  
   The closest precedent for ownership-moving, copyless IPC.

5. **Barrelfish’s multikernel paper**  
   Grounds per-core ownership and explicit cross-core coordination.

6. **Birrell and Nelson on RPC**  
   Essential for binding, retries, duplicate suppression, and invocation semantics.

7. **IX and Arrakis**  
   Inform the userspace NIC path, batching, buffer ownership, and I/O isolation.

8. **MINIX 3, CuriOS, and Pebble**  
   Inform user-space services, supervision, restart, and driver recovery.

9. **EROS, KeyKOS, and object-capability literature**  
   Deepen the authority, delegation, confinement, and revocation model.

10. **Capsicum and CHERI**  
    Inform compatibility-layer confinement and memory safety.

11. **Borg, Omega, and Kubernetes**  
    Grounds the cluster-placement half of the server-class cluster vision (Chapter 17).

12. **TUF and SWIM**  
    Ground the signing/bootstrap and auto-discovery halves of the cluster vision.

---

## 14. Overall assessment

The CharlotteOS architecture is well aligned with several durable 
research conclusions:

- authority should be explicit;
- services should be isolated;
- kernel mechanisms should be smaller than userspace policies;
- communication boundaries should have precise ownership semantics;
- per-core state and bounded message passing can improve scalability;
- user-space drivers can improve fault containment;
- asynchronous completion should be distinct from IPC;
- zero-copy requires explicit lifetime and ownership management.

Its most ambitious claim -- making local and remote service invocation 
indistinguishable -- should be narrowed. A shared typed capability 
interface is useful, but latency, retry, cancellation, independent failure, 
and uncertain outcomes must remain visible.

The architecture’s strongest potential contribution is the integration of:

- object-capability authority;
- MMU-enforced movable and lendable memory;
- isolated userspace servers and drivers;
- shard-local asynchronous execution;
- bounded completion queues;
- native distributed service invocation.

That synthesis is coherent, but its success depends on treating lifecycle 
and failure semantics as foundational parts of the interface rather than 
transport-level implementation details.

The manual indicates that CharlotteOS already provides this as a local
service-lifecycle foundation: stale-generation detection, domain teardown,
borrow revocation, deterministic pending-call failure, driver reset,
operation reconciliation, fresh bootstrap, and a prototype stateful handoff
are defined and tested. 

The remaining gap is concentrated at the protocol and distributed-systems 
layers: durable state, versioned state transfer, externally visible effects, 
idempotent retry, uncertain outcomes, and bounded recovery.

---

## 15. Server-class cluster vision: deployment, placement, and signing

Chapter 17 of the manual (Server-Class Cluster Vision) describes intended
architecture for clusters of interchangeable server-class ARM nodes:
software is deployed to a named cluster rather than to named servers, the
cluster decides placement (declared affinity first, observed
inter-dependency second, cross-node migration third), nodes are "dumb"
compute over a shared object store, and software is validated against a
cluster-wide signing key held in replicated state. The related work falls
into four groups.

### 15.1 Interchangeable compute and processor pools

- **Amoeba** (Section 2.1) is the historical ancestor of the "pool of
  processors" model: terminals submit work that runs on any processor in
  the pool, with capability-based network-wide RPC. Amoeba deliberately did
  not implement process migration, which is the vision's extension.
- **Plan 9** splits the machine into terminals, CPU servers, and file
  servers, and centralizes authentication and key management in
  `factotum` -- an early "cluster holds the keys, not the node".
- **Inferno** continues Plan 9's model with a portable virtual machine and
  code distributed as data.
- **Jini / JavaSpaces** (Sun, later Apache River) contribute multicast
  discovery, registration through a lookup service, and the tuple-space
  "write, take, pick up" object model -- close to "software uploaded to the
  object store and picked up by whichever node it is assigned to".
- **MOSIX / openMosix** implement a single system image with automatic
  resource discovery and transparent process migration.

### 15.2 Cluster-level placement and migration

- Brendan Burns, Brian Grant, David Oppenheimer, Eric Brewer, and John
  Wilkes, [Borg, Omega, and Kubernetes: Lessons from Three Decades of
  Platform-as-a-Service](https://queue.acm.org/detail.cfm?id=2898444), ACM
  Queue 14(1), 2016. Established "deploy to the cluster, not the machine":
  declarative specs, cluster-side placement with constraints and packing,
  interchangeable machines.
- **HashiCorp Nomad and Consul** -- cluster scheduling with
  affinity/anti-affinity constraints, gossip membership
  ([https://github.com/hashicorp/memberlist](https://github.com/hashicorp/memberlist)).
- **VMware DRS and vMotion** -- declared affinity rules, load-observed
  rebalancing, and live migration with a network-level switchover: the
  full "affinity, observe, migrate" arc.
- **Erlang/OTP distribution** -- node discovery, hot code loading, and the
  "load, register, redirect, retire" upgrade pattern.
- A. Keren and A. Barak, [Opportunity Cost Algorithms for Reduction of I/O
  and Interprocess Communication Overhead in a Computing
  Cluster](https://ieeexplore.ieee.org/document/1158313), IEEE TPDS 14(1),
  2003. The earliest direct treatment of communication-aware placement,
  and the closest research precedent for interop-observed placement at
  OS-service granularity.

### 15.3 Trust, signing, and bootstrap

- **TUF (The Update Framework)** -- [theupdateframework.io](https://theupdateframework.io).
  Signed metadata describing which keys are trusted, with versioning and
  expiration; designed around key-compromise resilience. The root-key
  ceremony is the model for injecting key material into a blank-start
  cluster.
- **Uptane** -- [uptane.github.io](https://uptane.github.io). Automotive
  software-update security with offline and build-time key provisioning:
  matches the "bake a secret into the server binary at build time, but
  allow a blank start" option.
- **Sigstore / Cosign / Notary** -- the current artifact-signing ecosystem
  at deployment time.
- **Remote attestation and measured boot** (TPM, ARM CCA, AMD SEV-SNP,
  Intel SGX) -- hardware roots of trust that complement cluster-level
  validation of software with node-level validation of hardware.

### 15.4 Membership and artifact stores

- A. Gupta, K. Birman, and R. van Renesse, [SWIM: Scalable
  Weakly-consistent Infection-style Process Group Membership
  Protocol](https://www.cs.cornell.edu/projects/quicksilver/public_pdfs/SWIM.pdf),
  DSN 2002. Gossip membership and failure detection for auto-discovery;
  production form is HashiCorp
  [https://github.com/hashicorp/memberlist](https://github.com/hashicorp/memberlist).
- **Nix** -- [nixos.org](https://nixos.org). Content-addressed,
  hash-verified software store: the artifact-identity half of "software
  lives in the object store".
- **OCI container registries** with signed manifests (Notary/Cosign) -- the
  deployment-time equivalent.

The gap this vision targets: none of the above combines cluster-level
placement with the ownership discipline and the kernel/userspace boundary
this OS is built on. Interop-observed placement at OS-service granularity
is largely unexplored since Keren and Barak (2003); the shard-level
message-flow observability this OS already has (manual Chapter 9) is a
plausible basis for going beyond the existing literature.
