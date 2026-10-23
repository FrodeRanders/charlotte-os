# Security audit: shared admission for thread metadata

Date: 2026-10-23. Reviewed baseline: `738d9064`.

Thread creation now reserves **requested heap bytes and complete bundle counts**
for its stable context and preallocated retirement node. Instances and restarted
generations of a verified signed principal share **1 MiB / 128 bundles**. The node
admits **8 MiB / 4,096 bundles**, with ordinary work limited to **6 MiB / 3,072**.
This closes the previously unbounded context/node allocation path within SEC-07
R7-2/R7-3. Master-table capacity, observer storage and other kernel metadata remain
outside this pool.

The [tracked evidence](evidence/2026-10-23-security-thread-metadata.json) records
source/model/artifact hashes, selected diagnostics and results. The audit ledger
remains **29 corrected / 6 partial / 3 open**. SEC-07 and SEC-18 remain partial;
production trust remains disabled. This extends the
[table/stack batch](2026-10-23-security-principal-structure.md), preserving its
historical failures and limitations.

## Ownership and enforcement

`reserve_thread_metadata` validates the captured root generation and live memory
account under one table/ledger hold. It captures the installed signed principal
and trusted classification there. Signed launch installs identity before thread
construction. All instances reuse the existing principal-history record; there
is no new principal registry or heap-allocated sponsor. Kernel and trusted raw
unsigned fixture work consume node admission without an invented application
principal.

Admission precedes either allocator, generation claim, physical stack work and
scheduler publication. Both byte/count dimensions must fit. Denial returns
`ThreadPreparationFailed`. Node/context allocation rejection disposes unused
storage before releasing the reservation. Existing explicit unstarted cleanup
still needs confirmed stack-pair release before consuming constructed metadata.

The linear charge lives in the same prepared retirement node that accompanies
the thread. The context remains an ordinary stable-address Box; assembly's
context-pointer contract is unchanged. The shared collection gains a typed
charge parameter with a zero-sized default, preserving uncharged node layouts.
Staging, requeue and delayed reaping move admitted storage without allocation.

A failed or abandoned thread retains context, node, exact root/stack owners and
original charges. Explicit successful metadata disposal destroys the typed
payload/context and actually deallocates its node before refund. An interrupted
disposal retains its node and charge. Captured principal/classification cannot
be replaced by later promotion, source-root retirement or ASID reuse.

This is the existing C16 family. Its [cleanup row](../../reference/cleanup-recovery.md)
and [lock order](../../reference/locking.md) are updated. The change adds no
root/pin, physical proof, recovery custody, retry permission, wire format or ABI.
Counter reservation/refund allocates nothing; published fallback enters no
accounting or allocator lock. Wider constructor/destructor IRQ/guard contexts
retain their existing qualification limits. See the
[current contract](../../reference/thread-metadata-admission.md).

## Verification

Serialized native controls use real roots, context Boxes and retirement nodes:

- Two roots of one principal fill a private two-bundle ceiling. **256 denied
  thread creations** reject before either allocator, generation claim or physical
  stack work. Another principal creates and explicitly releases a thread.
- Independent small byte/count ceilings reject separately. Trusted promotion
  supplies no principal headroom and does not reclassify an existing charge.
- A prepared node retains its original sponsor after source-root destruction and
  ASID reuse by another principal. Stale-generation admission rejects; late
  release leaves the replacement principal's live charge unchanged.
- **3,072 real prepared nodes** exhaust ordinary admission on each architecture.
  One real kernel thread still constructs and releases from the reserve. Actual
  node deallocation restores the complete original counters; a new ordinary
  thread then constructs and releases successfully.
- A complete signed thread is abandoned under history, metadata-pool, root,
  heap and physical allocator guards. No lock entry or frame/charge refund occurs;
  its original root still rejects close with `OperationsInFlight`.
- Existing node/context allocation fault controls restore metadata counters;
  staged same-LP/off-stack reaping and explicit pair release restore their
  original metadata admission. Existing failed-pair retention remains terminal.

Fourteen host tests exercise the production retirement collection. Allocation
tracing checks payload disposal and node deallocation before charge release,
ordinary unused/failing preparation, allocation-free relinking and retained
charges after entry/list abandonment or interrupted disposal. Host unwind is
an interruption adapter, not a kernel panic-recovery mechanism.

Final-layout AArch64 passes **20 / 0 / 0**. The isolated x86 same-kernel
repeat passes **18 / 0 / 0**. Each passing run completes two scoped security
generations and all **14 host HTTP controls**. Healthy replies beside stalled
peers complete in **0.441–0.600 s** on AArch64 and **0.315–0.402 s** on x86.
The final requested metadata layouts are **656 bytes per bundle** on AArch64
and **1,184 bytes** on x86. No policy ceiling or fixture deadline was widened.

