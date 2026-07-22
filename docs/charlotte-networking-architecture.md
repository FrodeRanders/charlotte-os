# CharlotteOS Networking Architecture
## Native Capability-Based Distributed Services

**Status:** Architectural Proposal  
**Audience:** CharlotteOS/Sitas developers and Codex  
**Author:** ChatGPT (based on CharlotteOS design discussions)  
**Purpose:** Define the long-term networking philosophy for CharlotteOS and provide implementation guidance for future development.

---

# 1. Executive Summary

CharlotteOS deliberately departs from the traditional UNIX networking model.

Rather than treating TCP/IP as the foundation of all communication, CharlotteOS treats **networking as an extension of the operating system's message passing and capability model**.

The central idea is simple:

> **Remote IPC should be indistinguishable from local IPC.**

Applications should invoke services through capabilities, not by connecting to IP addresses and ports.

TCP/IP remains an important interoperability layer, but it is no longer the native programming model.

Instead, the networking architecture is built around:

- Ethernet frame transport
- reliable message passing
- capability-based service invocation
- distributed object references
- transparent service discovery

This architecture aligns naturally with the existing CharlotteOS goals:

- message passing
- capability-based security
- zero-copy communication
- ownership transfer of memory
- transport independence
- distributed systems

---

# 2. Motivation

Most operating systems inherit their networking model from UNIX.

Applications typically perform:

```text
socket()
connect()
send()
recv()
close()
```

Every network interaction is fundamentally based upon:

- IP addresses
- ports
- byte streams
- connection-oriented communication

This model has served the Internet extremely well.

However, it is not necessarily the best model for:

- microkernels
- distributed operating systems
- capability-based security
- enterprise service architectures
- message-oriented systems

CharlotteOS is not attempting to replace TCP/IP on the Internet.

Instead, it asks a different question:

> **If we designed a distributed operating system today, would sockets still be our fundamental abstraction?**

The answer proposed here is **no**.

---

# 3. Design Goals

CharlotteOS networking should satisfy the following principles.

## 3.1 Unified IPC

Local communication and remote communication should use the same programming model.

Applications should not care whether a service resides:

- inside the same process
- inside another process
- on another machine

---

## 3.2 Capability-Oriented

Services are identified by capabilities rather than addresses.

Instead of

```text
10.2.3.17:443
```

applications receive

```text
Capability<PayrollService>
```

---

## 3.3 Message-Oriented

The native abstraction is a message.

Not a byte stream.

Messages have:

- identity
- ownership
- metadata
- boundaries

---

## 3.4 Transport Independence

Applications should not depend upon:

- VirtIO
- Ethernet
- WiFi
- RDMA
- TCP/IP

The transport is an implementation detail.

---

## 3.5 Zero-Copy

Whenever practical:

- pages are transferred
- ownership changes
- copying is avoided

---

## 3.6 Compatibility

TCP/IP remains fully supported.

It is simply not the native API.

---

# 4. Historical Perspective

The Internet won.

That does **not** mean TCP/IP was the only interesting architecture.

Several operating systems and network architectures explored very different ideas.

## Xerox XNS

Introduced:

- service discovery
- RPC
- distributed naming

Many later systems borrowed heavily from XNS.

---

## DECnet

Focused on:

- remote processes
- cluster communication
- distributed services

rather than sockets.

---

## AppleTalk

Applications discovered services rather than hosts.

Examples:

```
LaserWriter
Payroll
Accounting
```

instead of

```
192.168.1.42
```

---

## Banyan VINES

Introduced directory-oriented networking where services were primary citizens.

---

## Amoeba

Perhaps the closest historical relative.

Everything was based upon:

- capabilities
- RPC
- distributed objects

Networking transported RPC messages rather than streams.

---

## Plan 9

Introduced:

- 9P protocol
- distributed namespaces

Networking became another filesystem.

---

## Inferno

Extended Plan 9 ideas.

Everything communicated using Styx (9P).

---

## Barrelfish

Built around message passing.

Networking was viewed as another transport.

---

These systems demonstrate an important lesson:

> **Networking need not revolve around sockets.**

---

# 5. Ethernet is Not TCP/IP

Ethernet provides:

- addressing (MAC)
- frame transport
- multicast
- MTU
- CRC

Nothing more.

```
VirtIO
    │
Ethernet Frames
```

