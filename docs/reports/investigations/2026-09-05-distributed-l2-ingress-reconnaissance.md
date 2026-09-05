# Distributed L2 ingress reconnaissance

Date: 2026-09-05

This note maps the existing CharlotteOS implementation onto the first
distributed L2 ingress design. It records the source-level constraints that
the implementation must preserve.

## Existing boundaries

| Concern | Existing CharlotteOS mechanism | Ingress use |
|---|---|---|
| Ethernet RX | `frouter` exclusively owns the NIC driver's deferred `net::OP_RECV` | Classify VIP frames here, before `tcpip` and smoltcp |
| Ethernet TX | NIC `net::OP_SEND` accepts a moved memory object and is safe for multiple producers | Move remotely selected frames directly from `frouter` to the NIC |
| smoltcp adapter | `CharlotteEthDevice` receives frames queued by `tcpip::OP_FRAME` and transmits through `net::OP_SEND` | Local selections follow the existing path unchanged |
| ARP | smoltcp answers for addresses installed on its interface | Deliver VIP ARP only on the selected advertiser; accepting a VIP and advertising it remain separate decisions |
| IP ownership | `tcpip` installs DHCP/static addresses in the smoltcp interface | Install the configured VIP as an additional local address on every backend |
| TCP dispatch | One `tcpip` domain owns its smoltcp `SocketSet`; applications use the capability-oriented socket API | The selected backend, never the ingress node, owns TCP state |
| Node identity | Persisted `{cluster}:{32-bit token}` identity; the token is MAC-derived on first boot | Use the stable token as the deterministic backend ID, never an ASID, pointer, or object ID |
| Discovery | `disco` maps observed node identities to Ethernet MACs and expires silent peers | Supplies reachability only; discovery alone never grants backend eligibility |
| Cluster membership | DNS owns the durable `RaftNode`; `ClusterConfiguration` records current/joint voter sets | Supplies authoritative eligibility after committed membership transitions |
| Load-balancing epoch | Configuration commands have committed Raft log indices; signed shutdown intents are replicated catalog state | Fingerprint the configuration index plus sorted drain generations |
| Failure detection | Discovery expiry removes a MAC route; Raft membership changes are explicit committed transitions | An unreachable admitted member is not selected locally; permanent removal remains a Raft operation |
| Placement | Replicated deployment records name a node key and generation | The first backend set is cluster-wide, but the selector accepts an explicit per-service set so placement can narrow it later |
| Application movement | The deployment agent retires and launches generation-fenced application domains | A reconnect can reach recovered application state on a new node |
| State serialization | Application-defined state can be serialized/deserialized during movement | This does not include TCP sequence, retransmit, window, congestion, or socket state |
| Service discovery | Local name service plus Raft-replicated DNS catalog | Cluster TCP services are explicit launch policy in the first slice; they are not inferred from arbitrary listening sockets |
| Inter-node frame route | The NIC can send a moved Ethernet frame directly to a discovered peer MAC | Add/remove a one-hop Ethernet envelope in the same moved memory object; preserve IP, TCP, and payload bytes |

## Chosen first-slice design

`ClusterService` separates the externally visible VIP/protocol/port from both
the ingress participant and execution node. A locally materialized
`BackendSnapshot` contains committed, active Raft voters for which a
discovery-associated route is available and a separate subset accepting new
flows. During joint consensus the admitted set is the voter intersection of
the old and new configurations:
joiners become eligible only after finalization, while departing nodes stop
receiving new flows as soon as removal is committed.

An operator-signed shutdown intent supplies the replicated drain authority.
Once committed, the target remains an admitted/routable member for established
flows but is removed from the new-flow subset. This connects ingress draining
to the role-separated shutdown path rather than adding an unauthenticated
frame-router control opcode.

The DNS/Raft owner publishes this snapshot over local IPC. `frouter` refreshes
it asynchronously and never calls Raft on the packet path. Rendezvous hashing
maps `(service, five-tuple, epoch)` to a stable node key. A bounded local flow
cache pins observed flows to the snapshot used for their initial SYN and
retains a small number of earlier snapshots. SYN retransmissions reuse the
same entry; FIN/RST releases it. If a replacement ingress has no cache entry,
it still selects the same backend while the policy epoch is unchanged.

For a local selection, `frouter` moves the unchanged frame to the existing
`tcpip::OP_FRAME` route. For a remote selection it adds an eight-byte
Charlotte forwarding envelope in the received memory object and moves the
same capability to `net::OP_SEND`. The envelope preserves the external source
MAC and EtherType, and prevents a receiving router from classifying the packet
twice while membership snapshots converge. The backend validates the outer
source against committed member routes, removes the envelope in place, and
delivers the original network packet locally. The IP destination remains the
VIP throughout.

Operational Raft RPCs use a dedicated direct Ethernet route so election and
heartbeat traffic cannot queue behind reliable-message application traffic.
Join/control messages remain on `relmsg`. Joining also requires an explicit
consensus-domain reset: a durable fence precedes removal of the singleton log,
application state and queued RPCs. This was necessary because independent
term-1 singleton histories otherwise satisfy Raft's same-index/same-term match
test despite containing different commands. Append retries crossing a compacted
snapshot require a second guard: the compacted prefix is skipped at the
snapshot anchor instead of being duplicated after its retained suffix.

Every backend installs the VIP in smoltcp so it can accept the packet and emit
a direct `VIP -> client` response. Only the deterministic VIP advertiser is
allowed to pass external ARP requests for the VIP into smoltcp. Advertisement
ownership uses the existing Raft election and is independent of flow
selection. The current Raft leader is the advertiser only when it is in the
committed eligible set. If that leader is draining, the lowest eligible stable
node key advertises until Raft leadership or membership moves. A newly selected
local advertiser emits a gratuitous ARP.

## Deliberate limits

- IPv4/TCP is the first protocol slice. The types do not equate VIP ownership
  with ARP, leaving IPv6 Neighbor Discovery as a later implementation.
- Backend failure can end its connections. Reconnect is the recovery boundary.
- Flow-cache state is bounded and ingress-local. It protects flows observed by
  that ingress across membership changes; failover without shared state is
  guaranteed only while the relevant policy epoch remains available and
  deterministically reconstructible.
- Application serialization does not imply transparent TCP migration.
- Signed whole-node shutdown provides draining for new ingress flows, but a
  service-only drain API and automatic failed-member removal remain future
  controller work.
- The initial cluster-service declaration is trusted platform launch policy.
  A later deployment controller can replicate service-specific backend and VIP
  policy without changing the packet classifier or selector.
- The forwarding marker authenticates no bytes. Source-MAC admission checks
  inherit Charlotte's current trusted-L2 assumption; a hostile peer capable of
  spoofing an admitted MAC requires switch isolation or a future authenticated
  cluster envelope.
- The three-guest acceptance fixture passed with three established backend
  flows, one surviving flow after killing the leader/VIP owner, and a fresh
  HTTP connection to a live backend. The replacement election can take
  several split-vote rounds, and failed voters are not automatically removed
  from committed membership.
