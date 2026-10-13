# Current-only catalog snapshots and explicit restore rejection

S04a of the [simplification sweep](../../reference/api-simplification.md), based
on `c5c434a6`. This is the storage/query portion of deployment/catalog removal;
packed deployment APIs and descriptor-less launch callers remain S04b work.
No production stores require migration now. Future deployed-store changes must
include intentional, interruption-tested migration rather than silent reset.

## Changes and enforcement

`NameCatalog` accepts only the complete version-15 `CATALOGF` snapshot. The
writer's existing bytes and magic remain unchanged. Versions 1–14 and their
synthesized generation, active-state, digest, descriptor, replica, operational
authorization, shutdown, ingress and capacity defaults are removed. Every
current section is mandatory. Restore rejects trailing bytes, duplicate keys,
noncanonical flags, zero generations, excessive collection counts and
inconsistent deployment primary/replica state before changing live collections.

The prior void `StateMachine::restore` allowed the catalog to reject bytes
silently while Raft still installed the snapshot and advanced applied indices
and membership. It now returns an explicit error with a contract that errors
leave application state unchanged. `RaftNode::new` propagates rejection before
exposing the voter or clearing join admission. DNS/generic service callers
handle startup errors explicitly. Network installation checks application
restore before log replacement, applied progress, membership and join completion;
failure drops the pending stream and returns unsuccessful/not-done, offset zero.
Term/vote and leader-contact processing retain their ordinary Raft behavior.

Deployment queries now require the entire descriptor-length and replica-count
sections, exact byte consumption, a nonzero generation, sorted unique nonzero
replicas and a matching diagnostic primary. The old 56-byte singleton prefix
and missing-replica suffix reject; the decoder cannot reconstruct an assignment
from the primary scalar. Encoding enforces the same invariant. Existing full
records, including explicitly empty descriptors used by current legacy callers,
remain representable until S04b removes those producers and launch paths.

See the [exact contract](../../reference/catalog-snapshots.md). Markdown
architecture/references, contributor instructions and the LaTeX/PDF manual are
updated. The manual's stale state-machine signature now matches the Rust API.
Historical audit reports retain their original observations and finding counts.

## Regression evidence

Eight new host tests cover:

- Each of the fourteen retired tags rejects and leaves the complete live
  snapshot unchanged; the current empty catalog still restores successfully.
- Every truncated prefix of a populated registration/deployment/release/key
  snapshot, plus trailing bytes, rejects without partial publication.
- Duplicate service keys, invalid active/presence flags, an absent key with a
  generation, missing sections and counts above each collection bound reject.
- Complete deployment queries round-trip; every truncation, trailing bytes,
  zero generation, inconsistent primary, zero/duplicate/unsorted assignments
  reject. An explicitly empty assignment requires a zero primary.
- Restart rejects unsupported application state while retaining durable
  snapshot, term, vote and join admission.
- A two-chunk unsupported snapshot returns failure without publishing log,
  commit/applied indices, membership epoch or join completion. A subsequent
  supported stream installs successfully and completes the original join.

`scripts/run-host-tests.sh` passes **410 / 0 / 4 ignored**, including tool,
signing-policy, boot-result, syscall-emitter and signature/placement self-tests.
Both signed service bundles build (41 ARM and 37 x86 ELFs). Strict service and
both deployment-profile kernel Clippy checks pass with `-D warnings`; workspace
formatting and diff checks pass; 685 local Markdown links resolve. Two intermediate lint runs rejected a nested
conditional and redundant count casts; those mechanical corrections passed
before final bundle generation. They were not guest failures.

Signed-bundle SHA-256 is over sorted newline-delimited `basename:ELF-SHA256`
entries:

- ARM: `1f7c9257703838c96c4ca18aa2f63ad3c59bbf9b23f11e76a8997f8a29f50f5a`.
- x86: `31307eee4e6d8edcb952bff0551c484fbc0dcfc8a2506e3e3175e7b718f51aed`.

The rebuilt manual remains 208 pages; changed pages 102–103 were rendered and
visually checked. PDF SHA-256:
`7a6238d1e3f33eb7d8424b739f276454587ed866af897d1e5ffd4edbe1580983`.

## Initial paired guest failure and fixture correction

The first fresh-storage deployment pair failed in both guests at
`self_test/el0_clusterctl.rs:249:51`: range end index 8 exceeded a zero-length
status buffer. ARM reported 18 passed / 1 failed / 3 pending at 13.450542 s;
x86 reported 16 / 1 / 3 at 24.453510 s. Both had committed deployment generation
1 before the verifier failed. Failed kernel SHA-256 identities:

- ARM: `c12af174f7e1a796616f6c5b07726a889c0790e25171a2dfc23971e5f2bf632e`.
- x86: `a4f4e520a6203b7b8c4b90069709cfc2970d11fa624bf17382757fc3e265ec77`.

Source identifies a deterministic fixture mismatch: its status/key reader
called public `memory::object::map(KERNEL_ASID, ...)`, discarded any error and
returned an empty vector. That API acquires `AddressSpaceOperation`, whose
admission explicitly rejects the kernel address space. No mapping-error code
was captured in these historical failed runs; the causal path is supported by
that unconditional source rejection, not by a claimed observed error enum.

