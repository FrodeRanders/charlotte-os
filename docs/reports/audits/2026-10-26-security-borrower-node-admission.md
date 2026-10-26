# SEC-07: shared read-borrower node admission

Date: 2026-10-26. Reviewed baseline: `316404a5`.

Read-loan borrower records used raw prepared/map/retired nodes outside the
requested-byte/count budget already applied to memory-capability payload and
namespace nodes. Shared authority admission bounded live loans, but did not
charge this additional layout or its retained storage. This was a source-confirmed
R7-2/R7-3 gap; no hostile cross-domain exhaustion exploit is claimed here.

Borrower nodes now use that **same existing configurable pool**. Its defaults
remain **4 MiB / 32768 nodes total, 3 MiB / 24576 nodes ordinary**. No additional
pool, limit family or registry is introduced. Read-loan preparation reserves
all three nodes before source escrow or pin mutation. A native negative control
restoring the uncharged factory fails the new third-node admission assertion.

The [evidence](evidence/2026-10-26-security-borrower-node-admission.json) preserves
source/artifact hashes, the deliberate failure, final executions and relevant
retry diagnostics. SEC-07 and SEC-18 remain partial; the ledger stays
**29 corrected / 6 partial / 3 open** and production trust stays disabled.

## Admission, ownership and configuration

`cap_storage::Preparing` already captured trusted destination classification
against the exact namespace handle before allocation. It now retains that value
for the borrower factory too. Names, roles, source privilege or later destination
promotion cannot grant a different class. ASID reuse cannot redirect an original
reservation or its refund. Source data backing keeps its existing sponsor.

Each of the three layouts reserves its requested bytes, including its typed
charge and link, plus one node before the fallible allocator. On the qualified
native builds the payload, namespace and borrower layouts are **64, 64 and 56
bytes**, respectively. These are observed layouts, not new configurable limits.
Both byte and count gates apply; these actual-pressure runs reach the count
ceilings first. They do not demonstrate independent real-byte-ceiling exhaustion.

A preparation without a read loan needs two temporary nodes. A read loan adds a
borrower node, even when the destination already has a namespace; a write loan
keeps borrower state inline and still needs two. A later-node rejection disposes
its unused prefix and hidden destination authority after callback-local guards,
without changing source escrow, pins or loan state. Existing joint authority/
payload commit then relinks the same admitted nodes without allocating.

The original charge follows the borrower through the source's read map,
revocation's complete prior-state owner, detached borrower/authority pairs and
uncertain complete backing receipts. Confirmed disposal frees the actual node
before returning its original metadata charge; paired payload/authority cleanup
keeps its existing order. Mapping removal, invalidation, scratch completion,
root/pin and physical-release proofs are unchanged. Retention gives no recovery
custody or authority to replay cleanup.

Unused borrower cancellation still runs through the existing active containing
transfer cleanup. This change does not qualify arbitrary outer IPC/lifecycle/IRQ
or destructor contexts, remove masks, add roots/pins, or change a wire/register
ABI. Userspace and signed service inputs are unchanged.

The [Configuration guide](../../guides/configuration.md), [resource policy](../../reference/resource-policy.md),
[node-admission reference](../../reference/memory-capability-node-admission.md)
and manual document the shared `memory_capability_nodes` limits and the
three-node overlap requirement. Selection remains compile-time through
`CATTEN_RESOURCE_POLICY`; no boot-resource parser or live resizing is added.
Raising unified capability counts alone does not raise this independent pool.

## Regression controls

The serialized native controls exercise the production pool and loan paths,
without resetting counters:

- Actual allocated payload nodes fill the ordinary pool. Returning two slots
  permits write-loan preparation/cancellation, while read-loan preparation
  rejects at the third node. Exact metadata, shared authority and source backing
  baselines return. The source has no persistent transfer/delivery fence,
  copy pin or loan state and remains writable.
- A trusted destination admits a real read loan from the total reserve without
  consuming ordinary quota. Its read mapping is installed normally.
