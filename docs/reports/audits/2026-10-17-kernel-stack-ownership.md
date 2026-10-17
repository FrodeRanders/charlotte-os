# Kernel stack range ownership

Date: 2026-10-17. Baseline `3a0454f8`, following the
[security status review](2026-10-17-security-status-review.md).
This addresses its stack guard-map observation under SEC-07 R7-1/R7-2 and
SEC-18 C16/C17/G1. Numbered dispositions remain **29 corrected within scope,
six partial and three open**. No new registry or cleanup family is introduced.

## Source boundary and correction

The stack allocator previously inserted two guard-page addresses into a
`BTreeMap` after mapping the usable range. Both insertions could allocate
infallibly under stack-arena serialization and the guard-map write hold.
Admission of stack data pages did not make those metadata allocations fallible.
The guard map's only reader was teardown; fault lookup did not consult it.
It checked addresses supplied as a scalar base/page-count pair.

The allocator now returns a non-copyable `KernelStack` containing the exact
base, page count and release-started fence. Only successful stack allocation
constructs that type; there is no scalar adoption constructor. `Stacks` owns it
beside the original user slot/root/reservation or kernel-only admission.
Existing architecture contexts continue borrowing `Stacks::kernel_base` to
initialize their stack pointer; an address observation grants no cleanup owner.

Explicit teardown borrows the same range owner and records its first attempt
before detachment. A second attempt rejects before entering the arena, page
tables or retirement work. Ordinary success removes that owner from its
containing pair. Rejection/interruption leaves the fence armed and the pair's
original admission retained. Existing pair-level one-shot and uncertainty
policies remain in force too. Dropping a range owner has no physical,
allocator, pool, callback or logging behavior.

The map and its reference counting are removed. Both guards remain unmapped;
teardown detaches only the owner's usable range. Adjacent ranges can share an
unmapped guard because neither owns a mapping there or needs a mutable guard
record. This removes the allocation site entirely rather than introducing
another metadata budget or registry. Invalid zero/over-capacity layouts reject
before arena or backing access. Mapper/table allocation inside range creation
remains a separate admitted translation boundary.

Physical retirement still uses the existing `RetiredKernelRange`: detach under
arena/table serialization, leave those guards, confirm invalidation and release
frames once. Its existing fallback retains uncertain backing; the containing
stack pair retains the original root, slot and complete reservation. The new
range owner is not a retry or recovery receipt. Caller-owned outer IRQ masks
are preserved, and physical release is still forbidden under unqualified outer
guards. No scheduler, IRQ, ABI, wire/storage format or hardware drain policy
changes are made.

## Controls

The native stack-admission fixture now exercises two real admitted kernel
ranges. It verifies that every usable page is mapped, both guards are unmapped,
and releasing the first returns exactly sixteen data frames without changing
the second's mappings or guards. Releasing the second restores its original
admission and data count. Invalid zero and over-capacity layouts reject
without mapping, consuming frames or changing admission.

The existing retained kernel-release fixture now injects its rejection through
the actual range owner's first-attempt fence. A direct subsequent production
teardown rejects before touching still-live backing. The containing pair also
rejects its second attempt; dropping it retains the same sixteen kernel pages,
exact user root/slot and original maximum reservation. This reuses the existing
negative control and adds no permanent retained fixture backing.

Existing real mapping/collision rollback, user invalidation, slot cancellation,
published/provisional abandonment, 128-round thread preparation/release churn
and scheduled exact-exit controls continue to exercise the containing owners.
These are bounded controls, not proof of whole-node progress under exhausted
backing or every constructor/syscall's outer context.

## Validation

Fresh host validation passed **509 Rust tests, zero failed, four ignored**
across 57 harness summaries, plus signing/tooling/Python controls and
cluster-sign self-tests. Strict kernel Clippy passed on both custom targets
with `-- -D warnings`. Workspace formatting and diff whitespace checks pass.

All five four-LP QEMU profiles completed with runner exit **0** and diagnostic
result `complete`. Each used fresh storage and a 180-second guest budget.

| Profile | New range-owner probe | Final passed / failed / pending |
| --- | --- | --- |
| ARM / SMMUv3 | 1.649876 s | 17 / 0 / 0 at 12.073447 s |
| Intel VT-d | 1.232562 s | 15 / 0 / 0 at 20.566040 s |
| AMD-Vi | 1.190586 s | 15 / 0 / 0 at 14.518357 s |
| ARM security | 1.171964 s | 19 / 0 / 0 at 55.918860 s |
| Intel security | 1.182439 s | 17 / 0 / 0 at 61.644405 s |

Together these check ten newly exercised admitted kernel ranges, returning
160 data pages, and complete 320 scheduled IPC claim/endpoint close episodes
with exact exit watches. The existing retained kernel-failure control still
consumes its original sixteen pages per guest; there is no additional persistent
negative-control cost. Passing runs do not establish sustained fairness or
attribute historical failures.

The changed Rust remained identical across validation. ARM reused its staged
service bundle; x86 services were built/staged with the existing x86 script.
No service source changed. Kernel assembly-section checks remained enabled.
AMD reused the Intel no-network kernel; their ELF hashes match. The ARM security
profile used forwards 19080/19081 and Intel used 19180/19181.

The first ARM runner attempt exited before guest execution because the sandbox
rejected the diagnostic QMP Unix socket (`Operation not permitted`). The
outside-sandbox repeat used the same kernel hash and passed; the failed attempt
is recorded separately. An early x86 Clippy invocation also preceded service
bundle staging and was rerun successfully after staging. Neither is a failed
native test sample. No unexpected guest panic, deadline or persistence fail-stop
was observed in the completed matrix.

The accompanying [version-controlled evidence](evidence/2026-10-17-kernel-stack-ownership.json)
contains exact commands/environments, source/kernel/boot-image/initial-disk/UART
hashes, staged service inventories and concise probe/result markers. It records
the pre-guest failures separately. Published evidence does not depend on temporary
logs or ignored build artifacts. Existing user documentation edits were preserved;
this is not a full-tree attestation or physical-platform qualification.

## Remaining work

The separate lazy memory-budget account map and waiter/timer sponsor
construction still need fallible preparation before address-space publication,
with exact generation/lifetime retention and post-guard disposal. This is not
the stack range boundary and is not corrected by removing its guard map.
General heap/aggregate principal admission, active observer/token/source
destruction contexts, integrated hostile pressure and recovery reconciliation
remain SEC-07/18 work. Production trust, authenticated channels and security
time remain separate milestones. No historical IPC, stack-timeout, persistence
or NIC failure is attributed to this source correction.
