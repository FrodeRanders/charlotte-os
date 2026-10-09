# Explicit published stack-pair retirement

This continues the [stack preparation abandonment correction](2026-10-09-security-stack-preparation-abandonment.md)
within C16/G1/G2/G7 of the [cleanup/recovery strategy](../../reference/cleanup-recovery.md).
The audit ledger remains **20 corrected, six partial and four open**. This is
scoped SEC-18 progress; it does not close the finding or its QEMU milestone.

## Ownership boundary

`Stacks` no longer has a physical cleanup destructor. Implicit architecture
context/stack/slot/charge destruction retains captured backing and admission;
it performs no invalidation, allocation release, root/pool lookup or logging.
General `Thread::drop` metadata/callback fallback remains a separate boundary.

Explicit pair release borrows the existing complete owner. It arms a one-shot
phase fence before detachment or physical adapters. The user half retains its
original `StackSlot`, exact root operation, bitmap slot and maximum user-plus-
kernel charge while its bounded inline frame batch is detached and invalidated.
The kernel half uses the existing post-arena/table `RetiredKernelRange` release.
Only confirmed completion of both physical halves permits admission completion.
The kernel-only charge follows the same rule. Growth rejects a retired pair.

Physical rejection/interruption is terminal here, including invalidation
rejection: a containing pair does not retain a complete retry receipt. Addresses
of already released or quarantined backing are never revisited. A kernel base
left in a failed owner is diagnostic information, not permission to re-adopt
that range. Lower-level receipts' retry contracts do not authorize pair retry.
Admission-completion errors are reported separately; confirmed physical cleanup
may already have cleared its bitmap/refunded its charge before operation-count
completion rejects. That does not authorize another physical attempt.

The reaper keeps its existing admitted node and entire thread/context across
explicit cleanup. Success permits node destruction. Failure reinserts the same
terminal owner; later scans leave it in place without callbacks or physical
retry. It remains visible to staged-generation/domain quiescence checks. No new
registry, snapshot allocation, shared custody API or operator policy is added.
Abandoned `ReapBatch`/node owners retain their existing nodes/transition fence.

Metadata notification happens once before stack release, while the original
root lease still prevents numeric-ASID reuse. Success does not run those
callbacks a second time from `Thread::drop`. Arbitrary callback internals and
implicit metadata deallocation still need their own qualification.

## Context and ordinary rejection

Both architectures now use pinned owner-LP reaper workers. Production reaping
rejects masked entry before claiming nodes, and retains executing-stack or Arm
`on_cpu` contexts. Arm's post-switch/IRQ-tail path still stages owner-side aborts
but no longer physically reaps before restoring its caller's interrupt state.
Fresh scheduled workers provide IRQ-enabled execution. No path enables IRQs
under an unknown outer mask.

Thread preparation admits the stable context allocation as `Box<MaybeUninit<_>>`
before generation claim or stack backing. After complete construction it writes
the context once and adopts the initialized allocation. Allocation failure can
no longer implicitly destroy a newly backed context. The assembly's stable Box
address and architecture ownership handshake remain intact.

Stack constructor rollback, rejected thread publication and fallible scheduler/
syscall submission explicitly release never-admitted stacks. Publication carries
the rejected complete payload outside lifecycle/publication/thread-table guards
before cleanup, including quota and root-fence rejection. Fixture cleanup also
uses explicit release. This qualifies local serialization only: outer boot,
constructor, loader or syscall masks/guards remain G1 work. Ordinary physical
failure retains admission/backing without a retry owner; consuming error Drop
cannot attempt release again.

## Execution evidence

Guest fixtures check:

- Both node and context-storage admission reject 64 times each before generation
  or physical mutation.
- Masked production reaping leaves staged owners/callbacks untouched before the
  boot-only never-admitted-context adapter exercises physical success.
- Actual stack-pair success restores frame/charge/slot baselines, rejects a second
  attempt before adapters and prevents growth. Existing executing-SP, Arm
  ownership, callback guard availability and exact generation tests remain.
