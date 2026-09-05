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

## Identities and authority

The service identity is IPv4 address, IP protocol and port. The ingress
identity is whichever committed node currently advertises the VIP. The
execution identity is the backend selected for a five-tuple. They need not be
the same node.

Only the platform launcher can place `vip` and `vipport` in the `frouter` and
`tcpip` manifests. Applications receive socket capabilities; they cannot alter
ingress policy or cluster membership.

DNS owns the operational Raft member. Its local `OP_INGRESS_MEMBERSHIP`
operation materializes an immutable snapshot containing stable node keys and
the discovery-associated MAC route for every admitted voter. The snapshot
separates that trusted/routable member set from the subset eligible for new
flows. It returns no snapshot unless every committed member has a route.
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
sequence numbers and payload bytes never change. The TCP/IP service installs
the VIP as a `/32` address on every configured backend, separately from its
DHCP or static node address, so the selected backend accepts the packet and
replies with the VIP as IP source.

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
configuration index and the sorted replicated shutdown-intent generations.
It changes for membership or drain-policy changes, but not for unrelated
catalog traffic.
`frouter` retains four membership snapshots and up to 1,024 local
`FlowKey -> epoch` bindings. Retransmitted SYNs and later packets retain the
original epoch. Adding a member consequently affects new flows without
remapping observed connections. When a backend is removed, bindings that
selected it are released so a reconnect can use the active set; bindings
owned by surviving nodes retain their older epoch. A draining backend remains
routable and therefore keeps its observed bindings; new SYNs exclude it.

This cache is deliberately not distributed connection tracking. Another
ingress participant with the same epoch independently selects the same backend,
so failure of the VIP advertiser alone does not destroy backend TCP state.
Bindings can be lost through bounded eviction or simultaneous membership
change and ingress failure; that is an explicit first-version limitation.
Failure of the selected backend may terminate its TCP connections.

VIP advertisement follows the leader elected by the existing Raft group,
provided that identity belongs to the committed eligible set. Only that node
passes VIP ARP requests to `smoltcp`; other nodes drop them, including while no
leader or complete snapshot is known. A new leader transmits a gratuitous ARP
reply. Loss of the ingress owner can therefore move advertisement after an
ordinary Raft election without changing the backend set or introducing a
second consensus system.

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
removal are not implemented. IPv6 neighbour advertisement, service-specific
placement sets, multiple VIPs and transparent TCP state migration remain
extension points. Application state restoration can
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

Run the complete multi-node validation separately:

```sh
./scripts/run-distributed-ingress-test.sh
```
