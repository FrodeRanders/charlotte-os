# CharlotteOS, Sitas, and Xous: A Co-Designed Architecture for Isolated Asynchronous Services

> **Status:** Architecture and implementation redirection note\
> **Audience:** CharlotteOS and sitas contributors; intended as direct
> input to Codex\
> **Purpose:** Replace the earlier *Sitas on CharlotteOS* working draft
> with a unified model that combines:
>
> -   **CharlotteOS** as the protection, scheduling, interrupt, memory,
>     and completion substrate;
> -   **sitas** as the shard-per-core userspace execution and ownership
>     model;
> -   **Xous** as the reference model for isolated userspace servers,
>     connection-oriented IPC, and MMU-enforced memory-message
>     semantics.
>
> This document distinguishes architectural commitments from experiments
> already implemented in the fork. It also identifies where the current
> direction should be preserved, corrected, or replaced.

------------------------------------------------------------------------

## 0. Current implementation status

The architecture described in this document is substantially implemented
and boot-validated on QEMU (`-M virt,gic-version=3`, Apple Silicon HVF
or TCG).  Below is a phase-by-phase summary; detailed commit evidence
is in the repository history and earlier revisions of this document.

| Phase | Description | Status |
|-------|-------------|--------|
| 1 | Capability table, rights masks, object teardown | Done |
| 2 | Endpoint IPC v1: scalar send/call/receive/reply, connection delegation, reply tokens | Done |
| 3 | Userspace name/service manager, bootstrap delivery, ELF loader, supervisor, generation tracking, long names via memory objects | Done |
| 4 | First-class memory objects: allocate, map, unmap, close, ownership accounting | Done |
| 5 | Memory IPC: Copy, Move, BorrowRead, BorrowWrite, reply-bound revocation, cancellation, server-death recovery | Done |
| 6 | CQ subsystem normalisation: operation IDs, detached submission, CQ_WAIT/CQ_WAKE, per-shard CQ partitioning, backlog batching, §8.2 32-byte richer completion records | Done |
| 7 | Sitas endpoint/CQ backend: endpoint readiness binding, unified shard wait (`CQ_WAIT`), `ShardExecutor` (budgeted polling, task wakeup from drained events), `ShardParker` seam (spin free), per-shard CQ rings, `kv::spin_recv` retired | Done |
| 8 | Userspace UART driver: delegated MMIO + IRQ, EL0 MMIO writes, interrupt-driven deferred reads, driver crash → device reset → outstanding-op reconciliation → generation-2 restart | Done |
| 9 | Virtio-net driver: PCI discovery, BAR0 + IRQ delegation, virtio init sequence, MAC read, virtqueue setup, frame TX/RX (compiles), MEMORY_GET_PHYS syscall. Protocol crates extracted. Smoltcp 0.13 adapter + TCP/IP service binary (compile). Runtime validation is blocked by HVF's EL0-MMIO limitation and remains pending on Linux KVM. | Driver built, not yet runtime-validated |
| 10 | Lock-free device interrupt delivery (deferred wake), idempotent scheduler wakes | Done |

### Success criteria met and boot-validated

1. ✓ Two EL0 domains communicate through a bounded endpoint without ASID/LP
2. ✓ Deferred async call while a shard continues other tasks
3. ✓ Moved memory object — sender locked out until return
4. ✓ Lending enforced by mappings, including death cleanup
5. ✓ Single `CQ_WAIT` for endpoint + completions + device interrupts
6. ✓ Terminal CQ results survive ring overflow (non-lossy backlog)
7. ✓ Coalesced remote shard wakes (idempotent scheduler fix)
8. ✓ Userspace UART driver with only delegated MMIO + IRQ
9. ✓ Restart invalidates stale connections + device reset + op reconciliation

10, 11, 12 are partially met or pending (see §21).

### What's next

- **Criterion 10** — runtime-validate the virtio-net driver and TCP/IP
  stack on a KVM host and close the batching/buffer-pool milestone.
- **TCP/IP service** — deploy the smoltcp adapter as a native EL0
  service consuming frames from the NIC driver endpoint.
- **Network service stack** — build the reliable-message layer, RPC,
  and distributed-object services per the networking architecture doc.

All code from Phases 1–8 and 10 is committed, boot-validated, and
pushed to both the CharlotteOS and sitas repositories.

## 1. Executive conclusion

CharlotteOS, sitas, and Xous fit together, but not by collapsing their
abstractions into one universal "async handle."

The coherent architecture is:

``` text
CharlotteOS kernel
    protection domains
    threads and LP scheduling
    capabilities and endpoint connections
    interrupt routing
    memory objects and page mappings
    operation submission and completion queues
    minimal notifications and timers

Userspace service process
    one or more externally visible IPC endpoints
    protocol-specific request/reply handling
    one sitas executor per assigned shard
    shard-local state
    typed in-process commands
    driver or service implementation

Client process
    typed protocol library
    endpoint connection capabilities
    futures backed by IPC replies or completion records
    owned buffers whose transfer mode is explicit
```

The most important redirection is:

> **Do not model all inter-process activity as asynchronous syscalls
> returning completion capabilities.**

Instead, CharlotteOS should provide **two complementary kernel
mechanisms**:

1.  **Xous-inspired endpoint IPC** for communication across protection
    domains:
    -   connection capabilities;
    -   bounded server queues;
    -   small scalar messages;
    -   page or memory-object messages;
    -   explicit copy, move, immutable-lend, and mutable-lend semantics;
    -   synchronous and deferred replies;
    -   service discovery outside the kernel.
2.  **Completion queues** for asynchronous operations whose lifecycle
    belongs to the kernel or a device:
    -   timers;
    -   device and DMA operations;
    -   asynchronous kernel work;
    -   completion of operations submitted through a resource
        capability;
    -   aggregated waiting by a sitas shard.

Sitas then supplies the third layer:

3.  **Shard-local asynchronous execution inside a process**:
    -   one executor per shard;
    -   cooperative futures;
    -   typed internal commands;
    -   bounded mailboxes;
    -   shard-owned mutable state;
    -   explicit placement and flow ownership.

This separation prevents several conceptual mistakes in the earlier
draft:

-   a Rust `Waker` is not a kernel completion;
-   a completion capability is not an IPC endpoint;
-   an endpoint connection is not merely a waitable event;
-   a shard is not identical to an LP;
-   a wake is not identical to an IPI;
-   "async throughout" does not mean that every syscall must allocate an
    asynchronous operation.

------------------------------------------------------------------------

# 

# 2. Fundamental Kernel Objects

Before discussing IPC, drivers, scheduling, or asynchronous execution,
the architecture should be understood in terms of three orthogonal
kernel abstractions.

These are the conceptual foundation of CharlotteOS.

``` text
                 Kernel Objects
                      │
      ┌───────────────┼────────────────┐
      │               │                │
 Capabilities      Endpoints      Memory Objects
      │               │                │
  Authority      Communication     Ownership
   Security        Rendezvous     Data Movement
```

Every other kernel facility is a composition of these three primitives.

## 2.1 Capabilities

Capabilities answer one question:

> "May I perform this operation?"

They represent authority, not work.

Examples include:

-   endpoint capabilities
-   connection capabilities
-   memory-object capabilities
-   interrupt capabilities
-   DMA-domain capabilities
-   timer capabilities

The capability model should remain independent of scheduling and IPC.