- A per-call kernel-release rejection occurs while the reaper owns a real
  admitted node. The ordinary failure branch reinserts the same context Box;
  two subsequent scans preserve its generation, phase fence and sixteen mapped
  kernel pages. There is one attempted release, without global fault mode.
- Two ordinary/platform published pairs are dropped with lifecycle, both table
  guards, physical allocator and original admission pool held. No release or
  phase observation occurs. Their two roots/slots and 34 reservation/data pages
  remain retained, along with private table hierarchies.
- The existing six failed stack owners and sixteen initial/growth abandonment
  fixtures retain their original admissions; real collision/quota/publication,
  launch rollback, natural-return, self-exit and cross-LP abort tests remain.

Interruption and physical rejection use scoped fixture adapters; these are not
actual panic unwinding, lost hardware acknowledgements or a concurrent recovery
matrix. Host allocation tracing also checks mutable payload access under the
same node ownership without allocation.

## Validation

| Check | Result |
| --- | --- |
| Complete host harness | Passed, including four retirement-list allocation/identity tests, 29 slot/lease probes and 13 signer tests. |
| Both custom-target kernel Clippy checks, `--locked -- -D warnings` | Passed. |
| Intel VT-d, fresh storage | 15 passed, zero failed/pending. |
| AMD-Vi, fresh storage | 15 passed, zero failed/pending. |
| Arm SMMUv3 security suite, fresh storage | 19 passed, zero failed/pending; probe `0xffff`, publication generations 1/2 and 4,856 cancellation requests retired. |
| Formatting, whitespace and documentation | `cargo fmt --all -- --check`, `git diff --check`, 133 relative links/anchors and unchanged 18-family/seven-gate map passed. |

QEMU execution was sequential after host/Clippy checks. All targets include the
published abandonment and failed-node retention markers. Real natural return,
watched self-exit, user isolation and remote abort remain successful. Assembly
permissions verified 249 x86 native entries and one Arm trampoline. No retirement
deadline expired; this does not resolve the previous Intel timeout.

Commands (unchanged userspace/ABI; existing signed staged bundles reused):

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance published-stack-intel-final-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance published-stack-amd-final-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18569 CATTEN_DEPLOY_HOST_PORT=17869 scripts/run-aarch64.sh --security-test --instance published-stack-arm-20261009 --fresh-storage --timeout 180
```

Executed kernel SHA-256:

- x86: `52dc003e9f38f95f26cf2a4a990bb907a826555fc684891ab4c8e48700d33288`.
- Arm security: `77c26d5100d24d7feff2f13021efc83f204bc2b6a6d529e150ac0909694d315f`.

Initial compile checks caught Box initialization resolving to a borrowed value,
a fixture trait import/mutable lookup, and Clippy inline-error/initializer issues.
These were corrected before passing runtime execution. Inline rejected owners
are deliberately returned without allocation. Initial Intel and AMD executions
also passed 15/15 before the strengthened failure-branch reinsertion test;
the final executions above reran that test. No QEMU execution
failed in this batch. The last source edit only updates stale reaper comments.

## Remaining boundaries

The original [Intel user-stack timeout](2026-10-09-security-preparation-abandonment.md#execution-and-unresolved-progress-observation)
remains unexplained. Later root/phase diagnostics preserve the original deadline
and retained-count policy; fresh-run success is not a recovery proof.

G1 still includes thread metadata/callback fallback, implicit allocation release,
outer ordinary constructor/syscall masks and growth's master thread-table guard.
C15/G3 still requires IOMMU preparation fallback and backend phase separation.
Terminal pairs have no recovery adapter, generalized custody or supervisor
reconciliation. R18-4/5 outstanding-I/O and combined pressure/concurrency evidence
and R18-6 physical-platform qualification remain required. This change creates
no authority to reclaim retained ranges or force-clear their admission.
