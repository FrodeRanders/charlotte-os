# Security audit: aggregate backing by signed principal

Date: 2026-10-23. Reviewed baseline: `84224784`.

A signed application now shares one backing allowance across its instances and
address-space generations: **16,384 combined memory-object, heap and image/runtime
pages (64 MiB), and 1,024 memory objects**. Its existing generation and node limits
still apply. Starting another instance, moving a capability away or retiring a
root cannot reset that principal allowance. This is new production enforcement
for SEC-07 R7-3. It does not complete general kernel-heap/principal accounting or
the essential-service pressure milestone.

The [tracked evidence](evidence/2026-10-23-security-principal-backing.json) records
source/model/artifact hashes, controls and the failed native runs separately.
The audit ledger remains **29 corrected / 6 partial / 3 open**. SEC-07 and SEC-18
remain partial, and production trust remains disabled.

## Before and after

Generation-owned budgets already prevented transfers, late unpin and reusable
ASIDs from refunding the wrong generation. Heap and image accounts also kept
failed physical cleanup charged to their original node pools. However, another
instance of the same signed artifact received fresh generation ceilings. Its
stack/heap history survived restart, but no shared principal admission counter
limited these three backing families together.

The existing boot-lifetime principal-history record now holds that counter.
There is no new registry. Verified artifact metadata selects the principal;
manifest names, roles, request bodies and later ASID lookups cannot select or
replace it. Trusted platform promotion changes access to family headroom but
cannot refund or enlarge the principal allowance.

The signed loader binds sponsorship before its optional preparation callback,
ELF/runtime mapping and thread publication. Binding first admits every existing
trusted-preparation charge in one reservation, then splits it among the original
generation, heap and image owners. Rejection leaves authority absent and those
owners unchanged. A preparation failure still has the complete launch owner;
external backing retained from that callback stays charged after the root closes.

Page preparation carries a typed aggregate share into its existing account.
Memory-object generation accounts retain theirs through moves, copies, retired
sponsors and receiver-held backing. Copies pay from the copying caller; moves
keep the original sponsor. Confirmed rollback or physical release precedes refund.
The joint-preparation fallback merges captured shares without acquiring the
principal, pool or allocator lock. Partial/abandoned root destruction retains
aggregate shares behind the existing physical-release fence. No retained counter
is cleanup custody or permission to retry.

The [current contract](../../reference/principal-backing-admission.md), family
references, lock-order row, manual and C02/C03/C18 cleanup rows describe the same
scope. There is no wire, signature, persistent-record or runtime ABI change.

## Verification

Serialized kernel controls allocate real mixed heap/image/object backing through
several roots of one principal. A private six-page/two-object ceiling rejects
256 rounds of object, heap and image requests while another principal continues
allocating and releasing objects. A separate control saturates object count
with page headroom left. Promotion cannot release quota. Copies charge a different
caller, moved objects survive sponsor exit and same-slot restart, and late release
leaves the replacement's heap charge intact.

Atomic binding is checked with existing unbound trusted-preparation backing:
rejection installs no authority or partial principal charge; a smaller admissible
preparation binds and later refunds normally. Two verified instances of one real
signed image contribute both image accounts to the same counter. Their oversized
object requests reject without allocating frames; generation/node checks still
apply, so this test alone does not isolate the principal rejection boundary.

A failed signed pre-launch callback moves one real page to the kernel fixture
receiver. The original unstarted root closes, but its principal retains one page
and one object until receiver release. Existing provisional and final-root fault
controls now reconcile principal retention with their original node charges.
They include tracking/allocation/mapping rejection, confirmed cancellation,
rejected physical release, uncertain publication and partial root destruction.
Preparation Drop additionally runs beneath the principal lock alongside its
original table/allocator/pool guards.

The finite `CharlottePrincipalBacking` model checks three unit backings and three
root slots, including one slot reused by a different authenticated principal.
Its safe configuration explores **38,504 distinct / 148,457 generated states**
to depth **13**. Four negative configurations find the expected violations for
per-generation-only admission, retirement refund, move-based sponsorship change
and late release against a replacement principal. Object counts, hardware,
allocator metadata and wall-clock fairness are outside that abstraction.

Final combined security/HTTP runs pass on both architectures: AArch64 has
**20 passed / 0 failed / 0 pending**, x86 **18 / 0 / 0**. Each completes both
scoped security generations, all 14 host HTTP/deployment liveness controls and
the final cancellation-stress retirement. Healthy replies beside stalled peers
complete in **0.244–0.402 s** on AArch64 and **0.312–0.375 s** on x86. Trickled
bodies reject in **30.18 s** and **30.15 s** respectively, within the existing
fixture policy. Cancellation workers retire **18,444** and **18,352** requests.
These are finite supported-target controls, not a sustained aggregate-pressure
acceptance matrix.

Strict base-kernel Clippy passes for both native targets. Workspace rustfmt and
whitespace checks pass. The full model harness passes **45 safe configurations
and 131 expected counterexamples across 42 modules (176 cases)**. No service
binary source changed, and the checked-in manual PDF is preserved.

## Failed runs and fixture correction

The first AArch64 and x86 runs passed the new mixed admission/rejection checks,
then failed a fixture assertion requiring the first closed slot to be reused.
The allocator reused the other just-closed slot. No quota discrepancy was
reported at that assertion. The corrected fixture reopens the first slot before
closing the second, making its intended exact-generation reuse deterministic.
Both failed runs remain recorded with their original source hashes and captures.

The next runs passed kernel aggregation, signed-instance and retained-cleanup
controls but failed existing EL0 full-quota checks **53 on AArch64** and **55 on
x86**. Those checks expected one rejected/cancelled page to restore a private
ceiling exactly. Sibling pressure workers now share the artifact principal and
can free aggregate headroom independently. The failing checks observed an extra
successful allocation; the captures do not record the simultaneous per-generation
and aggregate counters, so the precise release interleaving is unconfirmed.
The AArch64 kernel panic also interrupted the concurrent host HTTP probe, which
ended with a read timeout; that host failure is preserved rather than counted
as a passing HTTP run.

The test-only launch path now installs a lower **128-page/128-object generation
ceiling before thread publication**. All workers retain their original common
signed principal and ordinary classification. This isolates exact restoration
checks without raising the shared budget, inventing a new principal or resizing
a live sponsor. Production launches keep their normal generation policy. Both
intermediate combined security/HTTP repeats pass after that fixture correction.
Final runs additionally qualify the corrected loader preparation order and the
principal-held abandonment guard.

## What remains

This closes the generation-reset gap for these three backing families. It does
not prevent every route to shared resource exhaustion. Independent principals
can still reach node ceilings, and a node ceiling may reject below the fixed
principal limit. General kernel heap/metadata, principal-history storage, stacks,
page tables, capability/IPC/completion families and other retained state still
need aggregate principal admission. Runtime heap quota failure retains its
existing fatal user-fault behavior; there is no new recoverable allocation ABI.

The serialized pressure control is not concurrent EL0 fairness, a full restart
soak or a guarantee of management/timer/teardown progress under sustained load.
Direct memory-object failed-release injection with a bound principal, broad
cross-LP races, principal-record lookup/IRQ hold times and every enclosing destructor context remain qualification work.
The earlier publication lookup failure, cancellation-stress reclamation timeout
and lost ingress response remain unresolved historical observations; later
passing controls neither attribute nor resolve them.