## 2.2 Endpoints

Endpoints answer:

> "Who am I communicating with?"

They provide bounded rendezvous between protection domains.

Endpoints know nothing about packet buffers, futures, DMA, or ownership.
They carry requests and replies.

## 2.3 Memory Objects

Memory objects answer:

> "Where is the data, and who currently owns it?"

They represent page-backed storage whose ownership can move
independently of communication.

Their responsibilities include:

-   page ownership
-   mappings
-   transfer modes
-   DMA state
-   lifetime

## 2.4 Derived facilities

Everything else should be expressible as compositions.

 | Facility          | Capabilities | Endpoints | Memory Objects |
 |-------------------|--------------|-----------|----------------|
 | Completion Queue  | ✓           | ✓        |                |
 | Reply Token       | ✓           | ✓        |                |
 | Socket            | ✓           | ✓        | ✓             |
 | Pipe              | ✓           | ✓        | ✓             |
 | Userspace Driver  | ✓           | ✓        | ✓             |
 | Network Stack     | ✓           | ✓        | ✓             |
 | Filesystem Read   | ✓           | ✓        | ✓             |
 | DMA Mapping       | ✓           |           | ✓             |

This gives the architecture an explicit design rule:

> When adding a new subsystem, first ask whether it can be expressed
> using these three primitives. New kernel primitives should be
> introduced only when they cannot be naturally composed from the
> existing ones.

# 2. The combined thesis

The three systems approach the same design space from different
directions.

### 2.1 CharlotteOS

CharlotteOS contributes the mechanisms that must be privileged:

-   address-space isolation;
-   kernel scheduling;
-   LP-local state and interrupt routing;
-   page-table manipulation;
-   MMIO and DMA authority;
-   capability tables;
-   event observation and thread wakeup;
-   syscall dispatch;
-   asynchronous completion delivery.

The CharlotteOS fork already contains substantial experimental work in
this direction, including completion capabilities, a shared
completion-queue ring, real AArch64 EL0/SVC execution, bounded IPI
queues, typed kernel mailboxes, `ShardLocal<T>`, and a no-std sitas
userspace smoke path.

### 2.2 Sitas

Sitas contributes an execution discipline:

> Only the owning shard mutates shard-local state. Cross-shard
> interaction occurs through bounded typed messages carrying owned
> values.

Its main value for CharlotteOS is:

-   one executor per shard;
-   cooperative task scheduling;
-   futures as userspace state machines;
-   bounded backpressure;
-   shard-local ownership;
-   explicit placement;
-   typed command/reply APIs;
-   structured shutdown and cancellation;
-   observability through owned snapshots.

### 2.3 Xous

Xous contributes the missing protection-domain service model.

Its most valuable ideas are:

-   services are userspace servers;
-   clients connect to servers and receive connection identifiers;
-   connections express authority;
-   IPC is a kernel primitive rather than a convention over shared
    memory;
-   small scalar messages are distinct from memory-bearing messages;
-   memory IPC distinguishes ownership transfer from borrowing;
-   mutable lending temporarily grants exclusive writable access;
-   a userspace name service maps human-readable names to otherwise
    opaque server identities;
-   drivers and system services can live outside the kernel.

CharlotteOS currently has strong completion machinery and an emerging
mailbox model. The endpoint IPC model (Phase 2) has since replaced
    mailboxes, providing a sufficiently explicit, durable
**userspace server and memory-message IPC architecture**. That is the
principal gap Xous helps expose.

------------------------------------------------------------------------

## 3. Architectural vocabulary

The implementation should use the following terms consistently.

### 3.1 Protection domain

A **protection domain** is an address space and authority boundary. It
owns:

-   a capability table;
-   memory mappings;
-   one or more threads;
-   resource accounting;
-   failure and teardown state.

A process may be the concrete implementation of a protection domain, but
architectural documents should prefer the semantic term where
appropriate.

### 3.2 Execution context

An **execution context** is a kernel-scheduled thread.

It may have:

-   LP affinity;
-   priority or scheduling class;
-   a current protection domain;
-   a wait state;
-   an associated userspace executor.

### 3.3 Logical processor

An **LP** is CharlotteOS's representation of a schedulable hardware
execution context.

An LP is not a sitas shard. A useful initial mapping is:

``` text
one sitas shard
    ↔ one LP-affine userspace thread
    ↔ normally one logical CPU
```

The architecture must retain the distinction so that CPU hotplug, test
oversubscription, migration, asymmetric CPUs, and multiple sharded
processes remain possible.

### 3.4 Sitas shard

A **shard** is a userspace ownership and execution domain:

-   one executor runs its tasks;
-   at most one task at a time mutates shard-owned state;
-   cross-shard access occurs through messages;
-   placement is explicit but is not the shard's identity.

### 3.5 Endpoint

An **endpoint** is a bounded server-side IPC receive object.

It has:

-   an owning protection domain;
-   a queue capacity;
-   a protocol/interface identity;
-   receive rights;
-   lifecycle and closure state.

### 3.6 Connection capability

A **connection capability** is a client-side authority to send or call
an endpoint.

Possession of a service name is not authority. A name service or policy
service resolves a name and conditionally returns a connection
capability.

### 3.7 Message

A **message** is a kernel-validated IPC envelope consisting of:

-   interface or protocol identifier;
-   opcode;
-   flags;
-   inline scalar words;
-   zero or more memory-object attachments;
-   zero or more delegated capability attachments;
-   optional reply authority.

The kernel understands the envelope, not arbitrary Rust types.

### 3.8 Reply token

A **reply token** is one-shot authority to complete a blocking or
deferred call.

It is:

-   created by the kernel;
-   bound to the caller's pending call;
-   consumable exactly once;
-   invalidated by cancellation, caller death, or endpoint teardown;
-   optionally delegable when explicitly allowed.

### 3.9 Resource capability

A **resource capability** authorizes operations on a kernel-managed
object such as:

-   a memory object;
-   an endpoint;
-   a connection;
-   an interrupt;
-   an MMIO region;
-   a DMA domain;
-   a device;
-   a completion queue;
-   a timer.

### 3.10 Operation and completion queue

An **operation** is submitted work whose progress may depend on the
kernel, hardware, or a privileged service.

A **completion queue (CQ)** is the aggregated, per-shard or per-process
destination for operation results. It is waitable and has non-lossy
terminal completion semantics.

### 3.11 Notification

A **notification** means:

> State may have changed; inspect the corresponding object or queue.

It may be coalesced. It is not itself a result.

### 3.12 Rust `Waker`

A Rust `Waker` means:

> Poll this userspace future again.

It is a userspace scheduling hint. It is not a kernel completion record
and should not cross the ABI directly.

------------------------------------------------------------------------

## 4. What "asynchronous throughout" means

"Asynchronous throughout" should be retained as a design goal, but
defined precisely.

It means:

1.  operations that may wait for external progress have
    submission/completion semantics;
2.  no subsystem boundary forces the caller to dedicate a thread to
    waiting for one operation;
3.  a userspace shard can wait on one aggregated kernel source;
4.  completions are retained until observed;
5.  buffers and capabilities have explicit ownership during an
    operation;
6.  cancellation and teardown have defined state transitions;
7.  bounded queues preserve backpressure across layers;
8.  service implementations can suspend one task while continuing other
    work on the shard.

It does **not** mean:

-   every syscall is asynchronous;
-   every call returns a newly allocated completion capability;
-   no thread ever blocks;
-   every wake sends an IPI;
-   all waitable objects have identical semantics;
-   userspace code is interrupted by arbitrary asynchronous callbacks.

### 4.1 Blocking is still required

When no sitas task is runnable, the shard's execution context should
block in one kernel wait operation.

That is desirable:

``` text
ready tasks exist
    → poll tasks

no ready tasks
    → wait on CQ / endpoint readiness / deadline

event arrives
    → execution context becomes runnable
    → drain events
    → wake relevant futures
```

CharlotteOS's advantage is not "threads never park." Its advantage is:

> The shard parks directly on native completion and message state; no
> helper-thread pool is required merely to convert blocking kernel
> interfaces into futures.

### 4.2 Immediate operations remain immediate

Operations that cannot meaningfully block should complete synchronously.
Examples include:

-   capability duplication;
-   metadata queries;
-   nonblocking CQ drain;
-   validation-only operations;
-   querying current process/thread identity;
-   closing an already quiescent object.

Potentially waiting operations must have an asynchronous form. They may
still complete immediately.

### 4.3 No asynchronous userspace upcalls

Ordinary completions should not inject callbacks into userspace at
arbitrary instructions.

The path should be:

``` text
hardware interrupt
    → kernel records result or notification
    → blocked execution context becomes runnable
    → normal return to userspace
    → sitas executor drains CQ/endpoints
    → executor wakes relevant Rust tasks
```

Kernel preemption is allowed. Arbitrary userspace task upcalls are not
required.

------------------------------------------------------------------------

## 5. Two kernel data paths, not one

The earlier CharlotteOS/sitas draft leaned toward a universal
completion-capability model. Xous shows why that is insufficient.

CharlotteOS needs two distinct paths.

## 5.1 Endpoint IPC path

Use endpoint IPC when one protection domain invokes another protection
domain.

Examples:

-   application → network service;
-   network service → NIC driver;
-   application → filesystem service;
-   filesystem service → block service;
-   service → name service;
-   driver manager → driver process.

The receiver is a userspace server, not the kernel operation engine.

``` text
client task
    → send/call through connection capability
    → kernel validates and enqueues message
    → server endpoint becomes ready
    → server shard receives message
    → service handles or delegates work
    → optional reply token completed
```

## 5.2 Operation completion path

Use submission/completion when the operation belongs to the kernel or
hardware-facing mechanism.

Examples:

-   timer expiry;
-   asynchronous page operation;
-   IRQ-derived device completion;
-   DMA completion;
-   waiting for thread/process exit;
-   kernel-managed asynchronous object work.

``` text
userspace submits operation
    → kernel validates resource capability and buffers
    → operation becomes in flight
    → result posted to CQ
    → shard wakes and consumes result
```

## 5.3 Composition

A userspace driver may receive an endpoint message and then submit a
kernel operation.

Example:

``` text
network service sends TX packet to NIC driver endpoint
    → NIC driver validates message
    → NIC driver submits/updates hardware descriptor
    → IRQ arrives
    → driver CQ receives device completion
    → driver replies or posts protocol completion to network service
```

The endpoint message and the hardware completion are related, but they
are not the same kernel abstraction.

------------------------------------------------------------------------

## 6. Xous-inspired IPC for CharlotteOS

## 6.1 Server and connection model

CharlotteOS should add kernel objects analogous to Xous servers and
connections, but use capability terminology explicitly.

Proposed conceptual API:

``` rust
fn endpoint_create(
    interface: InterfaceId,
    version: InterfaceVersion,
    capacity: usize,
) -> Result<EndpointCap, IpcError>;

fn connection_mint(
    endpoint: &EndpointCap,
    rights: ConnectionRights,
) -> Result<ConnectionCap, IpcError>;

fn connection_delegate(
    connection: &ConnectionCap,
    target_domain: ProtectionDomainCap,
    reduced_rights: ConnectionRights,
) -> Result<(), IpcError>;
```

Ordinary clients should not mint arbitrary connections. A service
manager or name/policy service normally delegates them.

## 6.2 Name service outside the kernel

Follow Xous's principle:

-   the kernel uses opaque endpoint identities and capabilities;
-   a userspace name service maps human-readable names to services;
-   lookup is policy controlled;
-   service restart changes instance generation;
-   stale connections fail deterministically.

Suggested identity:

``` rust
struct ServiceInstanceId {
    service_uuid: [u8; 16],
    generation: u64,
}
```

A name such as `"system.network.v1"` is discovery metadata, not
authority.

## 6.3 Message classes

CharlotteOS should support at least:

``` rust
enum MessageKind {
    ScalarSend,
    ScalarCall,
    MemorySend,
    MemoryCall,
}
```

The public API may expose more ergonomic operations, but the ABI should
keep small messages distinct from memory-bearing messages.

### Scalar message

Suitable for:

-   opcodes;
-   handles;
-   small numeric arguments;
-   status values;
-   queue indices;
-   compact control-plane operations.

### Memory message

Suitable for:

-   structured requests;
-   strings and names;
-   packets or blocks;
-   larger replies;
-   descriptors and configuration structures.

## 6.4 Memory transfer modes

Adopt the strongest Xous idea and generalize it around CharlotteOS
memory objects:

``` rust
enum TransferMode {
    Copy,
    Move,
    BorrowRead,
    BorrowWrite,
}
```

### Copy

-   receiver gets copied data;
-   sender retains ownership;
-   simplest semantics;
-   suitable for small or untrusted input.

### Move

-   ownership transfers to the receiver;
-   sender loses mappings and use rights;
-   receiver becomes responsible for later transfer or destruction;
-   safe Rust wrapper consumes the buffer value.

### BorrowRead

-   receiver receives temporary read-only mapping;
-   sender retains ownership;
-   reply or call completion revokes receiver mapping;
-   writable aliases in the receiver are forbidden.

### BorrowWrite

-   receiver receives temporary exclusive writable mapping;
-   sender access is revoked for the duration;
-   reply or call completion restores sender ownership/access;
-   mutable lending must be hardware enforced, not inferred from Rust
    types.

The implementation must not rely on safe Rust alone. Unsafe code or a
non-Rust process must still be contained by page tables and capability
checks.

## 6.5 Proposed message envelope

``` rust
#[repr(C)]
struct MessageHeader {
    interface: u128,
    version: u32,
    opcode: u32,
    flags: u32,
    inline_len: u16,
    segment_count: u16,
    capability_count: u16,
    reserved: u16,
    user_tag: u64,
}

#[repr(C)]
struct MemoryAttachment {
    object: CapabilityId,
    offset: u64,
    len: u64,
    mode: TransferMode,
}
```

Constraints:

-   fixed-width ABI fields;
-   explicit alignment;
-   no Rust enum layout assumptions;
-   no persistent use of `usize`;
-   checked offsets and lengths;
-   explicit protocol versioning;
-   maximum attachment counts;
-   no arbitrary userspace pointers as durable message identity.

## 6.6 Typed Rust protocols remain in userspace

The kernel does not understand `M: Send`, Rust enums, or drop semantics.

A protocol crate supplies typed wrappers:

``` rust
trait Protocol {
    const INTERFACE: InterfaceId;
    const VERSION: u32;

    type Request;
    type Response;
}
```

Generated or hand-written bindings encode requests into the kernel
message envelope.