IP is simply one protocol that runs on Ethernet.

Others include:

- ARP
- LLDP
- AppleTalk
- IPX
- EtherCAT
- custom protocols

CharlotteOS therefore treats Ethernet as a **generic frame transport**, not as "IP networking."

---

# 6. Proposed Layering

The native CharlotteOS networking stack becomes:

```
Applications
        │
Capability Invocation
        │
Distributed Objects
        │
RPC
        │
Reliable Message Layer
        │
Ethernet Frame Transport
        │
VirtIO / NIC Driver
```

Each layer has a single responsibility.

---

## Ethernet Layer

Responsible for:

- transmitting frames
- receiving frames
- multicast
- MTU handling

Knows nothing about:

- IP
- TCP
- RPC
- services

---

## Reliable Message Layer

Provides:

- sequencing
- acknowledgements
- retransmission
- fragmentation
- congestion control
- batching

This layer exports **messages**, not streams.

---

## RPC Layer

Provides:

- request/reply
- asynchronous invocation
- object references
- serialization

---

## Distributed Objects

Transforms RPC into object invocation.

Applications simply invoke capabilities.

---

# 7. Native Programming Model

Traditional networking:

```text
socket()
connect()
send()
recv()
```

CharlotteOS:

```rust
let payroll = lookup("Payroll");

payroll.call(
    "calculate_salary",
    employee
);
```

The runtime handles:

- routing
- fragmentation
- retries
- encryption
- authentication

Applications never manipulate sockets.

---

# 8. Service Discovery

Traditional systems:

```
DNS
  │
Host
  │
Port
```

CharlotteOS:

```
Capability Directory
        │
Service Name
        │
Capability
```

Applications locate:

```
Payroll
Identity
Storage
Logging
Notification
```

rather than machines.

---

# 9. Message-Oriented Networking

TCP provides byte streams.

CharlotteOS instead uses discrete messages.

Advantages:

- natural IPC integration
- explicit ownership
- batching
- zero-copy
- simpler scheduling

Example:

```
Message
├── Header
├── Metadata
├── Payload
└── Capabilities
```

Messages become first-class operating system objects.

---

# 10. Reliability Without Streams

Reliable communication does **not** require TCP.

Instead:

- acknowledgements
- sequencing
- retransmission
- flow control
- congestion control

can all exist independently.

The abstraction exported remains:

```
Reliable Message
```

rather than

```
Infinite Byte Stream
```

---

# 11. Capability-Oriented Security

Traditional networking authorizes using:

- IP addresses
- ports
- TLS certificates
- ACLs

CharlotteOS instead authorizes capabilities.

Example:

```
Capability<Payroll>
```

contains authority.

Applications cannot fabricate capabilities.

Advantages:

- least privilege
- natural sandboxing
- no ambient authority

---

# 12. VirtIO Integration

VirtIO is merely one implementation.

```
QEMU
   │
VirtIO
   │
Ethernet Driver
   │
Frame Transport
```

The remainder of the networking stack remains unchanged.

Future drivers:

- Intel E1000
- Realtek
- USB Ethernet
- RDMA
- loopback

all expose the same interface.

---

# 13. Transport Independence

The architecture intentionally separates:

```
Transport
```

from

```
Communication Model
```

Therefore the message layer can run over:

- Ethernet
- shared memory
- loopback
- RDMA
- PCIe
- InfiniBand
- future transports

without affecting applications.

---

# 14. Compatibility Layer

TCP/IP remains available.

It simply becomes another operating system service.

```
              TCP/IP Service
                     │
         ┌───────────┴───────────┐
         │ Capability RPC Layer  │
         └───────────┬───────────┘
                     │
          Reliable Message Layer
                     │
            Ethernet Transport
```

Legacy software uses TCP/IP.

Native software does not.

---

# 15. Relationship to IPC

The networking model is intentionally identical to local IPC.

```
Application
      │
Capability Invocation
      │
IPC
      │
─────────────┬──────────────
             │
     Local Destination
             │
     Shared Memory IPC

             or

     Remote Destination
             │
     Reliable Messages
             │
        Ethernet
```

From the application's perspective:

there is no difference.

This is one of the primary architectural goals of CharlotteOS.

---

# 16. Zero-Copy Communication

CharlotteOS already explores page ownership transfer.

