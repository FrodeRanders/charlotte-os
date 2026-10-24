# Security audit: prepared read-loan borrower storage

Date: 2026-10-24. Reviewed baseline: `43f5132d`, including the existing
uncommitted memory-object storage follow-up.

Read-loan publication no longer allocates its borrower-map record after shared
authority commits. Fallible preparation precedes source escrow; the existing
transfer owns the unused node. Confirmed cleanup carries a detached borrower
node and its exact authority past the memory guard, freeing the borrower node
before the original authority charge returns.

This advances SEC-07 R7-2 and C05/C06/C18. The ledger remains **29 corrected /
6 partial / 3 open**; SEC-07/18 remain partial and production trust remains
disabled. The [tracked evidence](evidence/2026-10-24-security-memory-borrower-storage.json)
records source/artifact identities, results and selected diagnostics. Historical
reports, including the preceding object-storage checkpoint, are preserved.

## Preparation, publication and retirement

The former read-borrower `BTreeMap` inserted after atomic authority publication.
Its insertion could allocate after the source/destination mutation. It now uses
the shared admitted map. Source transfers reserve the exact captured destination
and prepare read-loan storage before entering `MEMORY_OBJECTS`. Source/target
admission, ownership, rights and loan-state validation still run under memory
serialization before source escrow and its pin. Preparation failure publishes
nothing and leaves the source writable; rejected/unused storage and its
reservation dispose after that local guard. Write loans need no separate node.

`PreparedTransfer` retains the node through cancellation or publication. The
existing batch validation/authority commit is unchanged; read-loan publication
only relinks its prepared node. Source moves and write loans also perform their
destination reservation before memory serialization. Copy preparation retains
its existing source-pin path. No source escrow or live pin crosses a newly
introduced unlocked interval.

Revocation's existing `prior` state owns the complete read-borrower map. A
failed or abandoned started revocation keeps every node with its Revoking fence,
backing pin and original authority. Unstarted cancellation alone restores that
same map. Confirmed read revocation detaches the selected borrower, removes its
scalar payload and pairs its exact shared authority in the original borrower
node. Post-registry release frees that node before authority metadata and its
original admission refund. Other readers stay linked. Write revocation also
carries its detached authority past the memory guard.

Mapped and unmapped borrower namespace cleanup use the same pairing after their
existing proof. Removing the scalar payload at pairing prevents a later namespace
pass from retiring it twice. Final object backing disposal explicitly drains any
remaining borrower nodes after physical completion, before object-node disposal
and original backing refund. Physical rejection retains the complete object.
No new root, pin, registry, recovery family, custody, timeout or physical/replay
authority is added. Existing outer IPC/IRQ/destructor contexts remain separate.

## Verification

Serialized native controls exercise:

- **128 borrower-allocation rejections** after destination admission and before
  source escrow/pin. Memory, heap and frame allocator are available at that
  factory boundary. The source stays writable; authority counts and physical
  frame baselines return after each rejection.
- **64 unused read-loan cancellations** retain hidden destination authority and
  its node together, then restore source access and original admission counts.
- **64 two-reader publications/detachments** use the exact production publisher
  and actual node types while holding the memory registry and heap. Removing one
  record preserves the other. Disposal occurs after the guards; IRQ state is
  preserved. Isolated scalar rows publish no synthetic capability authority.
- **32 real mapped revocations plus unmapped peer namespace closes** preserve
  the other reader, keep lender writes fenced until final loan cleanup, and
  restore original authority/backing counts after confirmed release.
- Existing four revocation failure/Drop fixtures retain the same four data pages.
  Detach failure still uses a write loan; barrier failure, scratch failure and
  guarded abandonment now use read loans. Thus three additional borrower nodes
  are deliberately retained within those existing quarantines, without new data
  pages or budget resets. Abandonment holds both memory and heap guards; it can
  neither deallocate the prior node nor restore borrower state.
- Existing mixed authority, hidden delivery, mapped-loan failure, returned
  capabilities, cancellation and scheduled namespace/endpoint fixtures remain
  enabled. These results do not inject global allocator exhaustion or qualify
  arbitrary interruption/schedules.

The first unsandboxed native executions pass on both architectures: **AArch64
20 passed / 0 failed / 0 pending**, and **x86_64 18 / 0 / 0**. Both complete
all **14 HTTP checks** and both security-service generations. Healthy responses
beside stalled peers take **0.198–0.385 s** on ARM and **0.196–0.408 s** on
x86. Trickle checks reject after **30.14 s / 30.21 s**; cancellation pressure
retires after **19,960 / 15,904 requests**, respectively.

Each run preserves an initial host metrics `TimeoutError` and four
`ConnectionResetError` retry diagnostics. Later attempts pass within existing
retry/deadline limits; no guest repeat or limit widening was used. These host
observations do not attribute earlier intermittent failures.

All **16 shared collection host tests** pass, including allocation-free
relink/detach, inert abandonment and actual node disposal before admission
refund. Strict base-kernel Clippy passes on both architectures. Workspace
formatting and whitespace checks pass. Bundled services are reused; userspace
sources and the tracked manual PDF are unchanged. Assembly-section checks remain
enabled in both native runners.

The initial sandbox launches never executed a guest: QEMU could not bind its
local diagnostic socket (`Operation not permitted`). The initial TLC attempts
likewise stopped before state exploration because local RMI socket creation was
blocked. Authorized unsandboxed runs are recorded separately; these setup
rejections are not guest panics or model counterexamples.

## Formal qualification and remaining work

Both focused safe configurations and all **seven expected counterexamples**
pass their harness checks. `CharlotteIpcTransaction` explores **26 distinct /
71 generated states**, depth **11**; `CharlotteRootRetirement` explores
**2,978 / 3,733**, depth **33**. Required safe actions execute, and unsafe cases
violate their named invariants through the intended actions.

No model/configuration changes. Existing loan/rollback, non-terminal timeout,
exact-root, physical-once and original-charge projections remain. Concrete
borrower-node construction/deallocation, the paired authority lifetime, implicit
field destructors and registry/allocator/IRQ contexts are native/host
qualification. The earlier full 190-case checkpoint is separate; no executable
refinement or sustained-pressure fairness proof is claimed.

Memory capability/namespace and mapping maps still allocate, including target
payload insertion after authority commit. Source-escrow metadata preparation and
some containing IPC/IRQ/factory/destructor contexts remain guarded work. Shared
capability admission bounds loan counts; general metadata bytes, allocator
overhead and aggregate principal ceilings remain incomplete. Borrower lookup
and insertion are now linear; count limits do not establish worst-case hold
time or sustained management/teardown progress. Historical queued-loan mapping
panics, root-close timeouts and startup/probe intermittence retain their own
causal limitations; passing runs here do not explain them.