``` rust
struct Client<P: Protocol> {
    connection: ConnectionCap,
    _protocol: PhantomData<P>,
}
```

The existing kernel `ShardMailbox<M>` remains useful internally. It must
not be mistaken for the stable userspace IPC ABI.

------------------------------------------------------------------------

# 6A. Memory Objects and Ownership Transfer

The endpoint IPC model defines **who communicates** and **how requests
are routed**. The memory-object model defines **how data crosses
protection-domain boundaries**.

These are independent concerns and should evolve independently.

## 6A.1 First-class memory objects

CharlotteOS should introduce a dedicated kernel object representing
page-backed memory with explicit ownership and capability semantics.

The kernel manages:

-   physical frames;
-   mappings;
-   ownership;
-   transfer state;
-   DMA state;
-   lifetime.

Userspace never exchanges raw pointers across protection domains.
Instead, IPC messages attach **memory-object capabilities**.

## 6A.2 Transfer semantics

The architecture adopts four transfer modes:

-   **Copy** --- duplicate data.
-   **Move** --- transfer exclusive ownership.
-   **BorrowRead** --- temporary immutable mapping.
-   **BorrowWrite** --- temporary exclusive writable mapping.

The MMU enforces these semantics. Rust ownership complements, but does
not replace, kernel enforcement.

## 6A.3 Ownership rather than copying

The preferred primitive is:

> Transfer ownership by changing mappings instead of copying bytes.

For `Move`, the kernel:

1.  validates ownership;
2.  reserves destination queue space;
3.  prepares receiver mappings;
4.  removes sender mappings;
5.  performs required TLB invalidation;
6.  commits ownership;
7.  publishes the IPC message.

Physical pages never move.

## 6A.4 Control plane and data plane

Endpoint IPC belongs to the control plane.

Memory objects belong to the data plane.

For networking:

-   endpoint IPC configures queues and drivers;
-   descriptor rings coordinate producer/consumer state;
-   packet payloads move as memory objects.

This avoids turning every packet into a copied IPC payload while
preserving strict ownership.

## 6A.5 DMA

DMA introduces a third ownership dimension:

-   CPU ownership;
-   DMA ownership;
-   Rust/API ownership.

Future IOMMU/SMMU integration should extend capability enforcement to
DMA transactions.

## 6A.6 Why this section comes before request/reply

Request/reply semantics build upon the memory model.

Once endpoint IPC and transfer semantics are defined, synchronous calls,
deferred replies, userspace drivers, and networking all become natural
applications of the same primitives.

------------------------------------------------------------------------

## 7. Request/reply and asynchronous service calls

Xous is commonly synchronous at the client API even though communication
is message based. CharlotteOS and sitas should retain both
synchronous-looking futures and true asynchronous service execution.

## 7.1 Client call

A client performs:

``` rust
let response = network.connect(address).await?;
```

The Rust API is asynchronous. Underneath it:

1.  the runtime sends a call message;
2.  the kernel creates a one-shot reply token;
3.  the caller task becomes pending;
4.  the caller shard continues running other tasks;
5.  the server receives the message;
6.  the server replies immediately or retains/delegates the reply token;
7.  reply arrival wakes the client future.

No client OS thread is dedicated to the request.

## 7.2 Deferred replies

A server must be able to retain reply authority while waiting for other
work:

``` rust
struct PendingRequest {
    reply: ReplyToken,
    buffer: BorrowedWriteBuffer,
}
```

This is essential for:

-   network receive;
-   filesystem read;
-   device operations;
-   service chains;
-   timeouts;
-   batching.

## 7.3 Service-side task routing

The external endpoint receiver should be an ingress adapter. It routes
work to a sitas shard:

``` rust
async fn endpoint_loop(
    endpoint: Endpoint,
    shards: ShardedSubmitter,
) -> Result<(), ServiceError> {
    loop {
        let envelope = endpoint.receive().await?;
        let shard = route(&envelope);

        shards.submit_to(shard, async move {
            handle_message(envelope).await
        }).await?;
    }
}
```

The service's mutable state remains shard-owned.

## 7.4 Avoid one server loop as a scalability constraint

Do not copy a simplistic "one process, one blocking receive loop"
structure from Xous.

A CharlotteOS service process may own:

-   several endpoints;
-   several receiving tasks;
-   several sitas shards;
-   one or more completion queues;
-   per-shard endpoint ingress queues.

The server abstraction is a protection and authority boundary, not a
single-thread requirement.

------------------------------------------------------------------------

## 8. Completion queues for kernel and device operations

## 8.1 Aggregated wait

A sitas shard should wait on one kernel-managed CQ or wait set, not
independently wait on every operation capability.

``` text
many operations
    → one per-shard CQ
    → one executor wait
```

## 8.2 Completion record

Suggested minimum:

``` rust
#[repr(C)]
struct CompletionEntry {
    operation: OperationId,
    user_data: u64,
    status: CompletionStatus,
    result: i64,
    flags: u32,
    returned_capability: CapabilityId,
}
```

A result may also refer to memory ownership returned through a
registered operation table.

## 8.3 Non-lossy terminal results

The current fork's decision to retain CQ overflow in a kernel backlog is
correct and must become an architectural invariant:

> A terminal operation result must never be silently lost because the
> userspace CQ ring is full.

The ring's overflow counter is pressure telemetry, not a count of
discarded completions.

The kernel may:

-   retain overflow in a per-domain backlog;
-   stop accepting new submissions;
-   propagate `WouldBlock`;
-   force the application to drain completions.

It must not discard terminal results.

## 8.4 Per-operation capability is optional

The existing experimental model returns a `CompletionCap` per
submission. Do not make this mandatory in the final ABI.

Use:

-   `OperationId` plus a CQ for the common path;
-   a first-class operation capability only where independent waiting,
    cancellation, delegation, inspection, or policy requires it.

This avoids capability-table pressure for high-rate operations.

## 8.5 Race-free waiting

CQ waiting must atomically avoid lost wakeups:

``` text
inspect CQ
if completion threshold met:
    return

arm waiter against CQ generation/state
reinspect CQ
if threshold met:
    disarm and return

block
```

The observer implementation is an internal mechanism. The ABI contract
is that state transition and waiter registration cannot lose a wake.

**Current implementation.**  The kernel uses a per-CQ monotonic
`work_generation` counter paired with a `last_seen_generation` field.
Every `complete()`, `complete_detached()`, and `wake()` bumps the
generation; `wait_on_cq` blocks until `work_generation !=
last_seen_generation` and saves the new value on return.  This
decouples the wait condition from the shared ring's `pending()` count,
solving the undrained-ring problem: callers that poll individual
completions via `poll(cap)` and never drain the shared ring are not
stuck in a busy-spin simply because the ring accumulated stale
entries.  The lost-wake guard re-checks the generation after the
waiter is registered, re-admitting the thread if work arrived during
the registration window.

### 8.6 LP affinity

Each execution context is assigned an affinity LP on first admission.
Re-admission after a wake (timer expiry, endpoint message, device
interrupt) returns the thread to its affinity LP rather than scanning
for the globally least-loaded LP.  This keeps timer events on the same
LP's queue, eliminates cross-LP migration races (observer-not-found
when `signal_cq` runs on a different LP than the one where the Waker
was registered), and preserves cache warmth.

------------------------------------------------------------------------

## 9. Sitas inside a service process

