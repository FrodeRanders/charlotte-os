# Scheduler wait preparation and publication

Date: 2026-10-15. Baseline `0c4f44a8`, following inline scheduler ready queues.
Scoped R7-1/2 and C16/G1/G4 work; SEC-07/18 remain partial. Finding counts,
eighteen owner families and seven cleanup gates are unchanged. Passing bounded
runs below do not attribute historical IPC exit, stack, persistence or NIC
failures.

## Observed gap

`SystemScheduler::block_thread_with_constraint_generation` allocated its
`Arc<Waker>` and called source registration while holding the master thread
table, plus the LP guard for Ready targets. The source could allocate admitted
entry/list backing or acquire another registry beneath those guards. Immediate
readiness and errors could also destroy that metadata there.
`RoundRobin::add_thread` cancelled a Blocked waker's registration and replaced
its final strong reference while holding both LP and master-table guards.

Simply moving registration before publication would lose an event delivered
in that interval: the old callback's generation-only admission could see a
Running/new target, return without parking it, and leave a later Blocked
publication with no producer. A retained callback from an older park could
also wake another park in the same thread generation. No historical failure
is assigned to either source observation.

## Containing ownership and publication

The preparation captures thread generation, state/LP and its existing
generation-owned waiter sponsor under a short table borrow. After that guard
leaves, fallible waker and source registration run without local LP or
master-table guards. The same waker owns the subscription through preparation
and publication; no ID-based re-adoption, new registry or root/stack owner is
introduced. The sponsor is cloned, never re-resolved by recyclable ASID or
reclassified during a live wait.

Publication borrows that complete preparation. It locks the captured Ready LP
before the table, validates the original generation/state and retirement-ready
abort fence, then removes inline queue membership and publishes Blocked with
the same waker. A changed state/LP, reused slot, existing Blocked owner or
termination rejects before mutation. This is not an automatic publication
retry. Running self-block keeps its current handle through the outgoing save.

Each waker stores an atomic notification hint before attempting admission.
Publication checks that hint under the table guard. If the callback was early,
the hint prevents parking; if it races after that check, admission takes the
same table serialization and sees the installed Blocked waker. Source callbacks
must match that waker's pointer identity as well as thread generation. Its
strong reference keeps the original identity allocated, so a later waker
cannot reuse that address while the callback survives. A hint from an older
park cannot admit a newer park, including within the same generation.

Wake admission moves the complete old waker out through the caller's LP scope.
`with_detached_wait` explicitly cancels and drops it after local LP/table
guards leave, even when a subsequent placement step returns an error. It
handles all four production `add_thread` callers: new admission, wake,
explicit pinning and initial migratable placement. Rejected, immediate-ready
and early-notified preparation also cancels after publication guards leave.
Detached notification batches retain their own original charge until release.
Existing reaper metadata cancellation remains outside its local table/staging
guards; complete Thread fallback still retains every field without cancellation.

This changes neither a wait result into operation completion nor timeout into
cancellation. Pending operations, borrowed memory, exact roots and original
charges retain their existing owners. Explicit condition/watchdog
`submit_woken_thread` hints remain generation-qualified and may coalesce an
already-runnable thread; source callbacks use the stricter installed-wait
path. No physical, quiescence, retry or custody authority follows from either.

## Caller and context inventory

| Entry | Local boundary after correction | Remaining qualification |
| --- | --- | --- |
| Completion waits and checked CQ waits; scheduler timed/event-only condition loops | All use the shared capture/prepare/publication adapter. Source callbacks retain early hints. Queue/state mutation is allocation-free; rejected/unused registration disposal leaves LP/table guards. | Existing outer system-read and park masks remain. Timed loops keep the original deadline, watchdog owner and condition recheck. No forced IRQ enabling or ABI change. |
| IPC receive/reply, blocking mutex/RwLock, sleep and reaper waits | Same scheduler adapter; no source registration beneath its LP/table guards. Sources still detach notifications before callbacks. | Existing source registry/initialization allocation and caller-specific outer masks remain. Admission fallback, accepted-operation ownership and source readiness contracts are unchanged. |
| Four scheduler admission callers | Removed Blocked waker is carried outside the LP scope before cancellation and strong-reference disposal, including later ordinary error. | Outer system read/caller contexts remain; this is local metadata disposal, never physical cleanup. |
| Thread retirement/fallback | Existing explicit metadata completion cancels Blocked registrations after local guards; complete fallback retains all fields. | No new reaper, root lease or recovery owner. Broader constructor, heap, timer and final bookkeeping contexts remain open. |