Networking naturally extends this concept.

Receive path:

```
VirtIO RX Buffer
        │
Driver
        │
Ownership Transfer
        │
Message Layer
        │
Application
```

Once processing completes:

```
Application
      │
Return Ownership
      │
Driver RX Pool
```

No copying is required.

---

# 17. Future Extensions

The same message layer can support:

- publish/subscribe
- distributed actors
- replicated services
- distributed scheduling
- cluster messaging
- distributed object references

without modifying the underlying transport.

---

# 18. Raft Consensus as a Capability Service

The reliable message layer, RPC layer, and capability-based service
invocation described above are the building blocks of a **generic,
transport-agnostic Raft consensus service** — a replicated state
machine exported as a native CharlotteOS IPC endpoint.

> **Implementation status.** The Raft core, transport traits, wire
> format, and service binary all compile and load as EL0 services.
> A two-node cluster in one CharlotteOS instance repeatedly elects one
> leader over local endpoint IPC on four-LP QEMU/HVF. Election timers use
> completion events, and verifier/test domains terminate rather than polling
> forever; the settled system reaches the idle/WFI path. Cross-machine leader
> election remains blocked on two dependencies: (1) the NIC driver and TCP/IP
> service need runtime validation on Linux KVM because HVF cannot safely
> exercise the required EL0 MMIO path, and (2) the reliable-message service
> needs end-to-end implementation and validation over that driver.
> Commit `2679085` established local two-node IPC election; later scheduler,
> timer, and lifecycle fixes preserve that result under the current HVF boot.

