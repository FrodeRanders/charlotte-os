# Catalog snapshots and deployment query records

The catalog accepts one snapshot grammar: version 15, with numeric magic
`0x434154414c4f4746` (`CATALOGF`), encoded little endian. The current writer's
bytes are unchanged. Versions 1–14 are unsupported; their decoders and defaults
are removed. No production stores require conversion for this change.

## Complete snapshot

After the eight-byte magic, every section below is mandatory, in order.
Collection counts are little-endian `u32`; variable byte strings use a `u32`
length. Replica counts use `u16`; scalars otherwise retain their declared
widths. Presence and active flags accept exactly zero or one.

1. Service registrations: name, node, generation, active, deployment generation.
2. Deployments: artifact name, object ID, diagnostic primary node, generation,
   SHA-256 digest, descriptor, replica count and sorted unique nonzero nodes.
3. Releases: name, generation, operations sequence, operations digest, envelope.
4. Operational bindings: complete release/profile/object identities, replay
   fences, recipient/signing IDs and detached authorization signature.
5. Shutdown intents: node, generation and signed envelope.
6. Per-artifact readiness: replica registrations with their exact deployment
   generation.
7. Ingress policy: presence, then generation and signed envelope when present.
8. Cluster key: presence, generation and 32-byte key. Absence requires both
   generation and key bytes to be zero; presence requires a nonzero generation.
9. Node capacity: complete node/boot/epoch/frame/load samples.

Restore rejects missing sections, truncation, trailing bytes, duplicate keys,
noncanonical flags and counts above the existing collection limits. Service,
deployment, release and shutdown collections are bounded at 4,096 keys;
operational bindings at 8,192; capacity samples at 256. Deployment readiness
has at most 4,096 artifact keys. The primary node must equal the first explicit
replica, or zero for an empty assignment. No old fields are synthesized.
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
`generation:u64 | object_id:u64 | primary:u64 | sha256:32 |
descriptor_len:u32 | descriptor | replica_count:u16 | replicas:[u64]`.
Every field is required, with exact total length, nonzero generation and sorted,
unique nonzero replicas. The primary is diagnostic and must match the replica
set; it never supplies a missing singleton assignment. Both encoder and decoder
enforce that relationship. The old 56-byte prefix and descriptor-only suffix
are rejected. Current producers and consumers share this codec.

## Remaining removal and future migration

This is S04a, the storage/query portion of the
[simplification sweep](api-simplification.md). Packed-name deployment APIs,
descriptor-less commands/records and local-upload launch fixtures remain until
their callers have a coherent signed replacement. An explicitly encoded empty
descriptor is therefore still representable in the current snapshot/query
grammar; its missing length or replica section is not. The diagnostic primary
field also remains until the API/record replacement removes its consumers.

Once stores are deployed, snapshot evolution must ship an explicit migration
preserving application state, membership and replay fences, with durable commit
and interruption/restart tests. Unsupported data cannot be treated as an empty
catalog. Current restore still uses infallible collection allocations and the
existing per-section publication locks; this change does not establish a
fallible allocation transaction, concurrent reader snapshot isolation, or
power-loss atomicity across application restore and persistent log writes.

See [execution evidence](../reports/audits/2026-10-13-protocol-simplification-catalog.md).