- Actual allocations then fill the total pool. Mapped ordinary-loan revocation
  succeeds at exhaustion. After destination promotion, it still returns the
  **original ordinary** borrower and capability-node charges. With those two
  slots available, another trusted read-loan preparation rejects and returns
  its exact prefix/authority baseline.
- Mapped trusted-loan revocation returns its two total reservations without
  refunding ordinary quota. All pressure nodes, live capabilities and fixture
  roots then close, restoring the exact initial pool baseline.
- **192 allocator-adapter rejections**, 64 for each layout, observe the real
  reservation before allocator entry and restore its exact baseline. These
  are controlled factory failures, not global allocator OOM.
- Promotion immediately after classification capture cannot reclassify any of
  the three reservations. Their original charges survive real root retirement
  and actual ASID reuse until ordinary cancellation.
- Existing **128 pre-escrow rejections, 64 unused cancellations, 64 heap-held
  two-reader relinks/detaches and 32 reader-preserving revocation/namespace
  closes** pass with the charged borrower type. Existing capability publication,
  returned-resource, mapping, DMA and root-retirement controls stay enabled.

There is no new terminal-retention fixture allocation. The shared production
retirement-node host suite passes **16 tests**, including allocation-lifetime
charge release and inert detached-node abandonment. The resource-policy harness
passes all **three tests** covering default/larger profiles, invalid selections
and checked counter behavior. No model input changes or new formal proof is
claimed; existing policy projections were not rerun for this layout extension.

## Native negative control

Only the borrower map/prepared/retired aliases and factory were temporarily
restored to their previous uncharged forms; all other final Rust and new controls
remained. The isolated four-LP AArch64 no-network guest stopped with
`reason: kernel-panic`:

```text
panicked at crates/catten/src/memory/object/cap_storage/admission/tests.rs:272:5:
read-loan bypassed third-node metadata admission
```

This checks the returned result before deliberately held-lock controls. The
old path admits the preparation with two slots and ordinary cancellation runs;
no deliberate deadlock or hardware replay is needed. It is an expected failing
regression control, distinct from passing final guests. The mutation is not a
complete historical-baseline execution. All seven final Rust files were restored
and verified byte for byte before the passing runs.

## Final validation and remaining work

Strict base-kernel Clippy passes on both custom targets with signed bundle inputs
and `--no-default-features --features acpi -- -D warnings`. Existing signed
userspace bundles are reused. Final fresh-storage, four-LP security/HTTP runs
execute sequentially; the evidence records each artifact, actual pressure count,
security generations and host probe result.

Each first final-source attempt finishes with `reason: complete`, no failed or
pending deferred tests, both security publication generations (1 and 2) passing,
and all fourteen HTTP probes passing:

| Backend | Deferred passed / failed / pending | HTTP checks passed |
| --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 |
| x86_64 / AMD-Vi | 18 / 0 / 0 | 14 |

Each host run records four `ConnectionResetError` retries: one after EOF,
one after abortive peer closes and two during the idle-client probe. Intel also
records one initial-metrics `TimeoutError`. Every affected probe subsequently
passes. Evidence preserves all thirteen diagnostics with their eventual results;
no timeout, retry bound or quota is widened. No unexpected final guest failed.
Passing results do not establish the cause of historical scheduler, IPC,
root-close, metrics or NIC failures.

The manual rebuild completes two passes. Affected PDF pages **51, 198 and 237**
are rendered and visually checked. The 249-page build still emits 189 overfull-box
warnings, with no undefined references; the full manual is not re-audited.
Workspace formatting, whitespace, frozen source hashes and local Markdown targets
pass.

This closes the borrower-layout admission gap within R7-2/R7-3 and C05/C18.
Nested allocations, object nodes, allocator overhead, aggregate principal/heap
admission, sustained concurrent hostile EL0 pressure and essential-service
fairness remain separate. Active containing cleanup and wider outer guard/IRQ
contexts remain G1 qualification work. No numbered finding is newly closed and
no physical retry owner, production trust boundary or universal RAM-fit guarantee
is supplied.