The raw kernel verifier now uses the existing fallible `snapshot_bytes`
boundary, as the join fixture already does, then explicitly closes its uniquely
received capability. Read/close errors return a typed result; the automated
verifier gives an explicit diagnostic and the serial console reports the error.
No low-address scratch mapping, unsafe empty-vector indexing, root-admission
exception or physical-retirement bypass is introduced. C08 evidence retains the
existing ownership policy. The failed pair used MAC suffixes 61/62; the repeat
uses the deployment fixture's expected 01/02 identities for its fixed target.
This topology correction did not cause the initial mapping failure.

## Corrected reader in an unsuitable mixed-architecture deployment pair

With the new reader, both guests passed clusterctl status/key checks, observed
`generation=1 node=0x42e27737`, completed DNS remote echo and membership join,
and then failed the deployment's owner-generation convergence deadline at
`self_test/el0_dns.rs:332:22`. ARM finished 21 / 1 / 0 at 146.531030 s;
x86 finished 19 / 1 / 0 at 150.791782 s. Their kernel identities were
`b9f090fdc56e644a746c477d00d30ba35e02fd66e56d760a7ff410e0eda3856b`
and `239c66e5448de0e0919806f7071a66d3cc7998d3b5c2014885906cf80c477b5b`.

This pairing was an invalid test choice: the legacy deployment fixture stages
its local architecture's ELF on each node, but the leader commits one exact
artifact digest. The ARM and x86 `greet.elf` hashes are respectively
`cb9418da948192975dec9de574928b339e47c5ffcd10a1bd689c13ccb638bf88`
and `e543f0133d2b0067906396f2f45d28f2df2328c7222ef7b5f719ba93be6f42a6`.
The x86 agent logged launch rejection with bounded backoff through failures
1–6. Source verifies the local object against the committed digest before
launch; these distinct artifacts cannot fulfill that shared identity. No
permissive digest or architecture fallback was added. The testing guide now
states that deployment pairs require identical architecture/bundles and the
fixture MAC identities; mixed architecture remains suitable for DNS testing.
The failed sample remains separate from subsequent matching-architecture runs.

## Matching-architecture native validation

Two fresh-storage ARM guests and two fresh-storage Intel x86 guests passed
with the same final sources and signed bundles. Each matching pair uses
`--deploy-test`, MACs `52:54:00:12:34:01` and `52:54:00:12:34:02`, socket
listen/connect, and a 360-second runner bound. ARM used port 19477, x86 19478.
The prebuilt debug kernels enable `acpi`, `virtio_net_test`, `disco_net_test`,
`disco_cross_node_test`, `dns_net_test`, `deploy_net_test` and `clusterctl_test`.
Services were rebuilt and signed; skip-build options applied only to these
exact prepared artifacts. Native assembly-section checks passed in the runners.

| Guest | Passed / failed / pending | Completion time |
| --- | --- | --- |
| ARM node 01 | 22 / 0 / 0 | 34.267405 s |
| ARM node 02 | 22 / 0 / 0 | 34.854574 s |
| Intel x86 node 01 | 20 / 0 / 0 | 40.493123 s |
| Intel x86 node 02 | 20 / 0 / 0 | 40.852482 s |

Both pairs verify clusterctl status and key ceremony, DNS catalog/remote echo,
exact-generation tombstones, dynamic membership, note-signed cross-node
`greet` invocation, migration, cooperative retirement and zero-grace forced
retirement with replacement reachability. Both architecture profiles include
the ordinary owned-runtime/CQ/storage/scheduler fixtures. The ARM pair's kernel
SHA-256 is
`b9f090fdc56e644a746c477d00d30ba35e02fd66e56d760a7ff410e0eda3856b`;
the x86 pair's is
`239c66e5448de0e0919806f7071a66d3cc7998d3b5c2014885906cf80c477b5b`.
Each guest's immutable boot sidecar agreed with its pair's identity.

These boots validate integration and fixture cleanup, while the host tests
exercise unsupported/malformed application snapshots and install rejection.
They do not force a DNS snapshot threshold or crash a live DNS quorum during
restore. The mixed-pair failures above remain separate evidence, rather than
being erased by these passing samples.

## Limits and remaining work

This introduces no new resource owner, readiness source, cleanup registry or
physical completion proof. C08 retains the existing pending-call ownership:
logical snapshot rejection cannot authorize replay of an accepted IPC or
physical teardown. Infallible snapshot allocations and existing per-section
locks remain; this is not concurrent reader snapshot isolation or power-loss
atomicity across application restore and durable log writes. Generic Raft nodes
without an application state machine retain their existing opaque-snapshot
behavior. Shared membership-envelope compatibility is separate S08 work. The TLA+
conformance mapping now distinguishes pre-persistence application validation
from successful post-persistence logical activation. The bounded model omits
application rejection and does not prove this new error path or write-failure
atomicity; the executable host tests cover rejection separately.

Descriptor-less commands/records, packed-name deploy/query/spawn APIs, local
object-upload fixtures and the redundant primary field still require ordered
caller replacement. Current-only snapshot/query decoding does not close those
removal tasks or any broader security finding.
