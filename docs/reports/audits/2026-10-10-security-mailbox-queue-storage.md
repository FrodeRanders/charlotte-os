# Word-queue namespace storage and exact-root operations

This continues the [final-root metadata correction](2026-10-10-security-root-metadata-retirement.md)
within C02/C03/C18 and G1/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md). SEC-07 and
SEC-18 remain partial: **20 corrected, six partial and four open** findings,
eighteen owner families and seven acceptance gates. Existing admitted-node
machinery and complete root receipts are extended; there is no new registry,
ticket, generic queue retry or independent cleanup controller.

## Defect and correction

Legacy and capability word send/receive shared a numeric-ASID `BTreeMap` of
bounded queues. First send allocated both its map node and queue backing under
the exclusive registry guard. Final close retained queue payload backing in the
root receipt, but `BTreeMap::remove` still deallocated the registry node under
lifecycle/queue serialization. Queue entries had no captured generation, and
word operations did not retain the exact root or check retired sponsorship and
staged closing before registry access.

[`mailbox_queue::Operation`](../../../crates/catten/src/syscall/mailbox_queue.rs)
now owns the captured identity, exact `AddressSpaceOperation`, fallibly prepared
namespace node and unused queue backing together. Root admission precedes queue
registry access. First send prepares storage outside local lifecycle/mailbox
guards, then revalidates generation, retired sponsorship and closing state
before relinking into the shared `AdmittedMap`. Queue entries store the captured
handle; it is never refreshed by numeric-ASID lookup. Established queues retain
shared registry borrows and the existing concurrent transport without fresh
storage or a global exclusive queue lock. Both send and receive now take the
short lifecycle validation hold; no throughput result is claimed.

A competing creator uses the existing exact queue and retains its unused
preparation through post-guard completion. Ordinary rejection explicitly disposes
unused fields after local guards leave, completing the root last. Its
`ManuallyDrop` fallback retains every field, including an unused node/queue and
the root lease, without locks, hardware, allocation, counter work or logging.
Abandonment is terminal retention, not retry custody.

[`RetiredMailboxes`](../../../crates/catten/src/syscall/mailbox_retirement.rs)
preflights both namespace generations before mutation and detaches the complete
queue node without allocation or destruction. That node joins the existing
closing transaction and detached root alongside payload and unified authority.
Failed final invalidation retains the same complete owner. Confirmed invalidation
explicitly releases the queue node and backing outside local guards before root
physical release/slot completion. Existing root custody and terminal physical
rejection rules remain intact. Serialized raw boot fixtures retain their `None`
identity convention; real user traps supply a live root. Their scalar teardown
adapter explicitly releases detached nodes after the local registry guard.

**Queue backing construction remains infallible.** The locked dependency
version, concurrent-queue 2.5.0, has no fallible bounded constructor; the shard
factory also allocates Vec/Arc backing. These allocations now run outside the
local guards, but this does not establish recoverable OOM, an aggregate byte
ceiling, principal budgeting or protected progress. Only namespace-node allocation
is made fallible here. Record-count budgets do not charge legacy words or their
queue backing. No custom unsafe queue or allocator compatibility branch is added.

## Selected execution evidence

The [actual queue fixtures](../../../crates/catten/src/syscall/mailbox_queue_tests.rs)
run alongside existing syscall/quota/generation and final-root custody fixtures:

- Invalid target rejection leaves node-failure injection pending. Injected
  node-preparation failure returns the original word and publishes no namespace.
  A subsequent established send succeeds without consuming another injected
  preparation rejection.
- Two exact-root creators prepare separately. Publication while the heap guard
  is held succeeds for both, preserving the first queue and FIFO words 11, 12
  and 13. The losing creator's unused node/backing is released after guards.
- Two rounds each enqueue **256 ordered words**, reject the 257th with its
  original value, drain in order and observe empty state. This checks bounded
  backpressure and wrap without growing queue storage.
- Actual final mailbox metadata detachment succeeds while lifecycle and the
  heap allocator are held, leaving no visible queue. Explicit release occurs
  after both guards leave. Retired sponsorship rejects send and receive.
- A successor reuses the numeric ASID with a different generation. Captured old
  send/receive reject and cannot consume its queued word. An older operation
  retained across staged close cannot publish or pop; new operation admission
  also rejects. Ordinary completion releases that exact lease and the same
  closing owner finishes.
