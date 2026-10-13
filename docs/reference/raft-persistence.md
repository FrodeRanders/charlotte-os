# Current Raft persistence

`catten_services::disk_raft::open` opens the complete durable pair before either
store is exposed to Raft. DNS always requires it. The generic Raft service
accepts explicit memory mode (`storage=0`, or omitted) or required durability
(`storage=2`); mode 1 and malformed policy values reject. A rejected disk pair
never becomes an in-memory voter.

The embedded application snapshot has its own
[current-only catalog contract](catalog-snapshots.md). A structurally valid
disk pair does not imply a supported application snapshot: Raft construction
propagates application restore rejection without resetting the durable pair.

## Object layout and admission

Cluster/node identity hashes select a stable two-object range:
`base = 0x8000000000000000 | (namespace & 0x0ffffffffffffffe)`.
The state object is `base`; the log object is `base+1`. Both remain in the
service-owned `0x8xxx` partition, disjoint from node identity (`0x9xxx`) and
artifact/executable namespaces. This replaces the former four-slot layout.

Only **two absent objects** authorize fresh initialization. Opening reads both
before any creation or write. Exact successful creation of both objects is
required; `ERR_EXISTS` is a rejection, not initialization authority. The
initializer writes canonical empty state and log records and flushes before
returning. A read, creation, write or flush failure stops the attempt without
rollback, overwrite or replay. Partial initialization may leave an incomplete
pair that rejects on the next open; it is never silently reformatted.

Existing records must both be present and valid. One missing record produces
`IncompletePair`; empty/truncated or corrupt records produce `MalformedRecord`;
a different format tag produces `UnsupportedFormat`. I/O and creation failures
remain `Io`. Opening valid records performs no writes. No retired slot probes,
split-snapshot migration, term/vote-only defaults or raw-entry fallback remain.
Local allocation rejection during record reconstruction also rejects through
`MalformedRecord`; runtime buffer/admission failures can return `Io`. These
labels alone do not establish on-disk corruption or authorize reformatting.

The version-3 pair tags cover the current catalog command grammar as well as
snapshot bytes. Former v1/v2 pairs reject even when their application
snapshot is empty: an old log cannot replay retired deployment
commands into an apparently empty catalog. Field widths and object IDs remain
as below. No tag rewrite or automatic reset is permitted.

## Record grammars

All numeric fields are little endian. The state record is:

| Bytes | Field |
| --- | --- |
| 0–7 | `CRFTSTA3` |
| 8–15 | Current term, u64 |
| 16–19 | Voted-for byte length, u32, at most 256 |
| 20–23 | Join-anchor byte length, u32, at most 256 |
| 24–31 | Join-admission snapshot index, u64 |
| 32 onward | Exactly the voted-for and join-anchor UTF-8 bytes |

Zero voter length means no vote. Zero anchor length means no admission and
requires snapshot index zero. A present vote or admission cannot have an
empty identifier. All join fields are required even when admission is absent.

The current log record uses `CRFTLOG3`:

| Bytes | Field |
| --- | --- |
| 0–7 | `CRFTLOG3` |
| 8–15 | Snapshot index, u64 |
| 16–23 | Snapshot term, u64 |
| 24–31 | Snapshot byte length, u64 |
| 32–39 | Encoded entry-stream byte length, u64 |
| 40 onward | Exactly the snapshot and entry stream |

Each entry has term u64, peer-ID length u32, data length u32, UTF-8 peer ID
(at most 255 bytes), and exact data bytes. Encoding rejects oversized fields
instead of truncating IDs. Decode rejects malformed entries and trailing
partial entries as a whole; it never returns a usable prefix or substitutes an
empty peer ID for invalid UTF-8. The retired all-zero entry sentinel rejects.
Snapshot-plus-entry index arithmetic and aggregate lengths must not overflow.
Snapshot payload/membership semantics remain the consensus layer's contract.

Snapshot installation and retained suffix share one serialized log-object
replacement. Term/vote/admission mutations replace the state object. Each
mutation flushes before returning; persistence or encoding failure is fail-stop.
The two objects are not a cross-object atomic transaction. In-memory record
mutexes leave before object-store IPC. Object-store copy-on-write and its
content/metadata checks remain the physical durability boundary.

## Ownership and waiting

The two stores share one `Arc` holding a uniquely owned runtime `Connection`.
Each write prepares both `OwnedMemory` buffers before submission and moves
them through `PendingCall::wait`; reads keep returned memory in `MappedMemory`
while copying exact bytes. Errors use the runtime's existing owned cleanup.
No scalar owning connection, fixed scratch virtual address, manual cleanup
ladder or accepted-call replay remains. Persistent object IDs are durable names:
dropping the stores closes their connection, never deletes the records.

This adds no physical retirement owner or kernel recovery registry. A reply or
valid record is not proof of DMA/root quiescence. General Raft reactor ownership,
admission backoff and broader cleanup-context qualification remain separate work.

## Current rollout and future evolution

There are no production stores to migrate for this change. Rebuild the signed
services and regenerate development fixture disks intentionally. Existing
developer disks are not automatically rewritten or deleted. The tagged state
record and compact pair are a new contract; old stores are unsupported.

Once persistent stores are deployed, every format or object-layout change must
ship an explicit version-directed migration with the change. Preserve term,
vote, admission anchor/fence, snapshot membership and exact retained log suffix.
Use a separately validated converter or staged replacement with a durable
commit point and interruption recovery; qualify old/new round trips, failed
writes/flushes and restart before retiring the old reader. Never reuse an old
tag while its old interpretation remains accepted, or treat unsupported data
as absent storage. The current code intentionally contains no speculative
future converter or retained older decoder.

See the [simplification ledger](api-simplification.md) and
[batch evidence](../reports/audits/2026-10-13-protocol-simplification-raft.md).