Source-side allocation remains concrete follow-up work. For example,
`WaiterSource::register` can initialize a list under its own slot mutex, and CQ
and IPC sources call sponsor registration under their registry read guard.
Moving the scheduler's enclosing locks does not qualify those inner holds or
the allocator's complete call graph. Existing fallible entry preparation can
support a later exact-source capture/revalidation change without adding a
per-object registry.

## Focused verification

The new native probe runs in the existing thread-admission fixture, using
never-dispatched real Thread owners affined to the masked producer LP and
uncertified for migration, with private waiter sponsorship prepared before publication and normal
node admission. It never substitutes a live shared account or claims a
worker-return marker as quiescence.

- A source acquires the real local LP and master-table guards during
  registration. Legitimate remote contention can resolve; a recursive
  scheduler hold would deadlock and be captured by the bounded runner.
- Rejected and immediately ready sources preserve constructor state,
  constraints and zero private waiter usage.
- A real waker callback between registration and publication records its hint;
  publication remains runnable. A callback after publication performs normal
  Ready admission and cancels despite retained strong references.
- A second park removes real Ready membership. A retained first-wait callback
  leaves the second waker and its subscription untouched; its own callback
  still wakes it.
- Publication, Ready removal, changed-state/slot rejection and wake detachment
  run while the actual global heap is locked. The complete detached token
  remains charged until cancellation after heap/LP/table guards leave.
- Changed-state rejection and exact TID-slot reuse between capture and
  publication leave the current owner untouched. A stale callback cannot
  schedule the successor. Explicit queue unlink precedes never-dispatched
  fixture release; the entry IRQ state is preserved.

These are deterministic forced orderings, not exhaustive cross-LP race or
allocator-failure injection. Existing scheduled CQ/completion/IPC, lock, timer,
abort/reap, park-boundary, first-entry and real EL0 stack-growth tests supply
normal concurrent execution coverage in the guest matrix.

Host validation: **494 Rust tests passed, 0 failed, 4 ignored** across the
existing harness, plus signing/boot/diagnostic/runtime-input/tooling/object-store
checks and cluster-sign self-tests. There is no new host-harness test for this
freestanding scheduler; the new checks execute in the guests. Strict Clippy
passed for both kernel targets with `--no-default-features --features acpi --
-D warnings`; formatting, whitespace and 473 local documentation-link checks
passed.

## Native matrix

The plan was one fresh four-LP no-network run for ARM/SMMUv3, Intel VT-d and
AMD-Vi, followed by ARM and Intel security profiles, each with a 180-second
limit and the existing automatic all-LP failure capture. All completed runs
are reported separately below; none has been replaced by a passing repeat.

| Profile | New publication probe at | Exact IPC close episodes at | Final passed / failed / pending at |
| --- | --- | --- | --- |
| ARM/SMMUv3 | 1.180144 s | 64 at 8.104772 s | 17 / 0 / 0 at 10.089474 s |
| Intel VT-d | 1.182539 s | 64 at 13.274387 s | 15 / 0 / 0 at 14.117161 s |
| AMD-Vi | 1.200092 s | 64 at 13.404003 s | 15 / 0 / 0 at 14.265587 s |
| ARM security | 1.173108 s | 64 at 8.815990 s | 19 / 0 / 0 at 55.746650 s |
| Intel security | 1.174610 s | 64 at 13.685639 s | 17 / 0 / 0 at 61.346096 s |

All five runners exited **0** with diagnostic result `complete`; no kernel
panic or Raft persistence fail-stop was observed. Each passed the new native
probe, four exact first-dispatch controls, four forced Ready-producer park
controls and three real EL0 growth modes. Total: **320 exact close/exit episodes,
20 first-entry controls, 20 forced park controls and 15 EL0 growth modes**.

