# Catalog snapshots and deployment query records

The catalog accepts only version 18, numeric magic `0x434154414c4f4749`
(`CATALOGI`), encoded little endian. Its deployment records retain generation,
signed descriptor and exact replica variant pins. Object ID, diagnostic primary
node and variant-zero digest are removed. Versions 1–17 reject; no production
stores need conversion for this pre-production change. The Raft pair uses
`CRFTSTA3` / `CRFTLOG3` so old command logs also reject without a snapshot.

## Complete snapshot

After the eight-byte magic, every section below is mandatory, in order.
Collection counts are little-endian `u32`; variable byte strings use a `u32`
length. Replica counts use `u16`; scalars otherwise retain their declared
widths. Presence and active flags accept exactly zero or one.

1. Service registrations: name, node, generation, active, deployment generation.
2. Deployments: artifact map key, generation, descriptor, replica count and
   sorted unique nonzero nodes, each followed by its signed variant index.
   The map key must equal the descriptor's signed artifact name.
3. Releases: name, generation, operations sequence, operations digest, envelope.
4. Operational bindings: complete release/profile/object identities, replay
   fences, recipient/signing IDs and detached authorization signature.
5. Shutdown intents: node, generation and signed envelope.
6. Per-artifact readiness: replica registrations with their exact deployment
   generation.
7. Ingress policy: presence, then generation and signed envelope when present.
8. Cluster key: presence, generation and 32-byte key. Absence requires both
   generation and key bytes to be zero; presence requires a nonzero generation.
9. Node capacity: complete node/boot/epoch/frame/load samples and canonical 16-byte execution targets.

Restore rejects missing sections, truncation, trailing bytes, duplicate keys,
noncanonical flags and counts above the existing collection limits. Service,
deployment, release and shutdown collections are bounded at 4,096 keys;
operational bindings at 8,192; capacity samples at 256. Deployment readiness
has at most 4,096 artifact keys. Variant indices must be in range for the
current descriptor; empty assignments remain explicit. No primary or scalar
content identity is synthesized.
Existing signed-envelope validation remains in place; this codec is not a new
authentication or platform-trust boundary.

All sections parse into temporary collections before replacing live state.
`StateMachine::restore` returns `SnapshotError::Unsupported` or `Malformed`
without changing application state. The default implementation accepts only
an empty snapshot. `RaftNode::new` propagates rejection before exposing a node
or clearing its durable join-admission fence. DNS stops startup on that error.
Network installation validates application restore before replacing the log,
publishing commit/applied indices, membership, or join completion. Rejection
discards the pending stream and returns unsuccessful/not-done with offset zero;
a supported stream can then start again. Normal term/vote and leader-contact
processing still precede application validation.

## Deployment query record

The sole reply grammar is
`CDEPQR01:8 | generation:u64 | descriptor_len:u32 | descriptor |
replica_count:u16 | replicas:[(node:u64, variant:u16)]`.
It has exact total length, nonzero generation, a current descriptor and sorted,
unique nonzero nodes with aligned, in-range variant indices. Its signed name,
object keys and digests come only from the descriptor; there is no diagnostic
primary field or implied singleton. The former untagged 56-byte scalar prefix
and longer old query records reject. All current consumers use this codec.

## Command and admission contract

DNS `OP_DEPLOY_SUBMIT` (33) carries only the exact signed descriptor in moved
memory, with its length in `arg0`. The former redundant named envelope (15)
is retired. Follower relay tags 0x22/0x23 carry correlation/session identity,
caller and descriptor; tags 0x17/0x18 reject. Source-route checks remain intact.
Single-component Raft command 0x0e carries resolved node:u64, variant:u16,
descriptor_len:u32 and descriptor. Its name comes from the signature; retired
command 0x05 rejects. Apply checks signature, singleton placement, signed pin,
node profile compatibility, sequence and exact-state idempotence before mutation.
Release and reassign keep their current exact replica pins.

## Future migration

Once stores are deployed, snapshot evolution must ship an explicit migration
preserving application state, membership and replay fences, with durable commit
and interruption/restart tests. Unsupported data cannot be treated as an empty
catalog. Current restore still uses infallible collection allocations and the
existing per-section publication locks; this change does not establish a
fallible allocation transaction, concurrent reader snapshot isolation, or
power-loss atomicity across application restore and persistent log writes.

See [execution evidence](../reports/audits/2026-10-13-protocol-simplification-catalog.md).

## Canonical registration commands

Opcode 0x01 is `name_len:u32 | name | node_len:u32 | node |
deployment_generation:u64`, after the opcode byte. Every command carries the
last field: zero explicitly identifies a system service, and nonzero binds an
application to its exact desired generation and committed replica node. Missing,
partial or trailing bytes reject before generation reservation or catalog mutation.
All callers use `encode_register(name, node, deployment_generation)`; the
separate deployment encoder and suffix-free system grammar are removed.
Preparation and later exact-generation activation remain separate operations.
The snapshot and Raft pair tags advance together so old command logs cannot
replay the suffix-free grammar, including stores without a snapshot.

## DNS observation boundary

Persistent catalog-v18 and Raft pair-v3 remain unchanged by S08b2b. The DNS
`OP_CATALOG` local observation uses the separate complete bounded
[CDNSCAT1 grammar](dns-catalog.md). Its former untagged one-page writer and
partial HTTP reader are removed; this read does not restore state or establish
a linearizable lookup. Full identities and repeated replica names survive.