| Capability | Status |
|---|---|
| Raft core (leader election, log replication, commit/apply) | Two-node local election boot-validated; broader replication/state-machine coverage remains incomplete |
| Charlotte IPC transport (`CharlotteTransport`) | Local peer-connection table and endpoint transport boot-validated |
| Election timer (`submit_timer`) | Implemented (SVC #1, OpCode::Timer, CNTV hardware) |
| Scalar RPCs over endpoint IPC | Local VoteRequest/AppendEntries path boot-validated via endpoint calls |
| Memory-object RPC payloads (zero-copy log replication) | Wire format defined; transport uses scalar calls only |
| Durable log (survives service restart) | In-memory `LogStore` exists; page-backed not yet implemented |
| Cross-machine Raft (NIC driver + reliable message layer) | Blocked on NIC runtime validation (KVM) |
| Distributed name service (on top of Raft) | Design only; depends on cross-machine Raft |

```
┌──────────────────────────────────────────────┐
│          Distributed Name Service             │  ← replicated capability→name store
│  (a StateMachine on top of the Raft service)  │
└──────────────────┬───────────────────────────┘
                   │ IPC (OP_CLIENT_COMMAND)
┌──────────────────▼───────────────────────────┐
│          Raft Consensus Service               │  ← one EL0 service per node
│  endpoint: "RAFT"  opcodes: 1=Vote 2=Append  │
│           3=Snapshot  4=Command  8=Status     │
└──────────────────┬───────────────────────────┘
                   │ IPC / memory objects
┌──────────────────▼───────────────────────────┐
│     Reliable Message Layer / NIC driver       │
└──────────────────────────────────────────────┘
```

## 18.1 Architectural fit

| Raft concern | CharlotteOS primitive | Status |
|---|---|---|
| Peer discovery (single-instance) | Name service (`OP_LOOKUP`) | Works: nodes register as `raft-{id}`, look up local peers |
| Peer discovery (cross-machine) | Distributed name service (on top of Raft) | Requires cross-machine Raft first |
| Inter-node RPC (scalar) | Endpoint IPC (`ipc_scalar_call`) | Works: VoteRequest, AppendEntries travel as scalar messages |
| Inter-node RPC (zero-copy) | Memory objects (`Move` transfer) | Wire format defined; transport currently scalar-only |
| Election timer | `submit_timer()` (SVC #1, OpCode::Timer) | Implemented: CNTV hardware → CQ completion |
| Durable log | Memory objects | In-memory `LogStore` implemented; page-backed planned |
| Client command submission | Capability-based endpoint call | `OP_CLIENT_COMMAND` opcode defined; not yet wired to state machine |
| Linearizable reads | Reply tokens + read barrier | Supported by `RaftNode` logic; untested end-to-end |

## 18.2 Why this is better than socket-based Raft

**Authority, not addresses — after bootstrap.** Once the cluster is
running, peers are identified by name service entries, not IP:port.
A node that has joined the cluster receives connection caps for its
peers through the name service. No IP configuration and no DNS are
needed for ongoing operation.

Bootstrap, however, remains a seed problem: a new node must contact
at least one cluster member to discover the rest. The seed is a
service name (e.g. `raft-cluster.node-1`) rather than an IP address,
but it still must be supplied — through the launch manifest, a
configuration page, or a hardware-level discovery protocol.
Section 18.4 discusses this in detail.

**Transport transparency.** The transport layer is an implementation
detail behind the `RaftTransport` trait. A connection capability routes
through local IPC, Ethernet frames, or a future RDMA transport — the
consensus core never changes.

**Zero-copy replication (planned).** Log payloads can transfer via `Move`
memory objects: the kernel flips page-table ownership instead of
copying bytes. The wire format supports this; the current transport
uses scalar IPC with inline payloads. Moving to memory-object RPCs
is a transport-layer change that does not affect the consensus core.

**Capability-based security.** A client invokes the Raft service only if
it possesses a connection cap. The service manager attenuates rights:
`CALL` for `OP_CLIENT_COMMAND` but not `OP_ADD_SERVER`. No ambient
authority, no IP-based ACLs.

**Failure isolation (partly implemented).** Each local Raft node runs in a
separate protection domain, and generic service/UART lifecycle tests validate
generation changes and stale-connection failure. Automatic Raft-node restart,
durable state recovery, and continued quorum operation after a node crash have
not yet been validated. Those remain requirements for the cross-machine
service rather than current evidence.

## 18.3 Distributed name service (design)

> **Status: design only.** The existing name service (`ns.rs`) is
> single-node. Making it distributed requires (1) cross-machine Raft
> validated on KVM, (2) the reliable message layer implemented, and
> (3) the name service refactored as a `StateMachine` implementation.

The existing single-node name service becomes a replicated
service by running as a `StateMachine` on top of the Raft service:

- `OP_REGISTER` / `OP_LOOKUP` — submitted as commands through the Raft
  leader, replicated to the log, applied at each node's state machine
- Service generations — replicated consistently; a restart on node 3
  cannot create a split-brain where node 1 sees generation 5 and node 2
  sees generation 3
- Access keys (`OP_REGISTER_KEYED`) — enforced consistently across the
  cluster; revocation propagates through the replicated log

The Raft service itself remains **generic**: it replicates opaque
commands against a pluggable `StateMachine` trait. A distributed name
service, a distributed lock service, and a cluster configuration store
are different state machines on the same Raft substrate — none of them
know about sockets, IP addresses, or TCP.

## 18.4 Cluster bootstrap and seed discovery

Like every consensus system, Charlotte OS Raft faces a bootstrap
problem: a node starting with an empty peer list must contact at least
one existing cluster member. The difference is that the seed is
expressed in terms the OS already understands — service names and
transport-layer identifiers — rather than IP addresses.

### Static seeds (launch-time configuration)

The simplest mechanism, already implemented in the Raft service
binary: named launch-manifest records carry the node and seed-peer service names.

```
node-id = "r4"
peer-id = "r1"  // repeated keys form the seed list
peer-id = "r2"
peer-id = "r3"
```

The node calls `OP_LOOKUP("raft-r1")` on its local name service to
obtain a connection cap. If the seed node resides on the same machine,
the name service returns the cap directly. If the seed is remote, the
local name service forwards the request through the distributed name
service (once available), which resolves it to a remote connection
routed through the reliable message layer and NIC driver.

This is functionally equivalent to `--peers=10.0.0.1:7000` in a
traditional Raft cluster. The difference is that the seed identifier
is a service name, not a transport address, so the transport layer can
change (Ethernet → RDMA → shared memory) without invalidating the seed
configuration.

### Pre-shared cluster capability

A cluster administrator or provisioning system creates a "cluster
capability" — an endpoint connection cap to a known cluster member —
and delivers it to new nodes as a bootstrap argument. This eliminates
even the service-name lookup step: the node already possesses a
first-class connection to a peer. The supervisor writes this cap into
the node's config page at launch, alongside the name service
connection.

### Hardware-level discovery

On a single Ethernet segment, nodes can discover each other without
any seed configuration. A joining node broadcasts an Ethernet frame
with a well-known EtherType (the `charlotte-protocol-msg` layer
already reserves `0x88B5`), carrying a "Raft peer discovery" payload.
Existing cluster members respond with their service names and a
connection offer. The joining node then performs a standard name
service lookup to obtain attested connection caps.

This is analogous to ARP or IPv6 Neighbor Discovery, but at the
cluster-membership level rather than the address-resolution level.
It works without any seed configuration when all nodes share a
broadcast domain, and it degrades gracefully to static seeds when
they do not (routed networks, overlays).

### Gradual deployment path

These mechanisms are ordered by implementation complexity:

1. **Static seeds in launch args** — works today, requires manual
   configuration per node. The Raft binary already accepts peer names
   as argument words.
2. **Pre-shared cluster capability** — requires the service manager
   or provisioning system to mint and deliver a capability. Small
   kernel change, large operational improvement.
3. **Ethernet broadcast discovery** — requires the reliable message
   layer to handle broadcast frames and a discovery protocol handler
   in the Raft binary. Zero-configuration on a single segment.
4. **Distributed name service** — once the cluster is running and
   the name service is replicated through Raft, new nodes need only
   a single seed to discover all peers through the consistent name
   registry. This is the end state.

---

# 19. Comparison

| Traditional UNIX | CharlotteOS |
|-----------------|-------------|
| socket | capability |
| connect | lookup |
| IP address | service |
| port | capability |
| TCP stream | message |
| DNS | capability directory |
| host-centric | service-centric |
| network API | IPC API |
| TCP/IP foundation | message foundation |
| TCP/IP everywhere | TCP/IP compatibility layer |

---

# 20. Guidance for Codex

The following rules should guide future implementation.

## Architecture

- Do not introduce socket APIs into native CharlotteOS components.
- Networking shall extend the IPC model.
- Applications shall invoke capabilities rather than addresses.

## Transport

- Ethernet drivers expose only Ethernet frames.
- Drivers shall have no knowledge of IP or TCP.
- Drivers shall support ownership transfer of packet buffers whenever possible.

## Message Layer

- Export reliable messages rather than streams.
- Preserve message boundaries.
- Support zero-copy ownership transfer.
- Support batching.

## RPC

- RPC shall build upon the message layer.
- Services shall be capability-based.
- Discovery shall be service-oriented rather than host-oriented.

## Compatibility

- TCP/IP shall be implemented as an operating system service.
- Native CharlotteOS components shall not depend upon sockets.
- Existing POSIX software shall continue to function through the compatibility subsystem.

## Long-Term Vision

The networking subsystem shall disappear as a distinct programming model.

Applications should simply communicate with capabilities.

Whether the recipient is:

- another thread,
- another process,
- another machine,

shall be transparent.

The operating system—not the application—determines how communication is transported.

## Replicated Services

Distributed consensus shall be available as a generic capability service.
Raft, built on the reliable message layer, exports a replicated state-machine
abstraction through the same endpoint IPC that local services use.

- The Raft service shall be transport-agnostic: its peer communication uses
  the `RaftTransport` trait, backed by endpoint IPC. Memory-object transfer
  (`Move` semantics) shall replace scalar payloads for log replication once
  the reliable message layer supports frame-sized limits.
- Cluster membership shall be managed through the name service once a node
  has joined. Initial bootstrap requires seed configuration (launch args,
  a pre-shared cluster capability, or Ethernet broadcast discovery — see §18.4).
- The service is generic: a distributed name service, lock service, or
  configuration store are different `StateMachine` implementations on the
  same Raft substrate.
- Timer-driven operations (election timeouts, heartbeat intervals) shall
  use the first-class `submit_timer()` completion primitive, not polling loops.
- Existing single-node services that need consistency across the cluster
  shall be refactored as state machines behind the Raft service rather than
  re-implementing consensus internally.

---

# 21. Architectural Principle

The networking architecture of CharlotteOS can be summarized by a single statement:

> **Remote IPC is local IPC with a different transport.**

Everything else follows from this principle.

TCP/IP remains a valuable interoperability technology.

It simply ceases to define the native architecture of the operating system.

The operating system is fundamentally message-oriented, capability-oriented, and transport-independent.
