# Distributed L2 ingress and cluster-wide TCP services

Charlotte can expose a TCP service as one IPv4 `VIP:port` while letting the
admitted cluster members own different connections. The ingress path does not
terminate TCP. It selects a backend and, when that backend is remote, wraps the
unchanged IP packet in a compact one-hop Charlotte Ethernet envelope before
returning the same moved memory object to the NIC driver. The selected
backend's `smoltcp` instance therefore owns the sole TCP state and replies
directly to the client.

```text
client -> VIP advertiser/frouter -- one-hop L2 envelope --> backend tcpip
client <----------------------- direct VIP reply -------- backend tcpip
```

## A cluster facade, not only a load balancer

Charlotte supports two complementary network identities. A node's DHCP or
static address deliberately identifies that node and remains useful for
node-specific client/server protocols, diagnostics, and infrastructure
integration. A cluster service VIP instead identifies a service placed within
Charlotte without revealing which node admits a packet or owns the resulting
connection.

Direct Server Return is commonly described as a load-balancing technique, and
this implementation does distribute five-tuples across eligible nodes. Its
more important architectural effect for Charlotte is indirection: the external
contract is `VIP:port`, while ingress ownership and execution placement may
move independently behind it. Individual node addresses remain mechanisms of
the cluster, not part of the cluster-addressed application's public identity.

The service declaration may now bind the VIP to a deployed application name.
DNS then intersects admitted members with the application's committed placement
and exact-generation readiness registration. A moved or replaced application
does not remain in the newly derived policy through a stale registration:
eligibility becomes empty after the new placement commits and reappears only
when the target node publishes the matching deployment generation. Routers
learn that policy asynchronously, so this statement describes the committed
projection, not instantaneous convergence at every packet path. Omitting the
application name retains the original platform-service mode in which every
admitted, non-draining member is a backend.

![Cluster management driving DSR eligibility and packet delivery](../manual-v2/figures/cluster-management-dsr.svg)

The diagram separates reconciliation from forwarding. Signed operations,
membership, placement, and readiness change a replicated eligibility epoch;
the frame router consumes an immutable projection of that state without making
a Raft call for each packet. Consequently, cluster management decides who may
receive new connections, while DSR decides which eligible replica owns a
particular five-tuple and leaves TCP state at that replica.

## Identities and authority

The service identity is IPv4 address, IP protocol and port. The ingress
identity is whichever committed node currently advertises the VIP. The
execution identity is the backend selected for a five-tuple. They need not be
the same node.

Only the platform launcher can place `vip`, `vipport`, and the optional
`vip-name` deployment binding in service manifests. Applications receive
socket capabilities; they cannot alter ingress policy, claim readiness for a
different deployment generation, or change cluster membership.

DNS owns the operational Raft member. Its local `OP_INGRESS_MEMBERSHIP`
operation materializes an immutable snapshot containing stable node keys and
the discovery-associated MAC route for every admitted voter. The snapshot
separates that trusted/routable member set from the subset eligible for new
flows. When `vip-name` is configured, that subset contains only nodes selected
by the committed replica set and carrying an active per-node catalog
registration for the exact deployment generation. It therefore yields any
subset from zero through the desired replica count as agents become ready.
DNS returns no snapshot unless every committed member has a route.
Discovery therefore supplies reachability but cannot admit a backend. During
joint consensus the admitted set is the intersection of the old and new voter
sets: a joiner enters only after finalization, while a departing node stops
receiving new work as soon as the joint change commits.

A committed, operator-signed node-shutdown intent attenuates ingress authority
before local teardown: its target remains in the admitted member set, so
existing bindings and authenticated one-hop delivery can continue during the
grace interval, but is removed from the new-flow backend set. If the Raft
leader is draining, the lowest stable eligible node key becomes the temporary
VIP advertiser until membership or leadership catches up. No unsigned local
knob can put a node into or take it out of this state.

## Packet path