The first two native runs also passed, before the final generic-layout adjustment.
They used an optional charge field; the final implementation uses uninitialized
storage only during fallible preparation and writes admission before exposing
an owner. This keeps the default zero-sized charge from enlarging other families'
nodes. Final runs above exercise that source version. The x86 failure below is retained
separately. HTTP probes recorded bounded
connection-reset retries within the existing host control policy; they do not
establish a cause for earlier unresolved IPC or release failures.

## Captured x86 failure

The first final-layout x86 run panicked after the new metadata controls passed:

```text
panicked at crates/catten/src/ipc/cancellation/tests.rs:121:79:
called `Result::unwrap()` on an `Err` value: UnmapFailed
```

The automatic capture completed QMP, all four CPU stacks/registers and LLDB
without reported capture errors. The failing scheduled endpoint fixture was
`Fixture::enqueue(caller=(id=122, generation=14), server=(id=140, generation=2),
endpoint=1, count=2)`, called by `run_endpoint_runtime`. It maps a queued loan;
these fixture roots have no application threads. Another LP was interrupted by
its timer while in root-retirement `try_send_sync_shootdown`; two LPs were idle.
The last result snapshot was **12 passed / 6 pending**. No final full-suite or
scoped-security success is claimed for this run.

`map_any_with_lease` can return this error after an unconfirmed invalidation or
failed mapping rollback. The public result loses the exact invalidation cause,
and the capture cannot identify which path produced it. The simultaneous LP
stack is an observation, not proof of a coordinator owner, missed acknowledgement,
timeout or causal relationship to metadata admission. No deadline, cleanup fence,
retry policy or physical-release gate was changed in response.

One isolated repeat reuses the exact failed kernel ELF, SHA-256
`c4b1932e8fd849076c41a1dd911a799a570c9c507d139eb435c9676540c4e92e`,
with fresh storage and the same service-bundle inputs. It runs after the formal
suite finishes and passes the complete x86 suite and HTTP controls. This changes
the workload overlap; it is not a causal control and does not clear the failed
execution. The generated, unused new security-fixture directory does not alter
the reused ELF's embedded fixtures. The failure and passing repeat remain
separate evidence records. Exact invalidation/map diagnostics and reproduction
under controlled scheduling/host load remain follow-up work.

## Formal checks

The new `CharlotteThreadMetadata` projection checks unequal prepaid bundles,
separate construction/disposal/free/refund steps, byte/count limits, abandonment
and original sponsorship across root reuse. Its safe graph has **4,132 distinct /
12,559 generated states**, depth **15**, with every required action exercised.
Four expected counterexamples admit by generation, refund before node free,
refund abandonment or bill a replacement principal.

The first formal invocation completed the safe graph but the harness rejected
missing action coverage: `error: required action AllocateNode had no TLC coverage
in CharlotteThreadMetadata`. A shared action helper hid the individual labels.
Inlining those transitions exposes their coverage; no invariant was weakened.
The failed harness result is retained separately in the evidence.

The complete harness passes **46 safe configurations and 137 expected
counterexamples across 43 modules: 183 cases and 227 checked inputs**. Strict
base-kernel Clippy passes on AArch64 and x86; workspace formatting and whitespace
checks pass. No userspace service source changed, and the embedded bundles were
reused. The checked-in manual PDF is preserved; Markdown and LaTeX sources are
updated together.

## Remaining scope

The pool measures two requested layouts, including the charge stored in each
node. It does not count allocator overhead/rounding, physical heap-arena backing,
master-table retained vector capacity, exit-observer/weak-reference storage,
waiter/timer/control blocks, root/principal-history metadata, or other resource
families. Thread exit does not refund the master table's capacity. Principal
history remains boot-lifetime and still needs its own total admission.

The pressure controls are serialized boot fixtures with never-dispatched user
contexts, not concurrent hostile EL0 workloads. They exercise ordinary count
exhaustion and one kernel-thread admission; they do not force every total-node
limit, prove a full RAM ledger or measure sustained management/timer/teardown
fairness. The finite model treats stack-release proof as an input and omits node
reserve, hardware, allocator failure, observer storage, IRQ contexts and progress.
It is not an executable refinement proof. These remaining R7-1/R7-2/R7-3/R7-4/R7-5
and SEC-18 boundaries are substantive work. Historical unresolved failures remain
qualified in their dated reports; these passes supply no retrospective cause.