## 9.1 One executor per shard

Each assigned shard normally has:

-   one LP-affine execution context;
-   one sitas executor;
-   one ready queue;
-   one timer structure;
-   one CQ or CQ partition;
-   shard-owned service state.

## 9.2 Internal messages are not kernel IPC

Within one protection domain, use sitas typed mailboxes where practical:

``` text
external cross-process boundary:
    CharlotteOS endpoint IPC

internal cross-shard boundary:
    sitas typed owned messages
```

Do not route every internal command through kernel endpoint IPC. That
would add unnecessary privilege crossings and ABI encoding.

## 9.3 Shard-local state

Retain the closure-based non-escaping `ShardLocal<T>` discipline, with
explicit kernel constraints.

For kernel `ShardLocal<T>`, access is valid only when:

-   execution is bound to the owning LP;
-   migration cannot occur during access;
-   no interrupt or deferred path accesses the same object;
-   re-entrant access is rejected;
-   panic/unwind behavior cannot leave the borrow flag permanently set.

Keep `PerLp<T>` for data requiring interrupt-safe or cross-LP access. Do
not present `ShardLocal<T>` as a universal replacement.

## 9.4 Cross-shard wake semantics

Replace the mental model:

``` text
wake = IPI
```

with:

``` text
wake = make the target shard eligible to observe queued work
```

Implementation rules:

-   enqueue before signaling;
-   send an IPI only when required;
-   coalesce wakes;
-   prefer empty-to-nonempty transition signaling;
-   avoid one IPI per message;
-   let the scheduler decide whether a remote reschedule interrupt is
    necessary.

------------------------------------------------------------------------

## 10. Userspace drivers

Xous's userspace-driver orientation should become a central CharlotteOS
direction.

## 10.1 Driver process authority

A driver manager creates a driver protection domain and delegates only
required resources:

``` rust
struct DriverGrant {
    device: DeviceCap,
    mmio: Vec<MmioRegionCap>,
    interrupts: Vec<InterruptCap>,
    dma_domain: Option<DmaDomainCap>,
    bootstrap_endpoint: EndpointCap,
}
```

A driver must not obtain arbitrary physical memory or arbitrary
interrupt vectors.

## 10.2 Interrupt delivery

The kernel interrupt path should:

1.  identify and acknowledge/mask the interrupt as required;
2.  mark an interrupt object pending;
3.  post a notification or CQ entry;
4.  make the owning driver shard runnable;
5.  return from the exception.

Repeated interrupts should coalesce where the device model permits it.

## 10.3 DMA isolation

Strong userspace-driver isolation requires an IOMMU/SMMU:

-   CPU page tables protect against driver CPU access;
-   IOMMU/SMMU protects against device DMA;
-   Rust ownership protects safe client code from accidental reuse.

Without an IOMMU/SMMU, userspace drivers still improve fault containment
and architecture, but a malicious or broken DMA device can violate
memory isolation.

## 10.4 Driver service layering

Use separate failure and authority domains where justified:

``` text
Application
    → socket endpoint
Network service
    → packet endpoint
NIC driver
    → MMIO / DMA / IRQ
Hardware
```

An initial implementation may combine socket and protocol-stack
functions, but the NIC driver should remain separately isolated.

## 10.5 Control plane versus data plane

Do not interpret message passing as "one IPC per packet descriptor."

### Control plane

Use endpoint IPC for:

-   queue creation;
-   device configuration;
-   buffer-pool registration;
-   MTU and link settings;
-   service discovery;
-   reset and lifecycle;
-   authority delegation.

### Data plane

Use:

-   shared descriptor rings;
-   transferred memory objects;
-   buffer-pool ownership;
-   batching;
-   per-queue/per-shard layout;
-   coalesced notifications;
-   CQ batches.

Example:

``` text
network shard writes N TX descriptors
    → signals only on required transition
NIC driver/hardware processes descriptors
    → produces N completions
driver posts batch notification/completion
network shard drains all available entries
```

------------------------------------------------------------------------

## 11. Backpressure

Backpressure must be defined end to end.

``` text
application request queue
    ↕
service endpoint queue
    ↕
service shard mailbox
    ↕
driver endpoint/ring
    ↕
kernel submission queue
    ↕
hardware descriptor ring
```

Every bounded layer needs explicit behavior:

``` rust
enum SendPolicy {
    Try,
    Wait,
    Deadline(Instant),
    DropIfCoalescible,
}
```

Rules:

-   terminal completions are never dropped;
-   coalescible notifications may be merged;
-   control messages normally fail or wait explicitly;
-   data-plane producers stop before unbounded allocation;
-   high-priority kernel-internal messages must not use arbitrary
    force-eviction of unrelated work;
-   emergency paths require reserved capacity or a separate priority
    queue.

The current bounded IPI queue is a useful experiment, but "force-evict
fallback" is not a durable general policy. Replace it with one or more
of:

-   reserved slots for mandatory kernel messages;
-   separate priority classes;
-   per-message retry/defer behavior;
-   synchronous fallback only where proven safe.

------------------------------------------------------------------------

## 12. Cancellation, teardown, and ownership

## 12.1 Operation state

Use an explicit state machine:

``` text
Created
    → Submitted
    → Accepted | Rejected
    → InFlight
    → Completed | Failed | Cancelled
    → ResultObserved
    → ResourcesReclaimed
```

## 12.2 Cancellation result

``` rust
enum CancelResult {
    Cancelled,
    AlreadyCompleted,
    TooLate,
    NotCancellable,
    UnknownOperation,
}
```

Cancellation may itself complete asynchronously.

## 12.3 Buffer ownership

For an operation receiving an owned buffer:

``` rust
async fn read(
    resource: ResourceCap,
    buffer: OwnedBuffer,
) -> Result<(OwnedBuffer, usize), ReadError>;
```

Safe Rust prevents ordinary reuse after move. The kernel must
additionally ensure:

-   memory-object ownership is valid;
-   pages remain stable while used;
-   DMA mappings are controlled;
-   ownership is returned exactly once;
-   cancellation does not free memory still reachable by the kernel or
    device;
-   domain death triggers cleanup, quarantine, reset, or deliberate leak
    according to policy.

Retain the sitas "drain or leak rather than use-after-free" principle
for experimental paths, but design the production kernel to reclaim
through explicit operation and device teardown.

## 12.4 IPC call cancellation

Cancellation of an endpoint call differs from cancellation of a kernel
operation.

The kernel must define:

-   whether an enqueued but unreceived call can be removed;
-   whether a delivered call can be marked cancelled;
-   whether the server may still complete it;
-   how lent mappings are revoked;
-   what happens to a delegated reply token;
-   whether cancellation is advisory or authoritative.

Do not reuse kernel-operation cancellation semantics blindly for service
calls.

------------------------------------------------------------------------

## 13. Service supervision and restart

Userspace drivers and services make failure semantics part of the
architecture.

A service manager should:

-   start protection domains;
-   grant capabilities;
-   register service names;
-   monitor process exit;
-   revoke stale connections;
-   restart according to policy;
-   increment service generation;
-   reset devices before driver restart;
-   reconcile or fail outstanding calls.

Stale client connections should fail with a clear error such as:

``` rust
enum IpcError {
    EndpointClosed,
    ServiceRestarted,
    PermissionDenied,
    QueueFull,
    DeadlineExceeded,
    Cancelled,
    InvalidMessage,
    ResourceExhausted,
}
```

