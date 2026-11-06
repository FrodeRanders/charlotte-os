# Q4 namespace-account principal and actual-layout admission

Baseline `63e76d20`. The four namespace-account types now fund their actual
payload Arc and private-holder layouts against a shared signed-principal byte
ceiling. Capability, IPC endpoint, IPC record and mailbox accounts retain their
existing global bundle admission and complete-owner cleanup rules. **Q4 remains
in progress** for other account/metadata families, allocator bookkeeping/excess,
external reference fields and coherent joint heap capacity.

The [qualification evidence](evidence/2026-11-06-q4-namespace-principals.json)
records frozen inputs, actual layouts, artifacts, controls and final campaigns.
The [24-pool checklist](../../reference/metadata-admission.md#principal-coverage-and-remaining-reconciliation)
updates the existing namespace-account row; no new object family is introduced.

## Shared mechanics and original capture

`SponsoredAllocator<S>` implements requested-layout funding at actual allocation
and free. `ByteAllocator` composes its existing family and principal policy
through that same adapter, preserving full growth overlap and original-class
refunds. No new global pool or allocation controller is added.

`ChargedAllocator<C, P, A>` retains the existing linear family charge and payload
policy, with an optional captured backing allocator for both heap allocations.
Its default remains Global; unmigrated owners retain their representation and
policy. These four account types select the sponsored allocator. Arc supplies
both actual layouts, including control words and alignment; no private Arc header
formula is reconstructed. A clone copies policy without reserving another object
or granting a second payload allocation.

Each allocation refunds principal bytes only after actual free. Weak-only payload
storage remains charged. An extra allocator clone can retain private-holder bytes
after the payload is gone; the family slot returns only after both allocations
are freed. Ordinary rejection preserves payload-before-family-charge disposal;
partial preparation and complete parent abandonment retain all fields without
allocation, logging, counter entry or child destruction.

Root registration captures the prebound verified principal and trusted storage
class from the unpublished owned translation root. Lazy mailbox preparation
captures generation, principal and class under one root-table hold, then keeps
its existing root lease and exact publication revalidation. Existing class-only
consumers retain their shared capture adapter; they are not silently migrated.
Signed application construction cannot use unidentified funding. Permanent kernel
and trusted raw preparation/fixtures retain their explicit family-only boundary.
No role, manifest name, successor ASID or closing worker selects a sponsor.

Principal byte sponsors share the existing history lookup and funding mechanics,
with zero-sized type markers selecting buffer or namespace-account budgets.
No per-account principal registry or extra heap holder is introduced. The existing
history record gains one 16-byte budget vector, included in its actual admitted
node layout. Published history and duplicate candidates remain bounded by their
existing pool; at 4096 total nodes that field adds at most 64 KiB requested layout.

## Capacity and limit tradeoff

`Policy.memory.principal_namespace_account_bytes` defaults to **1 MiB** per signed
principal, shared across sibling roots, retained generations and weak references.
It is independent of the existing 16 MiB buffer allowance, so retained accounts
do not consume that allowance for inputs, diagnostics or IPC cleanup scratch.
Global namespace-account ceilings remain 24576/32768 ordinary/total bundles.
No existing logical resource count, global pool ceiling, heap reserve, protocol
maximum, SDK or signed bundle changes.

Native actual complete bundles measure **72/80/104/104 bytes** for capability,
endpoint, record and mailbox accounts. The private holder is 24 bytes. A root's
three mandatory accounts need 256 bytes; mailbox backing brings that to 360.
The 1 MiB ceiling allows 4096 three-account roots in isolation, or fewer when
mailbox/retained backing also consumes it. That is headroom, not a launch promise:
other admission dimensions and actual heap availability can reject earlier.
This intentionally bounds one principal below whole-node family capacity;
independent count ceilings are not a per-principal entitlement. Raising it would
permit more retained account memory; lowering it can reject
mandatory root or lazy-mailbox preparation. No exercised workload needs an
increase or a reduction in existing logical/protocol limits.

The largest fixed 104-byte bundle and global count ceilings imply a separate
**2.4375/3.25 MiB ordinary/total upper bound**, including partial holders and
weak/allocator retention. This applies only to the current four fixed account
constructors and is outside the 19 byte-denominated pool sum. The version-2
policy census records the selected principal ceilings; the 24-pool catalog and
205.5/154.125 MiB independent byte-ceiling sums remain unchanged. These values
are not observed usage or proof of simultaneous fit in the mapped 72 MiB heap.

Account references now occupy **32 rather than 16 bytes** on qualified targets.
Enclosing node admission uses its current layout, so those fields consume family
bytes; their general principal coverage remains separate. Native IPC
capability/endpoint/pending/reply layouts are 136/320/216/216 bytes. Their existing
conservative node-only mix needs 9584640/12779520 ordinary/total bytes, below
12/16 MiB before buffers, queues and allocator excess. Namespace nodes total 632
bytes per five-node root; authority and mailbox payload nodes now measure 80
bytes. The current references update these costs rather than retain old figures.
Native AMD DWARF independently identifies 24-byte holders, 32-byte references
and 80-byte authority nodes in the recorded qualified kernel.

The runtime namespace-pool sample still reports bundle counts, not principal byte
usage. No mutable budget surface or allocator bookkeeping/fairness guarantee is
added. Reuse of the common allocator does not accept Q4, Q5 or Q7.

## Controls and qualification

Host tests execute the production generic owning adapter with an explicit backing
allocator tracer. They check actual holder/payload layouts, aligned payloads,
weak-only retention, retained allocator clones, no revival of a spent payload
allowance, and holder/payload rejection with original charge disposal. Existing
concurrent final-weak, allocation failure, cancellation and guarded-abandonment
controls remain. The production sponsored adapter separately traces actual System
free before byte refund, without changing family counts; existing family/sponsor
growth and rejection controls use the same implementation. Host adapters do not
qualify kernel IRQs or artifact-signature verification.

Native fixtures bind private principals at the trusted constructor boundary;
combined security campaigns separately exercise verified signed launch. New
controls reconcile all four account layouts, sibling sharing, late promotion,
256 heap-held principal denials and an independent healthy sponsor. Three actual
root-registration prefixes reject with no account, capability-only, or
capability-plus-endpoint headroom, restoring principal bytes, family slots,
physical frames and root counts without publication.

All four accounts survive a real root close and same-slot generation reuse as
weak-only backing. Individual final weak drops return original principal bytes
and ordinary slots without changing the healthy successor. A separate fixture
allocates **14560 real capability-account bundles** beside its 256-byte root,
exactly reaching the default 1048576-byte ceiling while the global ordinary pool
still has slots. Disposal and root close recover exact original baselines.
Private headroom changes preserve every live charge; this is not sustained
concurrent EL0 saturation or essential-service fairness.

The existing complete unpublished-root abandonment fixture now uses a prebound
principal and retains **256 original account bytes** under history, lifecycle,
root/table, heap, physical and family guards. No ASID is published, and no new
intentional-retention object is added for this control. This qualifies that
complete parent; it does not qualify arbitrary account-reference destruction.

| Final frozen-source qualification | Result |
| --- | --- |
| Formatting, including standalone byte-allocator fixtures | Passed |
| Full host suite and namespace-account policy checks | Passed |
| Strict ARM/x86 test and minimal profiles | All four passed |
| Four-CPU ARM SMMUv3 security | 19/19, unchanged 240-second budget |
| Four-CPU Intel VT-d and AMD-Vi security | 17/17 each, unchanged 240-second budgets |
| Four-CPU ARM and Intel minimal operational boot | Both passed, unchanged 90-second budgets |

All five campaigns reuse the exact signed service-bundle identities qualified
by the preceding IPC-principal batch. Frozen source/input hashes remained
unchanged through qualification. No native failure or deadline increase occurred.
These results qualify the named allocation/retention paths and current default
policy; they establish no aggregate gate, production trust or physical-platform
isolation claim.

## Preserved preflights and limits

An initial host check compiled the module as a standalone crate without its
allocator feature/extern harness; the proper harness corrected the invocation.
The first native check caught a prototype replacing the shared class-only capture
result with the richer funding token. Existing device/completion consumers retain
the class adapter, and mailbox uses the exact principal-bearing capture. A later
check rejected unused test hooks before their controls were wired. Corrected
preflights and final qualification remain separate from those diagnostics.

The ARM display second-grant-probe, Intel retirement-pin and ARM exit-watch
incidents remain unresolved. Passing headless repeats do not explain or close
them. Other principal/account coverage, allocator overhead, broad caller/IRQ and
implicit-Drop review, physical recovery and sustained progress remain Q4–Q8 work.
Finding dispositions, aggregate gates and production trust are unchanged.