ARM no-network boot migration moved two certified Ready threads; sustained
runtime sampling advanced its counter to three at 10.081554 s and lifecycle
completed at 10.088904 s. ARM security moved none at boot, then its sustained
counter reached one at 10.653166 s and lifecycle completed at 10.666682 s.
Intel no-network moved two never-dispatched threads at boot, AMD none, and
Intel security three. All x86 runs explicitly skipped started-context runtime
migration under the existing conservative architecture gate. Every profile
reported 192 timer wakes retaining established LP affinity.

ARM security fenced generations 1/2 at 33.525148/55.737340 s and retired 14,808
concurrent cancellation requests. Intel security fenced generations 1/2 at
38.852467/61.331979 s and retired 13,952 requests. These are bounded fixture
results, not per-domain progress guarantees or production enrollment.

SHA-256 identities of the actual kernel ELF and final UART bytes:

- **ARM/SMMUv3** (`aarch64-_iaz8o1_`):
  kernel `fb29505dfffdb592bbb78568e39019ffbdeb551b6cc429b999c65d5fa5aa1965`;
  UART `46fbde9533e278dff4036037119b7d2161728a364febeeb1cc024b4fd28dd93e`.
- **Intel VT-d** (`x86_64-_c3ftma2`):
  kernel `bec6fdf97f92734ecd9578ee9afe4d3412b51deea0b8f7d7678160e52dd8ec4f`;
  UART `39b4ab1bceff5482fa4669a86f107b995f6d8ad4cba930cff734681f0aa8d43a`.
- **AMD-Vi** (`x86_64-x3q5m8pe`):
  kernel `bec6fdf97f92734ecd9578ee9afe4d3412b51deea0b8f7d7678160e52dd8ec4f`;
  UART `05432fe68a441359bea7f5f2f637347c6766b8e59c301c269e9512057400b684`.
- **ARM security** (`aarch64-87hkot4s`):
  kernel `7348c89fbd662c0fd2449c28631a9be925b5d2d292cba256c9d9869952a0ee70`;
  UART `d14ffaa5fb9ca814f0cab607654b478330b186d071ec4e5fb9372e7a80a9f672`.
- **Intel security** (`x86_64-n3s4ue_x`):
  kernel `b33f0db9a1192988158693ddbc0206a253fe1f4d36a0e245b9b15c6fc5fa890b`;
  UART `5a551d0203e0976eaa762d994bcc28492d886541787291faed06621fb0a91c27`.

## Reproduction and artifact identity

Use `CATTEN_SKIP_EMBED_BUILD=1 CARGO_BUILD_RUSTFLAGS='-D warnings'` with each
runner. Bundled services were unchanged from the preceding queue batch. Their
complete per-architecture identities, reviewed Rust source hashes, boot-image
and initial-disk hashes, and selected per-run observations are preserved in
[version-controlled evidence](evidence/2026-10-15-scheduler-wait-publication.json).
The selected Raft/DNS identities also match
[the earlier artifact inventory](2026-10-15-raft-persistence-diagnostics.md#artifact-identity).

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance wait-publication-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance wait-publication-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance wait-publication-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance wait-publication-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance wait-publication-x86-security
```

AMD reused the exact Intel no-network kernel with `CATTEN_SKIP_KERNEL_BUILD=1`.
Security runs used `CATTEN_ALLOW_SANDBOX_NETWORK=1`, HTTP/deploy host ports
19080/19081 for ARM and 19180/19181 for x86. Runner assembly checks enforce
executable, read-only native trampolines. No assembly or userspace bytes change
in this batch; no linked-pair, reset or outstanding-I/O campaign is claimed.

Runtime logic and new native probes were unchanged throughout the matrix.
Rust documentation comments were reconciled after the first ARM build; its
own ELF/UART identities are retained separately. Run manifests capture dirty
tracked diffs, which exclude then-untracked `wait.rs` and its probe; that hash
alone is not a complete source identity. The committed source and per-run ELF
hashes are the anchors. Temporary runner logs/manifests are not required to
understand the results recorded here.

## Remaining work

Next: source-side waiter entry/list preparation outside registry and
initialization guards, retaining exact source identity and readiness rechecks.
Outer system/park masks, timer/table allocation, general metadata/principal byte
admission, integrated hostile-pressure progress and the full R18 call graph
remain open. This batch adds no recovery registry or physical retry owner,
does not close SEC-07/18 and does not explain the historical intermittent
failures.
