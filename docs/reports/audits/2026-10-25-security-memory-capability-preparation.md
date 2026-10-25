# Security audit: memory capability preparation ownership

Date: 2026-10-25. Reviewed baseline: `4f8153cd`.

Memory payload-capability preparation now retains unused payload/namespace nodes,
its exact authority/account reference and captured generation in inert fields
before factories. Ordinary success/rejection explicitly consumes unused metadata
after local guards. This corrects automatic allocator/registry/Arc destruction
from the bare preparation's fallback; containing transfer/backing destructors
remain separately qualified contexts.

The [tracked evidence](evidence/2026-10-25-security-memory-capability-preparation.json)
records source/model/artifact hashes, controls, the failed initial Arm run and
passing executions separately. SEC-07/18 and the ledger remain **29 corrected /
6 partial / 3 open**. No new capacity limit, root lease, registry, recovery family,
custody or physical/replay authority is introduced.

## Owner and ordinary completion

The old preparation used ordinary fields; constructor and caller early returns
implicitly freed nodes and cancelled a shared reservation. The new owner exists
before the first factory. Partial construction rejection explicitly finishes its
complete prefix. A consuming action helper returns the same owner on success;
error cancels only after action-local guards and unused borrower storage leave.
Direct allocation finishes after publication/unlock. Move/read/write-loan and
copy preparation hand the same owner into the existing `PreparedTransfer`.

Containing transfer cleanup explicitly frees unused borrower storage and consumes
its optional destination after the existing source/escrow/pin work. The destination
frees unused payload/namespace nodes before cancelling authority or releasing its
captured account reference. No fabricated empty reservation, scalar re-adoption,
compatibility allocator or new public ABI/wire format is added.

Bare partial, complete and published preparation abandonment performs no allocator,
memory/unified-capability registry or account-Arc destruction. This does **not**
make the containing `PreparedTransfer`, source escrow, scoped pin or private-copy
backing destructor inert. Their existing outer IPC/IRQ/physical cleanup policy
remains G1 work; local post-guard completion does not qualify every caller.

Namespace teardown may consume its hidden staged authority while unused payload
nodes remain retained. The preparation is neither a root lease nor a retry owner.
Late explicit finish uses the original account and cannot cancel a successor's
authority after actual ASID reuse. Retained payload metadata has no separate byte
admission; authority counts do not bound that allocation lifetime after teardown.
No new aggregate metadata/principal enforcement is claimed.

## Controls and retention

Serialized kernel controls exercise:

- Existing **192 constructor-prefix rejections**, with original shared admission
  and exact count/frame baselines restored. Publication/retirement and stale
  captured-namespace controls remain enabled with explicit ordinary finish.
- **64 explicit storage-before-authority observations**: both unused node fields
  are consumed, guards are available, and the original authority reservation is
  still present/charged before final cancellation.
- **64 action rejections** returning with memory/table/heap/frame guards live
  inside the action; consuming error cleanup follows their release and preserves
  entry IRQ state and original admission baseline.
- **192 containing rejections**: invalid source move, rejected read-borrower
  factory and copy backing rejection. Original source data remains writable;
  no copy pin, destination authority, new backing charge or frame loss remains.
- Real root retirement/ASID reuse with late explicit old-owner finish leaves the
  successor's actual allocation/authority usable and its count unchanged.
- Partial, full and two published preparations are dropped while memory, table,
  heap, physical allocator and `CAPABILITIES` guards are held. Entry IRQ state
  remains unchanged. Published payload/namespace records retire explicitly.

The guarded control deliberately retains **four unused metadata nodes**: two
payload candidates and two namespace candidates, plus the four preparations'
captured authority/account references. Two staged authority records remain until
ordinary fixture-root teardown; its namespace drains them through the existing
policy and returns authority counters to baseline. A successor closes normally.
No user-data/root/pin quarantine is added. These selected allocations are not a
total-retention measurement, metadata-byte admission or recovery receipt.

Strict base-kernel Clippy passes for AArch64 and x86_64. Workspace formatting,
whitespace and local references pass. The kernel-coupled path runs directly;
no alternate host implementation or capacity-policy change is introduced.

## Preserved initial Arm failure

The new controls pass, then the initial SMMUv3 guest panics in the existing
scheduled first-entry exit-watch fixture. The runner exits **1**, with a four-LP
QMP/debugger capture and no complete suite/HTTP result. Last coordinator state:

```text
SELFTEST WAITING: passed=12 pending=8 passed_bitmap=0x2a3fd pending_bitmap=0x20215c02
panicked at crates/catten/src/cpu/scheduler/deferred.rs:176:9:
assertion failed: completion::wait_timeout(asid, watched, 5_000).unwrap()
```

The last timestamp before panic is **16.468904 s**; LLDB resolves the stack to
`deferred::test_first_entry` → `completion::exit_tests::test_scheduled_cleanup` →
`self_test::cq_wait::cq_driver` → kernel-thread trampoline. Captured Arm kernel:
`c19003c7054ba35ec7948b582091b82a577d78c2e37058a189f0b432792aa755`.

The snapshot observes LP1 in panic, LP2/3 idle and LP0 at an exception vector.
Its independent scheduler atomics record current TID/generation/ASID tuples
`20/273/125`, `37/333/0`, `3/4/0`, `1/2/0`; these are diagnostic observations, not
transactional snapshots or quiescence proof. The awaited loop target identity,
watch ID and first-dispatch globals were not captured. The failure therefore
cannot distinguish dispatch, retirement, observer or scheduling-delay causes,
or establish a relationship to this memory change or historical failures.

The added **failure-only fixture diagnostic** preserves the 5000 ms wait and
assertion. On future false/rejected wait outcomes it records the exact awaited
LP/TID/generation/watch, typed wait result, dispatch arm/delivery counters and
copied live/staged observations after their local guards leave. A busy/absent
table, wake or worker marker grants no cleanup/replay authority. No new polling,
root completion, deadline increase or physical retry is added. The original
failure remains separate; passing repeats cannot attribute it.

## Native executions and formal scope

The initial pre-diagnostic Intel run passes **18/0/0** deferred tests and 14
HTTP checks. Final frozen source, including the failure-only diagnostic, passes
on all three backends: **56 deferred tests / 42 HTTP checks**. Together the four
successful executions provide **74 deferred / 56 HTTP checks**, separate from
the failed initial Arm run. Both security-service generations pass in each
successful execution:

| Source / backend | Deferred passed/failed/pending | Healthy beside stalled peers | Cancellation requests |
| --- | --- | --- | --- |
| before_failure_diagnostic / intel-vtd | 18/0/0 | 0.224–0.352 s | 17780 |
| final / smmuv3 | 20/0/0 | 0.265–0.378 s | 19588 |
| final / intel-vtd | 18/0/0 | 0.288–0.360 s | 17980 |
| final / amd-vi | 18/0/0 | 0.268–0.395 s | 18088 |

The original first-entry episode does not recur after instrumentation. The new
failure-only output branch is therefore compiled/source-qualified but not
exercised by these passing executions; its presence proves no failure cause.

Initial host metrics observations remain separate from guest completion:

- **before_failure_diagnostic / intel-vtd:** PASS with no preceding retry diagnostic.
- **final / smmuv3:** PASS with no preceding retry diagnostic.
- **final / intel-vtd:** PASS with no preceding retry diagnostic.
- **final / amd-vi:** PASS with no preceding retry diagnostic.

All host transport retry contexts remain inline in the evidence. Any timeout
cause remains unestablished; passing retries supply no historical attribution
or physical/replay authority.

Bundled services are reused; userspace sources and runtime ABI/wire formats are
unchanged. Native assembly-section checks remain enabled. Living Markdown and
LaTeX sources describe the ownership scope; the tracked manual PDF is unchanged
in this audit batch. No admission ceiling or deadline is widened to obtain a pass.

All **23 focused harness cases** pass: three safe configurations and **20 expected
counterexamples**. DMA operation explores **29175 distinct / 62003 generated
states**, depth **24**; IPC transaction explores **26 / 71**, depth **11**; root
retirement explores **2978 / 3733**, depth **33**. Required safe actions execute;
negative configurations violate their named invariants through the intended
unsafe action.

No model inputs change. Root original-charge, IPC loan/rollback/timeout and DMA
ownership projections remain; concrete constructors, Arc destruction, local guard
exit and generation-exact cancellation are native/source qualification. Models
do not reconstruct the first-entry failure or prove executable refinement,
arbitrary destructor contexts, concurrent pressure or essential-service fairness.

This advances C18/G1 and R7-2. Broader transfer/backing cleanup, payload metadata/
aggregate principal admission, allocator-interruption qualification and sustained
concurrent EL0 progress remain open. The first-entry timeout and historical
metrics/root-close/mapping failures remain unattributed; no finding closes.