`frouter` remains the single owner of `net::OP_RECV`. It polls DNS membership
asynchronously and never calls Raft on the packet path. It extracts protocol,
source and destination IPv4 addresses, and source and destination ports in
place. A fixed deterministic rendezvous hash scores that key against each
stable node key. Input ordering and Rust process-local hash randomization
cannot affect the winner.

If the winner is local, the moved frame follows the existing
`socket::OP_FRAME` path. If remote, `frouter` maps the same memory object
writable, shifts the network packet by eight bytes, and emits EtherType
`0x88b8` with the ingress MAC as source and the backend MAC as destination.
The envelope retains the external source MAC and original EtherType. The
backend accepts that envelope only from a MAC in its current committed member
snapshot, removes it in place, restores the original Ethernet fields, and
delivers the frame directly to its local protocol route. IP, TCP, TCP options,
sequence numbers and payload bytes never change. Every participating node's
TCP/IP service installs the VIP as a `/32` address, separately from its DHCP or
static node address; DSR policy nevertheless forwards new flows only to the
ready backend subset. The selected backend accepts the packet and replies with
the VIP as IP source.

Raft's Vote, AppendEntries and InstallSnapshot traffic uses the separate
private EtherType `0x88b7`. Keeping consensus heartbeats and election votes out
of the reliable-message service prevents application traffic from blocking
cluster liveness. Admission handshakes and DNS application/control messages
continue over `relmsg`. A durable join fence resets standalone log,
state-machine and queued transport state before an anchor's history is
accepted. Append retries that overlap a compacted snapshot are normalized at
the snapshot index, so an old prefix cannot be appended after a retained
suffix.

## Epochs and failure semantics

The load-balancing epoch is a deterministic fingerprint of the committed Raft
configuration index, service name, deployment generation, service-registration
generation, sorted ready-node set, and replicated shutdown-intent generations.
It changes for membership, placement, readiness, or drain-policy changes, but
not for unrelated catalog traffic.
`frouter` retains four membership snapshots and up to 1,024 local
`FlowKey -> epoch` bindings. Retransmitted SYNs and later packets retain the
original epoch. Adding a member consequently affects new flows without
remapping observed connections. When a backend is removed, bindings that
selected it are released so a reconnect can use the active set; bindings
owned by surviving nodes retain their older epoch. A draining backend remains
routable and therefore keeps its observed bindings; new SYNs exclude it.

Those properties hold while the required snapshot remains in the bounded
history and after the router has installed the relevant committed projection.
A successful complete refresh grants a five-second monotonic lease for VIP
advertisement and unbound-flow admission. The normal one-second refresh renews
it. DNS supplies a refresh only when its Raft state has current cluster
evidence: a leader must hold recent quorum contact, while a follower must have
successfully matched and applied a recognized leader's log within the previous
second. A rejected AppendEntries heartbeat does not refresh that evidence. A
failed, incomplete, or source-stale refresh may leave the last complete
snapshot in memory for established flows, but lease expiry suppresses ARP
responses and drops every packet without an existing binding. The two-stage
rule bounds authority after cluster contact is lost: the source witness ages
out within one second and the last router lease within a further five seconds.

Snapshot-history eviction remains independent of the flow-table bound. A live
binding whose epoch leaves history is retained as a fail-closed tombstone;
classification drops its packets instead of falling back to the current
snapshot. FIN or RST may retire the binding, and ordinary bounded flow-table
pressure may still evict old state. That latter loss remains an explicit
availability limit: without distributed connection tracking, an ingress node
cannot always distinguish traffic first observed after advertiser failover from
traffic whose local binding was evicted.

This cache is deliberately not distributed connection tracking. Another
ingress participant with the same epoch independently selects the same backend,
so failure of the VIP advertiser alone does not destroy backend TCP state.
Bindings can be lost through bounded eviction or simultaneous membership
change and ingress failure; that is an explicit first-version limitation.
Failure of the selected backend may terminate its TCP connections.