- Guarded complete-operation abandonment holds lifecycle, both mailbox
  registries, CPU root/kernel tables and physical/heap allocators. It retains
  **one additional root lease, its CPU table backing, one unused admitted node
  and one 256-word queue per LP**. No queue is published, record charges or data
  frames are added. Staged root close remains `Pending`; fallback did not release
  the root. These retained fields are not automatically recoverable.

Existing real custody fixtures continue to check queued content after failed
invalidation, successful retry, stale root extraction, guarded actual-attempt
abandonment and terminal partial physical rejection. Their retained queue owner
now includes its original admitted node. This change adds no further physical
quarantine episode or new image/IOMMU backing.

Selected phase diagnostics, identical across all three targets:

```text
[mailbox queue phases] 6 preparation-entry and 14 completion-entry boundaries outside local lifecycle/mailbox/queue/table/physical/heap/capability guards; entry IRQ state preserved
[mailbox queue ownership] node rejection, competing creators, 256-word FIFO/backpressure and wrap, heap-held publication/final detach, retired/staged/stale-root rejection; guarded abandonment retains one exact root and its unused queue/node
```

The counts cover the instrumented operation scope, not global allocation balance.
Completion entries include operations that allocate no queue. Existing root
metadata release scopes still report 2/2 and then 1/1 mailbox/authority entries.
Availability probes preserve the entry IRQ state and check local guards;
heap-held publication/detach check allocation/destruction-free relinking.
Serialized competing creators are not simultaneous cross-LP contention. These
fixtures do not establish real OOM, panic-unwind behavior, performance or every
outer syscall/caller context.

## Validation

Fresh-storage guests run sequentially, reusing validated embedded service bundles
because only kernel code changed:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance mailbox-queue-intel-20261010 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance mailbox-queue-amd-20261010 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18799 CATTEN_DEPLOY_HOST_PORT=18099 scripts/run-aarch64.sh --security-test --instance mailbox-queue-arm-20261010 --fresh-storage --timeout 180
```

| Guest | Final result | Queue preparation / completion entries |
| --- | --- | ---: |
| Intel VT-d | 15 passed, zero failed/pending; `0x1bbff` | 6 / 14 |
| AMD-Vi | 15 passed, zero failed/pending; `0x1bbff` | 6 / 14 |
| Arm SMMUv3 security guest | 19 passed, zero failed/pending; `0x2003ffff` | 6 / 14 |

Arm scoped checks also pass `0xffff` at publication generations 1 and 2.
Kernel SHA-256:

- Intel/AMD x86: `4a42a23fc53406cf81679ce5441f5b75bed6346a816579ad30e4787ddd302125`.
- Arm security guest: `2f60b10f78d7d4992f71bcd69dcae40e8b8f5b9fe03a1db2526cf9cb2279f268`.

All three guests passed on the first artifact. Injected node/retirement rejection
is expected fixture behavior, not a failed guest run. Strict locked default-feature
Clippy passes for both architectures (`-D warnings`), as do formatting and diff
checks. Runners verify native assembly section/load permissions (249 x86 entries,
one Arm entry). The shared admitted-node implementation/host fixtures are
unchanged; their previous eight-test/full-host-harness evidence remains in the
[namespace report](2026-10-09-security-capability-namespace-storage.md). That
harness is not rerun in this kernel-only pass. Actual queue, mailbox admission,
root custody and existing transport tests execute in all three guests.

## Remaining boundary

Queue backing OOM/byte/principal/progress admission, completion/scratch/accounting
teardown, other authority preparation/retirement and active token fallback,
broader heap budgets and enclosing caller contexts remain open. This operation
starts before queue access; it does not extend the separate capability-endpoint
lookup/close serialization contract. Root recovery diagnostics still omit queue
bytes and metadata record charges. No authenticated operator policy or cached
supervisor reconciliation is added. Historical intermittent Intel/AMD user-stack
lease timeouts remain causally unresolved; passing guests do not close
C16/G1/G2/G7. The [current ledger](../../reference/security-remediation.md)
preserves failed runs separately from passing repeats and the SEC-07/18 criteria.