A restarted service must not silently inherit outstanding reply tokens
or DMA ownership from the previous instance.

------------------------------------------------------------------------

## 14. Scheduling and priority propagation

A synchronous-looking service chain can suffer priority inversion:

``` text
high-priority application
    calls filesystem
        calls block service
            calls driver
```

The initial implementation may use ordinary priority inheritance on
endpoint waiters.

The ABI should leave room for:

-   deadline propagation;
-   priority inheritance;
-   scheduling-context or budget donation;
-   cancellation propagation;
-   admission control.

Do not encode these policies directly into sitas task priority. They
cross protection and trust boundaries and require kernel mediation.

### 14.1 Load rebalancing (future)

The kernel includes a `try_rebalance()` skeleton that detects
idle-LP/overloaded-LP pairs and migrates threads with no active timers
to the idle LP.  Threads with active timers are excluded because timer
events live on the affinity LP's per-LP queue; migrating them would
orphan the timer.  The migration clears the thread's `affinity_lp` so
it finds a new home.

Currently disabled: migrating threads mid-IPC can break request/reply
protocols.  Activation requires per-thread "safe to migrate" tracking
in the IPC layer.

------------------------------------------------------------------------

## 15. What to retain from the current fork

The following work aligns with the combined architecture and should
continue.

### Retain and harden

-   AArch64 EL0/SVC path;
-   caller identity derived from the current trap/thread context;
-   per-address-space capability tables;
-   shared CQ ring;
-   non-lossy kernel CQ backlog;
-   `CQ_WAIT` blocking through scheduler/observer machinery;
-   bounded queues and explicit `WouldBlock`;
-   no-std sitas split and CharlotteOS backend;
-   per-shard executor model;
-   typed in-kernel `ShardMailbox<M>`;
-   closure-based `ShardLocal<T>` with restricted use;
-   segment-aware ELF loading as an experimental base;
-   QEMU boot CI across AArch64 and x86-64.

### Reframe

-   completion capability work as the **operation completion
    subsystem**, not the universal service IPC subsystem;
-   observer/waker code as internal scheduling machinery, not the public
    semantic identity of all async objects;
-   IPI queue closures as a temporary kernel work mechanism, not the
    long-term cross-domain messaging model;
-   the current mailbox syscall ABI as a smoke-test interface only.

------------------------------------------------------------------------

## 16. What to rethink or replace

This section is the explicit redirection plan.

## 16.1 Stop expanding raw mailbox syscalls

### Current direction

The fork has incremental sender/receiver mailbox capability syscalls,
built from a typed kernel mailbox and earlier raw-LP smoke calls.

### Problem

A mailbox tied directly to LP identity is not a complete userspace
service abstraction:

-   it exposes placement where authority should be exposed;
-   it lacks protocol identity;
-   it conflates shard transport with protection-domain IPC;
-   it does not define scalar versus memory messages;
-   it lacks move/lend semantics;
-   it risks making service topology part of the ABI.

### Redirect

Implement endpoint and connection capabilities independent of LP
identity.

The endpoint owner may choose which shard receives a message. Clients
address the service connection, not a target LP.

## 16.2 Do not allocate one completion capability for every service request

### Current direction

The async-syscall model tends toward `submit -> CompletionCap`.

### Problem

For userspace service calls, the natural object is a reply token
associated with an endpoint call. For high-rate kernel operations,
per-operation capabilities may create unnecessary table pressure.

### Redirect

Use:

-   reply tokens for endpoint calls;
-   operation IDs plus CQ entries for ordinary kernel operations;
-   first-class operation capabilities only where independent authority
    is required.

## 16.3 Separate readiness, notification, and completion

### Current direction

Earlier prose treats waitable handles, observers, wakers, and
completions as essentially the same object.

### Problem

Their contracts differ:

-   readiness means progress may be possible;
-   notification means inspect state;
-   completion means an operation changed state and has result data;
-   waker means poll a future again;
-   capability means authority.

### Redirect

Keep one aggregated wait mechanism, but define distinct object semantics
and event records.

## 16.4 Replace arbitrary cross-LP closures

### Current direction

Kernel IPI RPC can carry boxed `FnOnce()` work.

### Problem

-   hard to account;
-   difficult to inspect;
-   brittle under backpressure;
-   obscures authority and lifetime;
-   unsuitable as a durable ABI model;
-   unsafe recovery/downcast paths have already caused concern.

### Redirect

Use closed kernel enums for mandatory kernel RPCs and typed mailbox
instances for subsystem-specific work.

For generic deferred kernel work, use an allocated work object with a
stable vtable/lifecycle owned by one subsystem, not an unrestricted
closure transported through the IPI layer.

## 16.5 Do not equate shard wake with IPI delivery

### Current direction

Remote wake maps directly to `send_ipi(target_lp)`.

### Problem

This can generate excessive interrupts and embeds mechanism in the
abstraction.

### Redirect

Queue first, mark pending, and send a coalesced reschedule IPI only when
the target cannot otherwise observe the transition promptly.

## 16.6 Introduce memory objects before rich IPC

### Current direction

The completion ABI and mailbox work are ahead of a general cross-process
memory-transfer model.

### Problem

Xous's most important contribution---move/lend semantics---cannot be
implemented safely with transient pointers or an ad hoc global result
page.

### Redirect

Prioritize:

1.  memory-object capability;
2.  mapping and unmapping;
3.  move between domains;
4.  temporary read lending;
5.  temporary exclusive mutable lending;
6.  revocation on reply/cancel/death;
7.  optional DMA pin/map integration.

Only then promote rich IPC payloads.

## 16.7 Replace global smoke-test ABI surfaces

### Current direction

Some experimental paths rely on fixed/global result pages, embedded test
images, and smoke-specific launch metadata.

### Redirect

Build a general process loader and startup contract:

-   ELF `PT_LOAD` mapping;
-   stack and guard pages;
-   declared heap or allocator bootstrap;
-   initial capability vector;
-   argument/environment block;
-   bootstrap endpoint;
-   no caller-supplied ASID;
-   no global shared result page.

## 16.8 Treat service discovery as userspace policy

Do not add string-based global service lookup to the kernel.

Build a userspace name/policy service that receives bootstrap authority
from the service manager and delegates connection capabilities.

------------------------------------------------------------------------

## 17. Proposed implementation sequence

The implementation order should change from "complete async syscall ABI,
then generalize mailboxes" to the following.

## Phase 0 --- Preserve the working baseline [done]

Before architectural changes:

-   keep the current booting `dev` branch green;
-   retain AArch64 and x86-64 CI;
-   document all current syscall numbers and smoke-only interfaces;
-   mark experimental ABI modules explicitly;
-   add tests that distinguish completion, notification, and mailbox
    behavior.

## Phase 1 --- [done] Capability and object model cleanup

Deliverables:

-   unified `CapabilityId` representation;
-   rights masks;
-   generation-safe lookup;
-   per-protection-domain capability table;
-   object teardown hooks;
-   transfer/delegation rules;
-   explicit object types.

Required object types:

``` text
Endpoint
Connection
ReplyToken
CompletionQueue
MemoryObject
Interrupt
MmioRegion
DmaDomain
Device
Timer
```

Decision gate:

> Can stale, forged, cross-domain, and wrong-type capabilities be
> rejected uniformly?

