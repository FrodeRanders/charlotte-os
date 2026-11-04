# Supervisor launch, handoff and store cache — Q2

Date: 2026-11-04. Base: `85e224ee`. This accepts the named Q2 scope in the
[SEC-07/18 plan](../../reference/security-closure-plan.md): complete launch and
upgrade parents, handoff storage and permanent store-image backing. Q1/Q2 are
accepted; six packages and all aggregate gates I/C/P/M remain open. This is not
a weighted readiness percentage or closure of SEC-07/18.

## Result and scope decisions

`DomainLaunchTransaction` retains loaded target/profile and exact source/target
operations. Its ordinary cancellation attempts each cleanup once, stops on the
first error and retains remaining state. Its destructor performs no cleanup.
`PreparingUpgrade` owns every actual transfer, both admitted arrays and that
launch owner. The actual upgrade syscall now uses this shared path. IPC captures
the old endpoint's generation before replacement classification; source and
naming authority retain independent exact roots. This removes duplicate scalar
rollback and later-ASID classification. The register ABI and reference single-state
handoff remain unchanged.

The two handoff arrays reuse `ByteAllocator<IpcNodes>` and original state-source
class. The 32-entry launch vector permits 31 states without an endpoint, or 30
with one. A transfer is 328 bytes on both targets; transfer+ID arrays request at
most 10416 bytes. Existing 12/16 MiB IPC ceilings remain adequate.

The store cache now reserves fixed publication storage/count before I/O and
owns its admitted immutable vectors for the kernel lifetime. No growing table
or leaked plain vector remains. Its separate `service_images` pool has 16 MiB/64
total and 12 MiB/48 ordinary defaults; current compiled-name kernel producers use
the immortal kernel's class. Temporary seed reads contribute bytes, zero entries.
This preserves the 12/16 MiB temporary snapshot budget instead of allowing
permanent cache retention to consume it. The combined 16+16 MiB requested-byte
ceilings still compete with other families/overhead within at least 72 MiB heap;
Q4 must qualify aggregate sizing. Heap reserve and protocol maxima do not change.
The fixed table is kernel storage, not heap-node backing.

Admission exposed a related repair hazard: seeding previously treated any failed
read as missing/invalid content. It now stops on denied, malformed, I/O, IPC or
snapshot failure. Only explicit not-found or completely returned invalid content
can authorize repair. Persistent formats and signature policy do not change.

The new pool appends catalog entry 24 and statistics version 11; prior IDs,
header/record widths, syscall registers and runtime ABI 3 stay fixed. Both
architectures' test/minimal bundles and the ARM test application were rebuilt and
signed against the current SDK. Old statistics versions reject.

## Qualification

[Retained evidence](evidence/2026-11-04-supervisor-launch-cache.json) includes
source/artifact identities, commands, inline diagnostics, the rejected unsafe
cache-Drop control, and the minimal-build preflight/final source distinction.
Fresh four-CPU security campaigns with a 240-second guest bound passed:

| Target | Passed | Failed / pending |
| --- | --- | --- |
| AArch64 / SMMUv3 | 19 | 0 / 0 |
| x86 / Intel VT-d | 17 | 0 / 0 |
| x86 / AMD-Vi | 17 | 0 / 0 |

All three include existing live state handoff and persistent signed-image upgrade
through the changed syscall path. New serialized controls exercise real target,
source, profile and transfer ownership; complete launch/handoff abandonment runs
unguarded first and then with lifecycle/table/IPC/heap/physical/object/capability/
pool guards held. Charges, frame availability and IRQ state stay unchanged.
Ordinary profile/domain/source cancellation, stale generation, bootstrap failure,
post-profile-transfer limits rejection and stop-on-busy-profile/root paths are
separate controls. Array quota rejection occurs under a held heap guard before
backend entry, both empty and after a first-vector prefix, for both classes.
Invalid-cap transfer failure cancels its actual prefix and restores source access.

Native cache controls retain four reservations and two private 37-byte images on
abandonment before/after copying, including heap/cache/pool guards. A signed store
read publishes immutable bytes; a cache hit works under held heap/pool guards.
After boot seeding, the fixture holds the global reader gate against all other
cache misses during total-byte counter pressure. Reseeding returns error, the
installed image compares unchanged after pressure release, and temporary bytes
recover exactly. This is a deterministic error-path control, not concurrent
hostile pressure or a management-latency qualification.

The full host suite passed. Its new production-cache harness passes 47 tests
(five new, 42 shared), including actual allocation/deallocation ordering, count
admission, real backend rejection and four actual buffers reaching the 16 MiB
byte ceiling. An injected active preparation destructor fails the exact retained
prefix assertion: 46 tests pass and that one fails. It is an expected unsafe
control, not a passing variant. Published entries in the real static cache never
evict; local host-cache disposal only observes destructor ordering.

Final strict test/minimal Clippy checks on both targets, formatting and diff
checks pass. Initial minimal compilation rejected an unused fixture import and
the now-test-only default-allocator snapshot wrapper. After all guest campaigns,
the correction moved that import into its fixture module and restricted
`Global`/`snapshot_bytes` to `test`/`kernel_tests`. Full-test bodies and services
are unchanged. The evidence preserves the preflight diagnostics, exact native
and final source hashes and their three-file diff; guest results remain bound
to their original artifacts. The final strict checks cover the corrected sources.

## Remaining boundaries

Current contracts are in [launch/cache ownership](../../reference/service-launch-ownership.md)
and the existing C18 owner-family row. No cleanup family, recovery registry,
scalar replay or force-refund mechanism is added. Busy/uncertain cancellation
retains state; it does not return a recoverable continuation.

Earlier loader `PreparingDomain` cleanup, other mandatory boot constructors,
store IPC-cap/lookup helpers, observer startup-claim publication and arbitrary
caller/IRQ/termination contexts remain Q3/Q5 review. The existing reader yield
loop has no notification/abandoned-reader recovery contract. Maximum arrays do
not represent 31 simultaneous physical moves or an end-to-end multi-state
endpoint upgrade. No general native heap OOM or partial physical-release/device
recovery injection is claimed.

New terminal probes retain seven object data pages, exact root/table/account
state, two handoff vector pairs and escrow, four cache reservations/two 37-byte
images. The busy-root probe also retains empty target/source roots. Tests never
reconstruct abandoned owners to reclaim them. Q6 recovery, Q4 aggregate
principal/heap coverage and Q7/Q8 saturated progress/integrated pressure remain
open. These passes do not explain the older Intel retirement-pin or ARM
exit-watch incidents.
