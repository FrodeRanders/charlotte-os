# Word-ring backing admission and retained charges

This continues the [queue-node correction](2026-10-10-security-mailbox-queue-storage.md)
inside C02/C03/C18 and G1/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md). SEC-07 and
SEC-18 remain partial: **20 corrected, six partial and four open** findings,
eighteen owner families and seven acceptance gates. The existing family account,
node machinery, operation and final-root custody are extended; there is no new
registry, object-category row or independent backing retry.

## Defect and correction

Word-queue nodes already prepared outside local guards, but the application path
still used infallible concurrent-queue/Vec/Arc construction. A bounded number of
words per LP did not bound staged queue allocations across creators/domains or
reserve backing capacity for platform progress.

The word ABI now uses [`mailbox_words::Words`](../../../crates/catten/src/syscall/mailbox_words.rs):
one fixed 256-word ring per LP, in a single fallibly allocated Vec. Ring access
uses the existing short IRQ-state-preserving per-LP mutex; it leaves before IPI
notification. Ordinary word operations were already serialized under lifecycle,
so this introduces no custom unsafe concurrent transport. No queue Arc/Weak or
durable sender escapes. Trusted typed kernel shard transport retains its existing
separate implementation; the application word path has no compatibility branch
to its old infallible constructor. No throughput result is claimed.

[`mailbox_budget`](../../../crates/catten/src/syscall/mailbox_budget.rs) adds two
independent dimensions to the existing captured mailbox account:

| Scope | Prepared/live queue sets | Requested ring backing bytes |
| --- | ---: | ---: |
| One exact domain generation | 2 | 1 MiB |
| Node total | 1,024 | 8 MiB |
| Ordinary domains on the node | 768 | 6 MiB |

The remaining 256 sets / 2 MiB are a shared platform progress reserve. Both
dimensions must admit before queue-node/backing allocation. Classification comes
from the operation's original captured kernel designation, never a manifest role
or later ASID lookup; later promotion does not change existing charges. A
published queue and every competing preparation share this same exact account.
The set ceiling bounds concurrent preparation; it does not create a second
published queue for one domain.

Legacy word callers also use the existing family account even when they have no
endpoint grants. [`Operation`](../../../crates/catten/src/syscall/mailbox_queue.rs)
prepares any missing admitted namespace/account outside local guards, revalidates
its captured root before relinking, and retains losing namespace storage until
post-guard completion. Backing rejection may leave that empty namespace owned by
the root, with no queue charge or published word queue. Competing creators and
legacy/capability callers cannot obtain separate accounts for one generation.
Account retirement rejects admission even when presented with platform class.

LP count is captured once for each preparation. Checked size multiplication
rejects overflow; `Vec::try_reserve_exact` reports allocation failure, and excess
capacity rejects before publication. No ring push/pop grows storage. The
original charge remains in the complete preparation until construction finishes,
then follows the ring owner. Its declared field order deallocates ring backing
before dropping that charge. Ordinary unused/rejected fields dispose outside
local guards, completing the exact root last. Namespace nodes/account control
blocks and allocator overhead are excluded from this requested-byte measurement.

Final root detachment and existing bounded custody retain the same backing
charge through invalidation rejection. Confirmed metadata release deallocates
rings before refund, then the existing one-shot root walk/slot completion runs.
Terminal operation/root abandonment retains the complete queue, original account,
classification and charge behind existing retaining owners, without locks,
allocator work, hardware or logging in fallback. No abandoned backing is
re-adopted; tickets still convey no independent queue authority.

## Selected execution evidence

The [actual operation fixtures](../../../crates/catten/src/syscall/mailbox_queue_tests.rs)
extend the existing queue tests:

- Backing rejection after account admission and node preparation returns the
  original word, publishes no queue, and restores captured domain/node queue
  counts and bytes. The empty family namespace remains owned until root close.
  Checked layout overflow and actual Vec capacity-overflow rejection return
  errors without huge allocations; these do not emulate real memory exhaustion.
- Two prepared creators share **two sets / twice the measured backing bytes**.
  A third preparation rejects at the exact generation ceiling without changing
  those charges. Publication under the heap guard preserves the first queue;
  post-guard loser completion leaves one live set/charge. Established delivery
  consumes no new storage or admission.
- Existing two rounds of **256 ordered words**, full rejection of the 257th,
  FIFO drain/wrap and syscall legacy/capability smoke checks now execute fixed
  rings. No message or status ABI changes.
- Actual final metadata detachment under lifecycle/heap guards keeps one set
  charged. Explicit post-guard release returns its account/node values to the
  baseline. A retired account rejects platform admission. Successor/staged
  operations retain their own account; old refunds cannot affect it.