## Phase 2 --- [done] Endpoint IPC v1: scalar messages

Implement:

-   endpoint creation;
-   connection delegation;
-   bounded receive queue;
-   scalar send;
-   scalar call;
-   reply token;
-   deferred reply;
-   close and domain-death cleanup;
-   endpoint wait/readiness integration;
-   explicit queue-full behavior.

(Rich memory-message IPC was added in Phase 5.)

Reference test:

``` text
EL0 client
    → scalar call
EL0 echo service
    → deferred reply from sitas task
client future completes
```

## Phase 3 --- [done] Userspace name and service manager

Implement:

-   service instance identity;
-   name registration;
-   policy-controlled lookup;
-   connection-capability delegation;
-   generation changes on restart;
-   stale connection errors;
-   bootstrap capability delivery.

Reference services:

-   log service;
-   echo service;
-   timer facade.

## Phase 4 --- [done] Memory objects (foundation for all rich IPC)

Implement:

-   allocation;
-   mapping;
-   unmapping;
-   protection changes;
-   ownership accounting;
-   capability delegation;
-   move between domains;
-   teardown on process death.

Reference test:

``` text
client allocates memory object
    → moves to service
service modifies and moves it back
client cannot access while ownership is absent
```

## Phase 5 --- [done] Xous-style memory IPC

Implement:

-   `Copy`;
-   `Move`;
-   `BorrowRead`;
-   `BorrowWrite`;
-   reply-bound revocation;
-   cancel and server-death recovery;
-   mapping alias checks;
-   typed userspace wrappers.

Reference tests must include malicious/unsafe-style attempts:

-   sender accesses moved object;
-   receiver writes read-only lend;
-   sender accesses mutable lend while outstanding;
-   service dies while holding lend;
-   caller cancels deferred call;
-   reply token used twice.

## Phase 6 --- [done] CQ subsystem normalization

Refactor current completion code around:

-   per-shard CQ;
-   operation IDs;
-   optional operation capabilities;
-   non-lossy backlog;
-   atomic wait;
-   cancellation state machine;
-   buffer return;
-   CQ batching.

Migrate `sitas-charlotte` to `CQ_WAIT` and remove busy polling.

## Phase 7 --- [done] Sitas endpoint backend

Add a CharlotteOS userspace runtime layer with two event sources unified
at the executor boundary:

-   CQ completions;
-   endpoint receive readiness/messages.

The kernel may expose a wait set or allow both to signal one shard event
object.

The executor should:

1.  drain CQ;
2.  drain ready endpoints;
3.  wake tasks;
4.  run ready tasks within budget;
5.  wait again.

## Phase 8 --- [done] Userspace UART driver

Use UART as the first complete userspace driver:

-   driver protection domain;
-   MMIO capability;
-   interrupt capability;
-   console endpoint;
-   scalar configuration messages;
-   moved/lent bulk buffer writes;
-   deferred read replies;
-   restart and reset behavior.

This validates authority, IRQ delivery, endpoint IPC, memory messages,
and service supervision without networking complexity.

## Phase 9 --- Virtio-net driver and network service [in progress]

Architecture:

``` text
test application
    → socket/network endpoint
network service with sitas shards
    → packet/control endpoint
virtio-net driver
    → MMIO/virtqueue/IRQ
```

Start with copied packets, then move to registered pools/shared rings.

Milestones:

1.  scalar control path;
2.  copied packet path;
3.  moved packet buffers;
4.  buffer pool;
5.  batched CQ;
6.  per-queue shard placement;
7.  UDP;
8.  TCP after ownership and backpressure are stable.

## Phase 10 --- [done] Kernel concurrency cleanup

After userspace architecture is proven:

-   replace generic IPI closures;
-   introduce reserved/priority kernel RPC capacity;
-   audit `ShardLocal<T>` use;
-   ensure no interrupt path accesses shard-local-only state;
-   add wake coalescing;
-   add LP affinity: threads assigned an affinity LP at first admission,
    re-admitted to the same LP after every wake (§8.6).  This eliminates
    cross-LP migration races and keeps timer events on the correct
    per-LP queue.  A rebalancing skeleton (§14.1) is available but
    disabled pending thread-safety review.
-   add LP migration/hotplug assumptions explicitly.

------------------------------------------------------------------------

## 18. Codex implementation rules

Codex should follow these rules while modifying either repository.

### 18.1 Preserve semantic boundaries

Do not:

-   use completion capabilities as endpoint connections;
-   expose LP IDs as service addresses;
-   pass arbitrary Rust types across the syscall ABI;
-   use raw userspace pointers as durable kernel references;
-   inject userspace callbacks from interrupt context;
-   convert every immediate syscall into an asynchronous operation;
-   silently drop completion records;
-   use unbounded queues.

### 18.2 Prefer explicit state machines

For IPC calls, operations, cancellation, endpoint closure, and memory
lending, implement named states and test transitions.

Avoid behavior that depends on scattered booleans.

### 18.3 Keep unsafe code narrow

Unsafe code should be localized to:

-   trap-frame access;
-   page-table manipulation;
-   userspace memory copy/validation;
-   MMIO;
-   DMA mapping;
-   carefully audited queue/ring primitives.

Protocol decoding, capability checks, and state transitions should
remain safe Rust where possible.

### 18.4 Add adversarial tests

Every capability and memory-transfer feature requires negative tests:

-   wrong domain;
-   stale generation;
-   wrong object type;
-   insufficient rights;
-   double reply;
-   double close;
-   queue full;
-   server death;
-   caller death;
-   cancellation race;
-   malformed message;
-   overlapping memory range;
-   writable alias during mutable lend.

### 18.5 Keep spike APIs visibly experimental

Until stabilized:

-   place experimental syscalls behind an explicit module/feature;
-   document that syscall numbers are unstable;
-   avoid building user libraries around smoke-only raw calls;
-   prefer typed wrappers so ABI changes remain localized.

### 18.6 Update this document with evidence

For each phase, append:

-   commit;
-   branch;
-   tests;
-   architectural findings;
-   rejected alternatives;
-   unresolved risks;
-   decision whether the experiment becomes durable architecture.

------------------------------------------------------------------------

## 19. Suggested repository-level changes

## 19.1 CharlotteOS

Suggested modules:

``` text
crates/catten/src/capability/
    table.rs
    rights.rs
    object.rs

crates/catten/src/ipc/
    endpoint.rs
    connection.rs
    message.rs
    reply.rs
    wait.rs

crates/catten/src/memory/object.rs
crates/catten/src/memory/transfer.rs
crates/catten/src/completion/
    queue.rs
    operation.rs
    cancel.rs

crates/catten/src/device/
    interrupt_object.rs
    mmio_object.rs
    dma_domain.rs
    device_cap.rs

crates/catten/src/service/
    bootstrap.rs
    supervisor.rs
```

Keep architecture-specific interrupt and trap handling under the
existing ISA modules.

## 19.2 Sitas

Suggested separation:

``` text
sitas-core
    executor
    futures
    shard-local
    typed internal mailboxes
    cancellation
    snapshots

sitas-unix
    readiness backend
    io_uring backend
    thread/affinity backend

sitas-charlotte
    CQ backend
    endpoint backend
    syscall wrappers
    memory-object wrappers
    protocol bindings
```

The CharlotteOS backend should not emulate Unix `RawFd` readiness. It
should implement native CQ and endpoint event handling.

