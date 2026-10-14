# NS-owned endpoint-death retirement

Date: 2026-10-14. Baseline: `9afff162`. Scope: C18 local publication ownership
and readiness; no caller privilege expansion, runtime ABI or persistence change.

## Finding and correction

The [full-name generation batch](2026-10-14-protocol-simplification-name-generations.md)
correctly denied DNS retirement of an independently Echo-owned NS binding. DNS
lookup/catalog ownership is not registrar or manager authority. Its separate
DNS-owned long alias supplied positive cleanup evidence, but dead third-party
node-local bindings lacked their own producer contract.

NS now stores `ns_publication::Publication`: an owned source Connection and
persistent endpoint-death Completion, instead of a scalar owning connection.
The incoming connection is adopted once at its documented receive adapter.
Watch preparation precedes every policy/registry publication. Submission
rejection disposes only the unused candidate and leaves the current entry
untouched. Publication replacement cancels/disposes the previous watch before
closing its source, after the new generation is authoritative. Explicit
retirement takes the same complete owner and keeps its generation tombstone.
Other raw receive/reply/lookup adapters remain separate work.

The existing kernel endpoint watch closes its registration/check window under
IPC, posts a capability-backed result to the default CQ and signals readiness.
NS binds its borrowed launch-owned request endpoint to that CQ and waits
event-only. It drains ready entries before each request and rechecks lifecycle;
there is no idle timer, watch reconstruction, repeated resource-poll loop or
new per-instance registry. The new `Completion::matches_cq` selects a hint,
then NS observes the owned completion to confirm `IPC_REPLY_ENDPOINT_CLOSED`.
Unrelated/stale/pending entries do not retire anything. Cancellation, unexpected
results, failed observation and rejected CQ waits are uncertain, not death:
NS aborts with the registry retained rather than replaying, clearing ownership
or acknowledging cleanup. Lifecycle requests likewise use terminal abort,
without a cooperative or hardware-quiescence acknowledgement.

Confirmed death unpublishes the current complete name/captured generation
through NS's own policy authority. The watch stays with its original publication;
a replaced entry's late hint cannot supply a replacement's terminal result.
Caller retirement retains the existing exact registrar/manager, kernel envelope
and generation checks. Neither catalog tombstones nor endpoint death grant DNS
manager rights. Source close is best-effort logical teardown; watch readiness
supplies no physical reclamation/refund or uncertain-cleanup replay authority.

## Admission boundary

Live publications are bounded to **64**. The trusted supervisor NS constructor
prepares a **65-operation** completion/record/watch account before thread
publication or event capture, allowing one replacement watch to be prepared
while the old source remains authoritative. Ordinary loaders retain their
16-operation limit. No live sponsor is resized or replaced; platform admission
is selected by the trusted constructor before root backing, never by a role,
artifact name or untrusted manifest, and normal node/aggregate accounting remains.
Tombstones retain generations in the existing bounded policy store without a
live watch. The quota predicate rejects a new 65th binding before policy mutation
and permits a live replacement at the bound. Wider policy/tree allocation,
metadata/Drop and physical close qualification remain separate C18/G1/G2/G4/G7.

## Host and static checks

The host suite passed **479 tests, 0 failed, 4 ignored**, including four new
publication controls: returned source on watch preparation failure; replacement/
explicit retirement disposing watch before source; stale CQ hints not touching a
replacement; pending/uncertain observations retaining sources; and the live
quota/replacement slot predicate. One new runtime control verifies unmatched
hints do no work, matching readiness can remain pending, terminal observation
closes once, and consumed completions ignore later hints. Existing owned-watch
submission failure/cancellation controls also pass. These use bounded mock
operations; no live allocator/watch pool is saturated, and no mapping/DMA
failure or native 64-publication saturation is claimed.

Strict service Clippy passed on ARM/x86; strict kernel Clippy passed with the
native discovery/DNS/deployment/cluster-control feature set. Workspace formatting
and diff checks passed. Signed service builds completed before kernel/guest
launch. All six relevant NS/DNS/Echo ELFs independently verify: NS Bootstrap,
DNS/Echo Service, with no manager grant added.

## First native mixed pair

Fresh v4 stores, four LPs per guest, `--deploy-test`, 300-second guest budgets.
The selector-backed coordinator registered stdout readiness and checked the
actual TCP 19160 listener (QEMU PID 29610) before x86 boot. Runner exits **0 / 0**.