[`CharlotteClusterIngress.tla`](../tla/CharlotteClusterIngress.tla) composes
membership, placement, exact-generation readiness, drain, router snapshots and
flow epochs. Its safe specification now matches the implemented lease and
fail-closed tombstone contract. Negative configurations retain the former
stale-new-flow admission and history-fallback remapping alongside the
stale-readiness regression.

VIP advertisement follows the leader elected by the existing Raft group when
that identity is an admitted, non-draining ingress participant. It need not be
an application backend: the ingress node can forward a flow to whichever node
the placement/readiness set permits. No node advertises the VIP while the
ready-backend set is empty. Other nodes drop VIP ARP requests, including while
no leader or complete snapshot is known. A new advertiser transmits a
gratuitous ARP reply. Loss of the ingress owner can therefore move advertisement
after an ordinary Raft election without changing the backend set or introducing
a second consensus system.

The forwarding envelope is an isolation marker, not cryptographic link
authentication. The receive path checks its source against committed member
routes, which prevents an unadmitted honest peer from becoming a backend, but
a hostile machine able to spoof an admitted MAC on the same L2 segment could
forge it. This first version therefore assumes the cluster-facing L2 is a
trusted or administratively isolated fabric. Authenticated link envelopes,
switch port controls, or a protected overlay are required before exposing that
segment to mutually untrusted hosts.

## Bounds, diagnostics and validation

The initial implementation supports one launch-configured IPv4/TCP service and
at most 64 admitted members. Signed node shutdown supplies the first graceful
drain trigger; a standalone service-drain operation and automatic failed-member
removal are not implemented. Service-specific replica sets are implemented;
IPv6 neighbour advertisement, multiple VIPs and transparent TCP state
migration remain extension points. Application state restoration can
support reconnect-and-resume semantics, but application serialization does not
include TCP sequence, retransmission or congestion-control state.

The frame-router status reply and shared status page expose the current epoch,
admitted-member, eligible-backend and derived draining counts, advertiser node
key, local/remote/drop counters and retained flow-binding count. Unit tests
cover deterministic selection, distribution, join, drain and removal
behaviour, ingress replacement, exact packet preservation and ARP construction.
The AArch64 SLIRP demonstrator serves the HTTP keyhole
through `10.0.2.42:80`, proving VIP ARP, local selection and direct smoltcp
delivery.

`scripts/run-distributed-ingress-test.sh` builds a three-guest stream-LAN plus
an independent host-side Ethernet/TCP participant. The fixture waits for one
stable three-voter configuration, establishes flows selected across all three
backends, kills the Raft leader/VIP advertiser, observes replacement
advertisement and gratuitous ARP, and issues HTTP on the already established
connections. It then opens fresh five-tuples and requires a complete HTTP
exchange with a surviving backend, covering reconnect after loss of the
original advertiser and one connection owner. A failed voter remains eligible
until an explicit committed membership change removes it, so the probe uses a
bounded family of new flows: some may still select the failed node, while at
least one must reach a live member. The observed two-survivor election can need
multiple split-vote rounds, so faster failover remains a tuning and pre-vote
work item rather than a claimed property.

For an operational AArch64 launch, pass the same descriptor to every member:

```sh
./scripts/run-aarch64.sh release --cluster-service 10.0.2.42:80
```

This option configures the runtime service; it does not register a verifier.
To bind new-flow eligibility to a deployed application and its readiness fence,
add the signed artifact name:

```sh
./scripts/run-aarch64.sh release \
  --cluster-service 10.0.2.42:8080 \
  --cluster-service-name orders
```

Before `orders` is committed, ready, and installed in a complete router
snapshot, no freshly initialized node advertises this VIP. During a move, the
committed policy makes new flows wait for the new exact generation and retained
epochs preserve observed flows. A router that cannot refresh can presently
continue using retained epochs for bound flows, but its five-second lease stops
VIP advertisement and admission of unbound traffic.

Run the complete multi-node validation separately:

```sh
./scripts/run-distributed-ingress-test.sh
```