## 19.3 Protocol crates

Create small versioned protocol crates:

``` text
charlotte-protocol-log
charlotte-protocol-console
charlotte-protocol-device-manager
charlotte-protocol-network
```

Each crate defines:

-   interface ID;
-   version;
-   opcodes;
-   request/response types;
-   encoding;
-   validation;
-   client wrapper;
-   server decoding helpers.

------------------------------------------------------------------------

## 20. Decision record

The combined architecture adopts the following decisions.

### Adopt

-   isolated userspace services and drivers;
-   capability-bearing endpoint connections;
-   userspace service discovery;
-   scalar and memory message distinction;
-   move/read-lend/write-lend semantics;
-   deferred replies;
-   sitas shard-per-core execution inside service processes;
-   completion queues for kernel/device operations;
-   non-lossy terminal completion delivery;
-   bounded queues and explicit backpressure;
-   no arbitrary userspace upcalls;
-   control-plane IPC plus ring/buffer-based data planes.

### Reject

-   one universal completion capability for all communication;
-   endpoint identity based on LP;
-   arbitrary typed Rust messages as a kernel ABI;
-   one IPI per message;
-   arbitrary cross-LP closures as a durable kernel work model;
-   string names as kernel-granted authority;
-   synchronous-only service loops;
-   unbounded completion or mailbox queues;
-   global smoke-test result pages as a process ABI.

### Defer

-   scheduling-context donation;
-   full priority/deadline propagation;
-   zero-copy shared data planes beyond registered memory objects;
-   live service upgrade;
-   distributed capability revocation;
-   formal verification of IPC state machines;
-   production SMMU/IOMMU support on every platform.

------------------------------------------------------------------------

## 21. Success criteria

The redirection is successful when all of the following are true:

1.  Two EL0 protection domains can communicate through a bounded
    endpoint without knowing each other's ASID or LP.
2.  A client can make a deferred asynchronous call while its shard
    continues running other tasks.
3.  A service can receive a moved memory object, and the sender is
    unable to access it until it is returned or retransferred.
4.  Immutable and mutable lending are enforced by mappings, including
    service-death cleanup.
5.  A sitas shard blocks on one native wait path and wakes for both
    endpoint work and kernel/device completions.
6.  Terminal CQ results survive userspace ring overflow.
7.  Remote shard wakes are coalesced rather than mapped one-for-one to
    IPIs.
8.  A userspace UART driver runs with only delegated MMIO and IRQ
    authority.
9.  Restarting that driver invalidates stale connections and reconciles
    outstanding operations.
10. A virtio-net data path can batch packet buffers without one syscall
    or copied IPC payload per descriptor.
11. All public ABI structures use fixed-width stable representations.
12. Capability misuse and cancellation races have adversarial tests.

Current status:

-   Criterion 1 has smoke-test evidence in `a97d9e4`: two separate EL0
    protection domains communicate through a bounded endpoint using
    delegated local caps only. Phase 3 strengthens this into a
    service-manager flow: the `el0_service` boot test exercises
    bootstrap capability delivery, userspace service naming, restart
    generations, and deterministic stale-connection failure across
    three isolated EL0 domains and a supervised restart.
-   Criterion 5 is met. Its kernel half has self-test evidence: one
    blocking CQ wait is released by kernel completions, explicit
    cross-thread wakes, and CQ-bound endpoint readiness
    (`test_cq_wait_wake`).  The `wait_on_cq` implementation uses a
    per-CQ monotonic work-generation counter (§8.5) decoupled from the
    shared ring's `pending()` count, so services that poll individual
    completions via `poll(cap)` and never drain the ring are not stuck
    in a busy-spin.  At EL0, the reference echo service serves its
    entire protocol — including shutdown and restart — from one `CQ_WAIT`
    with endpoint readiness bound to its default queue, and the sitas
    `ShardExecutor` (sitas repo) now performs task wakeup from the
    drained events: a KV shard blocks in its reactor's single wait and
    is     re-polled by waker-integrated channel sends (`el0_sitas`). Per-shard
    CQ ring mapping is in place: the loader maps `SHARD_CQ_COUNT` (4)
    additional CQ ring pages per domain and opens a kernel queue per shard
    (CqId `i + 1`), and the sitas reactor/waker target the shard's own
    queue so a wake never steals another shard's slot. Criterion 5 is
    fully met and boot-validated.
-   Criterion 9 is met: its connection-invalidation half has smoke-test
    evidence for a generic service (echo) — restarting the service domain
    invalidates stale connections (`EndpointClosed`) and the userspace
    name service reports a new instance generation — and its
    driver-specific half is demonstrated by `test_el0_uart`: an
    uncooperative driver crash is followed by teardown that resets the
    device (unroutes the interrupt, unmaps the MMIO), reconciles the
    outstanding deferred read (`Cancelled` rather than hung), and fails
    stale connections `EndpointClosed`, after which a restarted
    generation-2 instance serves with freshly minted device grants.
-   Criterion 8 is met. Its kernel half has self-test evidence
    (`test_device_capabilities`): a driver domain can be granted an MMIO
    region and an interrupt source as capabilities, map the region into
    its own address space as user Device memory, and be released from one
    `wait_on_cq` by a real GICv3 software-pended SPI routed through the
    live interrupt path. The EL0 half is demonstrated by
    `test_el0_uart`: the reference `uart` userspace driver runs in an
    isolated EL0 domain with only a delegated PL011 MMIO region and its
    interrupt (plus a name-service bootstrap connection), maps the
    register window, serves a console client's writes through the PL011
    registers with direct EL0 MMIO writes, completes an interrupt-driven
    deferred read (§7.2), and acknowledges a delegated device interrupt
    from EL0.

------------------------------------------------------------------------

## 22. References

Primary project references:

-   CharlotteOS fork: <https://github.com/FrodeRanders/charlotte-os>
-   Upstream CharlotteOS: <https://github.com/charlotte-os/charlotte-os>
-   Sitas: <https://github.com/FrodeRanders/sitas>
-   Xous core: <https://github.com/betrusted-io/xous-core>
-   Xous kernel documentation: <https://xous.dev/kernel/>
-   Xous server/API guide:
    <https://github.com/betrusted-io/xous-core/blob/dev/API.md>

Relevant Xous concepts to inspect in source:

-   `SID` and `CID`;
-   server creation and connection;
-   `Scalar` and blocking scalar messages;
-   memory `Send`, `Lend`, and mutable lend messages;
-   name service;
-   message receive and reply;
-   process and server teardown.

------------------------------------------------------------------------

## 23. Working summary for Codex

The architecture is substantially complete through Phase 10. The
implementation order below shows status:
```

Preserve the current completion, AArch64, CQ, and sitas work, but
reinterpret it as one half of the design.

The missing half is Xous-inspired protection-domain IPC.

The final model is built upon three kernel primitives---Capabilities,
Endpoints, and Memory Objects---and the remaining subsystems are layered
on top of them.

The final model is:

``` text
Xous ideas:
    who may talk to whom
    how server IPC works
    how memory crosses protection boundaries

CharlotteOS:
    how authority and isolation are enforced
    how hardware and operations complete
    how threads wait and wake

Sitas:
    how each service uses its CPUs
    how mutable state remains owned
    how asynchronous tasks compose internally
```

That division of responsibility should guide every subsequent change.