- Isolated production node counters separately exercise byte and set ceilings,
  ordinary exhaustion, platform reserve and total exhaustion. They neither
  allocate megabytes nor exhaust live shared counters.
- Guarded operation abandonment keeps the previously retained exact root/node/
  queue and its new backing charge unchanged. Promotion after preparation cannot change
  its captured ordinary classification. It remains unpublished and closing
  remains `Pending`.

The [real root custody fixtures](../../../crates/catten/src/memory/retirement/recovery/tests.rs)
now also capture the exact queue account before detachment. Failed invalidation
and guarded actual-attempt abandonment retain **one set and its requested bytes**.
Confirmed retry refunds before physical callbacks; a reused ASID's charge stays
independent. The existing partial-physical-rejection fixture requires queue
charges zero before its terminal owning walk. This adds charges to already
retained owners, **no additional retained root, queue, user heap/image or IOMMU
data frame, or physical quarantine episode**.

Selected phase diagnostics, identical across the three targets:

```text
[mailbox queue backing] 8288 requested bytes per 4-LP queue set; backing rejection refunds, shared per-generation ceiling rejects third preparation, final detach retains charge until release, successor and guarded abandonment preserve original ordinary classification
[mailbox queue phases] 8 preparation-entry and 16 completion-entry boundaries outside local lifecycle/mailbox/queue/table/physical/heap/capability guards; entry IRQ state preserved
```

The requested layout is **2,072 bytes per LP**, including fixed words, ring
indices and its mutex, on both tested 64-bit targets. Entry counts are scoped
operation probes, not a global allocation balance. Existing root release scopes
still report 2/2 and then 1/1 mailbox/authority boundaries. Guard probes preserve
entry IRQ state; they do not qualify every enclosing caller. Serialized creator
ordering/counter saturation is not concurrent cross-LP pressure or real OOM.

## Validation

Fresh-storage guests run sequentially, reusing validated embedded bundles because
only kernel code changed:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance mailbox-backing-intel-20261010 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance mailbox-backing-amd-20261010 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18809 CATTEN_DEPLOY_HOST_PORT=18109 scripts/run-aarch64.sh --security-test --instance mailbox-backing-arm-20261010 --fresh-storage --timeout 180
```

| Guest | Final result | Queue preparation / completion entries |
| --- | --- | ---: |
| Intel VT-d | 15 passed, zero failed/pending; `0x1bbff` | 8 / 16 |
| AMD-Vi | 15 passed, zero failed/pending; `0x1bbff` | 8 / 16 |
| Arm SMMUv3 security guest | 19 passed, zero failed/pending; `0x2003ffff` | 8 / 16 |

Arm scoped checks pass `0xffff` at publication generations 1 and 2.
Kernel SHA-256:

- Intel/AMD x86: `fc5f0499d758915ed9e72b7362699a623f3cb81b002907469509e4975268dba5`.
- Arm security guest: `bbce69844b297a81b0063743172a38c58c3a412c739c373bc349d407396da724`.

All three guests passed on their first artifact. Backing/quota/retirement
rejections are expected fixture outcomes, not failed guest runs. Strict locked
default-feature Clippy passes on both architectures (`-D warnings`), as do
formatting and diff checks. Runners verify native assembly section/load
permissions (249 x86 entries, one Arm entry). Shared admitted-node and generic
budget source/host fixtures are unchanged; the earlier eight-test/full-host
harness evidence remains in the
[namespace report](2026-10-09-security-capability-namespace-storage.md). That
harness is not rerun in this kernel-only pass. Actual word-ring backing/queue,
record quota, syscall ABI and root custody fixtures execute on all three targets.

## Remaining boundary

Requested word-ring byte/set ceilings do not cover namespace/account nodes,
allocator overhead, other typed kernel transports, aggregate principal/heap
budgets or bandwidth. Metadata allocation rejection outside local guards and
the existing complete-owner fallbacks are qualified; every wider context,
panic/interruption and genuine allocator exhaustion still need qualification.
Completion/scratch/accounting teardown, other authority preparation/retirement
and active token fallback remain open. Existing root diagnostics still omit
queue bytes/family charges. No authenticated operator policy or supervisor
reconciliation is added. Historical intermittent Intel/AMD user-stack lease
timeouts remain causally unresolved; passing guests do not close C16/G1/G2/G7.
The [current ledger](../../reference/security-remediation.md) retains separate
failure/repeat evidence and unchanged SEC-07/18 acceptance criteria.
