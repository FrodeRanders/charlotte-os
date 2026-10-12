# Current discovery protocol

CharlotteOS discovery uses EtherType `0x88B6` and version 1. There is one
complete advertisement grammar for broadcast probes and unicast responses.
Discovery supplies transient identity-to-MAC routes and cluster posture;
it does not authenticate a peer or grant voting, deployment or capability
authority. Membership still requires the replicated admission path.

## Frame and advertisement

| Frame bytes | Required field |
| --- | --- |
| 0–13 | Ethernet destination, source and EtherType |
| 14–15 | Version 1, big endian |
| 16–17 | Exactly one flag: probe 1 or response 2, big endian |
| 18–23 | Source MAC, equal to the Ethernet source |
| 24–25 | Advertisement byte length, big endian, 13–256 |
| 26–41 | Cluster ID, 16 bytes |
| 42 onward | Exactly the declared advertisement, followed only by permitted Ethernet padding |

The advertisement is `node_id_len:u8`, UTF-8 node ID, `service_name:u64`
(little endian), `role:u8`, `raft_id_len:u8`, UTF-8 Raft ID,
`leader_id_len:u8`, UTF-8 leader ID. Node IDs have 1–63 bytes; Raft and leader
IDs have at most 255 bytes each. All fields together must fit 256 bytes.
Roles are no-cluster 0, follower 1, candidate 2 and leader 3. An empty Raft or
leader ID expresses absence of that routing hint; a node without a cluster
normally advertises role 0 and empty IDs. Cluster fields are always present.

`build_peer_payload` returns `None` without modifying its buffer when any
field or the complete size is invalid. It never emits a base-only prefix.
`parse_peer_payload` requires the complete record and rejects extra bytes.
`parse_disco_frame` validates the header, declared length and advertisement
together. It accepts an unpadded short frame or zero padding to exactly 60
bytes (Ethernet's minimum excluding FCS). Partial padding, nonzero padding
and bytes beyond that minimum reject. Rejected frames do not learn peers or
trigger a response. Valid frames still undergo local cluster-ID and self-MAC
filtering before learning.

## Removal and rollout

The former base-only builder/parser, optional extended-payload API and
`ROLE_UNKNOWN` compatibility value are removed. Old frames used zero in
bytes 24–25; those headers reject, including full extended advertisements.
Declaring the record length prevents old short payloads from acquiring a
synthetic empty cluster block through Ethernet padding. The existing version
number remains 1; no second decoder or old header interpretation remains.

Rebuild every node's signed discovery service together. Mixed old/new service
bundles are unsupported: old nodes cannot satisfy the declared-length
contract. No stored discovery state needs migration; peers are a transient,
bounded cache. `OP_LIST_PEERS` and `OP_CLUSTER_STATUS` IPC layouts remain as
documented in `charlotte-protocol-disco`; this batch changes the Ethernet
advertisement boundary.

See the [simplification ledger](api-simplification.md) and
[execution evidence](../reports/audits/2026-10-13-protocol-simplification-discovery.md).
Periodic discovery probes, cached Raft-posture refresh and the existing
service reactor's ownership/readiness gaps remain separate audit work.