| Control | ARM | x86 |
| --- | --- | --- |
| NS source death | Echo-owned short/default long names retired at 8.059548/8.059932 s | Both DNS-owned long and Echo-owned short/default long names retired at 29.664892/29.665226/29.665557 s |
| Replacement after old endpoint exit | live handoff verified 8.180646 s | live handoff verified 14.177409 s |
| Original IPC exact close/exit watches | 64 episodes passed 9.646142 s | 64 episodes passed 15.348630 s |
| Authoritative suite result | **22/0/0** at 91.535728 s; bitmap `0xcfffff` | **20/0/0** at 42.620853 s; bitmap `0xcfbbff` |

The DNS leader was x86 in this pair. At 30.976326 s the strengthened leader
fixture observed both its DNS-owned full name and independently Echo-owned
short name absent using read-only lookups, without submitting a manager removal
to make cleanup pass. Both guests independently exercised NS-owned death and
replacement fencing in ordinary lifecycle controls. Catalog replication,
full-name calls and all three architecture-pinned rollout generations passed.
This does not newly qualify native wrong-principal requests or all 256-byte
publication paths; existing authorization predicates and envelope rules remain.

| Artifact | Bytes | SHA-256 |
| --- | --- | --- |
| ARM NS | 177,032 | `00b572a9820be8eaa598002de4c6193331df31fc1759fbcbb138959043922a5f` |
| ARM DNS | 920,176 | `a7d6ca48cf0781c61317bf27a8b9aaf936a8e2bb531846069efda8f6e6ddfb0a` |
| ARM Echo | 96,160 | `47b23d38f3b1d2a0f931d6fee20e4658bd00389d8ddc9fef78c6fe146050dce4` |
| x86 NS | 120,648 | `2554365fd1304b1089fa27b2971604c1d680b24ad07062cdb6881b882df7a74c` |
| x86 DNS | 942,512 | `4343416ae1b45aefd812df9fc6028e80fbefa91de1a55438f0cb1dff23785bf2` |
| x86 Echo | 32,264 | `200e436bc17dd084593b801dea2c909a30a203987f8eac622ec26c245255b56e` |
| First ARM kernel | — | `8f1fa829de9f65475fe41c13b2ec42f2205be622de197af4d1007bb28802904a` |
| First x86 kernel | — | `f8437b39cabd2ee4aff3946eed1fd7b5b1574f2b6ed9a7d9baf6d8f92726fb5d` |

NS interface **2**, CNSGEN01, runtime ABI **3**, DNS interface **4**, catalog-v18,
Raft pair-v3 and object-store v4 remain unchanged. The launch/runtime profile
and external broker negotiation boundaries are unaffected.

## Unchanged-payload fresh-store repeat

A second linked pair used TCP 19161 (QEMU listener PID 33232), fresh v4 stores,
four LPs and the same 300-second guest budgets. Signed service bytes and both
executed kernel hashes matched the first pair. Runner exits again **0 / 0**.

| Control | ARM | x86 |
| --- | --- | --- |
| Replacement after old endpoint exit | live handoff verified 8.300748 s | live handoff verified 14.224369 s |
| Original exact IPC close/exit watches | 64 episodes passed 9.753050 s | 64 episodes passed 15.401325 s |
| Authoritative suite result | **22/0/0** at 91.910858 s; bitmap `0xcfffff` | **20/0/0** at 42.737519 s; bitmap `0xcfbbff` |

ARM retired independently Echo-owned default-long/short names at
8.203702/8.204096 s. The x86 leader retired the DNS-owned full name and
Echo-owned default-long/short names at 29.266633/29.267016/29.267374 s; its
read-only absence control passed at 30.516538 s. Both guests completed catalog
replication and all three rollout generations again. Across these pairs,
**256 exact close/exit episodes passed**, without the historical exit timeout
or later persistence failure recurring. Repeats do not prove historical causality
or every possible watch/retirement interleaving.

Native runners checked assembly section permissions (1 ARM/249 x86 entries).
The Markdown ownership/producer/quota references, audit ledger, contributor rule
and LaTeX chapter were updated together. The manual rebuilt through two
successful LaTeX passes; changed pages 91/92 were rendered and visually inspected.
The 214-page PDF SHA-256 is
`d358a8b19ceef2c2ab1fa0af2979faea5e405be6adfaefd68446e60a2aac5b51`.

## Remaining qualification

The old wrong-principal fixture failure remains historical evidence of missing
producer authority, not justification for broad grants. Historical IPC exit
and subsequent persistence intermittence remain open to causal verification;
they did not recur in either corrected pair. Wider raw watches, receive/reply/
transport ownership, metadata/Drop/outer IRQ contexts, DMA/shared-CQ and physical
cleanup proof remain separate. No HTTP/S3/Kafka or production trust qualification
is claimed here. Published evidence above is inline or version-controlled,
not dependent on temporary captures or ignored artifacts.
