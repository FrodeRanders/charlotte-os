# Principal stack and heap history preparation

Date: 2026-10-18. Baseline `76698c82`, after the documentation/comment commits.
This follows the [signed-authority slot work](2026-10-18-root-authority-preparation.md)
and continues SEC-07 R7-1/R7-2 and SEC-18 C02/C03/C18/G1/G4. The 38 finding
dispositions stay at 29 corrected within scope, six partial and three open.

## Source change

Final domain cleanup previously recorded stack and heap peaks with two infallible
`BTreeMap::entry(...).or_insert(...)` calls while lifecycle serialization was held.
The signed principal was known before execution, so cleanup did not need to be the
point where its history storage was allocated.

The two maps now share one admitted record per principal, with separate monotonic
stack-page and heap-byte counters. `PreparingMetadata` fallibly prepares a fourth
node before local lifecycle/table publication guards. Its existing registration
owner retains this candidate with the unpublished root, capability namespace,
sponsor and other metadata. Rejection returns `DomainMetadataAllocationFailed`
and explicitly cancels unused storage outside local guards; partial or complete
abandonment retains every field without allocator or registry work.

Root publication transfers the candidate into its exact-generation authority slot.
After stale-handle, zero-principal and duplicate-authority checks, trusted identity
installation links that node for a new principal before publishing the identity.
For an existing principal it preserves the record and leaves the unused candidate
in the root's slot. Both paths avoid allocation and candidate destruction under
the authority/history guards. Empty authority slots still grant no identity.

Final stack/heap sampling now updates existing counters. It cannot allocate a
missing history record; absence is an internal invariant failure. Queries for a
principal with no observation still return zero, including a newly installed
principal with an empty record. Published records survive domain retirement and
ASID reuse for the boot lifetime, preserving the sizing inputs used by later
generations. This merges existing telemetry storage; it adds no recovery registry,
public API, ABI, wire/storage format or authority to retry physical work.

## Retirement and retention

An unused candidate follows the authority slot through the existing
`ClosingAddressSpace` and `RetiredAddressSpace`. Failed final invalidation keeps
it in the complete receipt. Confirmed invalidation explicitly cancels it during
post-guard metadata disposal, before root backing and slot completion. Both the
candidate wrapper and its containing owners retain allocations on abandonment.
Existing bounded invalidation retry and terminal physical-release rules remain.

The standalone partial-preparation controls now retain ten empty nodes across
four phases, seven more than the previous three-node control. The existing
abandoned complete registration retains one extra candidate. Existing retained
roots also keep their candidate when unused; no additional physical root is
abandoned by this batch. The real restart fixture adds two boot-lifetime history
records, while all three of its roots close successfully. The 64 private-map
controls explicitly release all their history and candidate nodes. These are
real heap costs without a separate metadata-byte/principal admission pool.

## Controls

Each native boot injects 64 rejections at five preparation boundaries: before
limits, after limits, after usage, after authority and after history preparation.
All 320 reject before publication, return root frames and capability admission,
and preserve root occupancy and the three root-registry sizes. Preparation and
ordinary root disposal observe the local publication/allocator guards available.
These are deterministic admission controls, not actual heap exhaustion.

Another 64 root publications hold the heap allocator while relinking all three
root registries and transferring the history candidate. Separately, 64 private
history controls hold it across first-principal publication and duplicate use;
the duplicate preserves the counters and retains its unused candidate until
explicit post-guard cancellation.

Real-root controls install identity and update/query history with the allocator
held. Retirement samples a touched stack mark of seven pages, and a successor
with the same principal preserves that mark and its 8,192-byte heap observation.
Lower and zero samples cannot reduce either counter. The stack-sizing policy
keeps its one-page headroom. An unrelated principal in the same reused ASID
starts with zero counters. These trusted fixture setters do not verify signed
images inside the heap hold; existing signed-loader and security suites exercise
their callers separately.

The existing partial/complete and detached-root abandonment controls now hold
the history guard as well as the allocator and prior guards. Post-invalidation
metadata probes include history-guard availability and preserve entry IRQ state.
These local controls do not qualify every enclosing caller, mask or destructor.

## Validation

The version-controlled [evidence](evidence/2026-10-18-principal-history-preparation.json)
contains the ten frozen Rust source hashes, artifact hashes, commands, outcomes
and concise execution markers. Published conclusions do not depend on temporary
logs or ignored build artifacts remaining available.

Host validation passed 509 Rust tests with zero failures and four ignored tests
across 57 summaries, plus signing/tooling controls. Strict Clippy passed on both
custom kernel targets; formatting and whitespace checks passed. Existing service
bundles were reused, with all 41 AArch64 and 37 x86 inventories unchanged from the
preceding authority batch. This batch changes kernel source only.

Native runs use fresh storage, four LPs and a 180-second fixture deadline:

| Profile | Deferred pass/fail/pending | Completion, guest seconds |
| --- | --- | --- |
| AArch64 / SMMUv3 | 17 / 0 / 0 | 12.096076 |
| x86 / Intel VT-d | 15 / 0 / 0 | 14.728907 |
| x86 / AMD-Vi | 15 / 0 / 0 | 19.869574 |
| AArch64 / SMMUv3 security | 19 / 0 / 0 | 56.705156 |
| x86 / Intel VT-d security | 17 / 0 / 0 | 62.141988 |

All five runners exited zero and recorded diagnostic result `complete`. Across
the matrix, the new controls cover 1,600 preparation rejections, 320 private root
publications and 320 first/duplicate history pairs. Existing IPC controls completed
320 exact close/exit episodes. No native guest failed in this batch.

Two initial sandboxed QEMU launches failed to bind their diagnostic Unix socket
before kernel execution. Their launcher failures are recorded separately from
guest results. Historical intermittent boot/IPC/storage/NIC failures remain
separate observations; passing this batch does not establish their cause.

## Remaining limits

History preparation now rejects fallibly, but total principal/history bytes,
general kernel heap and aggregate logical-principal consumption remain unbounded
by a dedicated admission policy. Every user root has an extra transient candidate,
including roots that never acquire an identity. Signed principals with no samples
now retain an empty boot-lifetime record. Essential-service progress under ordinary
pressure is still unqualified. History lookup/insertion is linear through the
shared admitted map; hostile-pressure latency and guard hold time need evidence.

Allocating telemetry snapshots, completion cleanup and wider caller/IRQ/field
contexts remain separate work. Platform authentication, rollback-resistant state
and key custody are unchanged; production trust remains disabled. SEC-07/18 remain
partial.
