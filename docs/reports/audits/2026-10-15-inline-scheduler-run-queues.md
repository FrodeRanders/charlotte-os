# Inline scheduler run queues

Date: 2026-10-15. Baseline `ec2c31c8`, following allocation-free selection and
the conservative x86 dispatch gate. Scoped R7-1/2, C16 and G1/G4 work. SEC-07/18
remain partial; the eighteen owner families, seven gates and finding counts
are unchanged. No historical IPC exit-watch, stack-lease, persistence or NIC
failure is attributed to this batch.

## Remaining gap and design choice

The preceding selection batch removed temporary load/candidate vectors, but
`RoundRobin` still owned a growable `VecDeque<ThreadHandle>`. New/woken admission,
outgoing requeue and migration could allocate under LP/master serialization.
Migration removed its source before the destination push, so ordinary storage
admission could not be moved outside those guards without retaining the exact
source and an allocation-free destination publication obligation.

The correction makes queue storage part of the original complete thread. A
fixed `QueueLink` lives in `ThreadPayload`, already present when the thread is
published. Queue heads retain only exact TID/generation keys, an independent
nonreused queue identity, and a count. There is no runtime ring growth, extra
node, allocation retry, per-instance registry or new custody category.

The link is stored with the existing payload/retirement-node and master-table
allocations. Existing fallible preparation precedes publication; failure still
returns/rejects that complete preparation before queue admission. It does not
add a general thread-byte/principal pool or move ID-table capacity preparation
out of its outer guards. On the 64-bit host, `size_of::<QueueLink>()` is 56
bytes and the queue header is 64 bytes; these are fixed metadata, not new
independently refunded backing charges. Both supported kernels use 64-bit
identities/pointers. No runtime aggregate metadata-bound claim follows.
The live table slot and prepared future retirement node both reserve payload
space; a byte inventory must count both allocations and table capacity, rather
than treating one 56-byte link as the complete thread metadata cost. There is
only one active link in the unique containing thread owner.

## Queue transaction

The production queue core contains no heap or unsafe pointer operations.
Entry accessors qualify the exact thread generation and borrow its inline
link through the caller's table. Moving table vectors cannot invalidate a
retained raw link pointer because none exists. Queue identity is a monotonic
scalar prepared only for boot LPs and bounded private fixtures, not a runtime
application-controlled constructor or a registry.

Enqueue checks the destination endpoints and the original unlinked entry.
Exact removal checks its owner, predecessor and successor before changing
links. Push, pop, removal and migration inspect a constant number of records;
candidate traversal is bounded by its captured count and returns one error
on stale/corrupt linkage. Validation rejects without clearing a successor or
inventing free capacity.

`migrate_ready` checks the source LP/generation, Ready state, certification,
pin/wait/abort/execution fences and architecture policy. It qualifies source
neighbors and destination endpoints before removing anything. Once qualified,
the same exclusive table and LP pair protect the source unlink, destination
append, Ready state and affinity publication. No allocation, destructor,
callback or guard release enters that splice. Load summaries update before
local guards leave; the destination IPI follows guard release. Arm's CPU
handoff and x86's never-dispatched-only gate are unchanged.

## Caller and retirement composition

| Entry | Ownership and serialization | Remaining boundary |
| --- | --- | --- |
| Thread preparation/publication | Existing thread payload, retirement node and ID-table storage include the inline link before external Ready publication. Original root, stack reservation, sponsorship and payload ownership remain. | General heap/principal admission and outer table preparation still need R7/G1/G4 qualification. |
| New/woken `RoundRobin::add_thread` | LP then master table. Queue validation/linking precedes consuming a blocked registration and publishing Ready; missing/stale/foreign/duplicate linkage rejects without consuming its original state. Already-Ready/Running wakes remain benign no-ops. | Waker registration cancellation/implicit field destruction under these guards is unchanged and separately unqualified; queue storage alone is heap-free. |
| `RoundRobin::next` | LP then one exclusive table borrow through outgoing requeue, exact pop and dispatch publication. No growable ring or stale-handle disposal allocation remains. An aborted candidate is unlinked but stays table-owned for requested retirement. | Quantum-timer preparation after selection, outer contexts and broader scheduler metadata remain separate. |
| Ready blocking / immediate abort | Queue removal borrows the caller's existing table instead of reacquiring it. An early-woken outgoing thread can still have a current-handle borrow; queue-only removal must retain that borrow until its context save. | Accepted wait/observer metadata and full call-chain inventory remain separate. |
| Owner-LP requested abort | System read → owner LP mutex → master write. Unlink exact queued membership before table extraction/staging. Release local guards before staging/deferred notification. Remote request publication revalidates its notification LP against the current captured generation/execution owner after a possible move. | This closes queue lifetime composition, not arbitrary thread quiescence or historical timeout causation. No logical state authorizes physical cleanup. |
| Stack retirement / unstarted release / reaper readiness | Linked stack retirement returns `RunQueueLinked` before metadata notification/physical work; unstarted release returns the complete linked owner. Reaper readiness requires no queue membership plus its existing off-stack/CPU-ownership checks. | Abandonment/corruption retains complete owners and original charges; no force-clear or ID-based physical retry. |

The earlier ring could discard a stale scalar handle after a thread had left
the table. Inline membership instead requires production retirement to unlink
before that containing owner leaves. A missing generation is a rejection or
invariant failure, never permission to adopt a newer occupant or detach its
resources. The current-handle borrow and readiness membership are distinct;
removing one must not discard the other prematurely.

Dropping a queue mutates no parent link. A new queue has a different identity
and cannot adopt retained membership. The complete thread's existing fallback
retains its context, root/stack owners, original charges and link. This is
terminal retention, not an owner that authorizes reconstruction or recovery.

The living scheduler state-machine note is also reconciled with observed
source: early Ready/current-handle overlap is explicit; idle's 50 ms admission
IPI fallback is not described as event-only; and runtime migration evidence is
Arm-qualified rather than an x86 CPU-handoff claim. No timer or migration
policy is changed by those documentation corrections.

## Regression evidence

Six direct host tests execute the production queue core:

- FIFO publication, head/middle/tail removal and transfer append order;
- rejected destination, stale generation, duplicate and foreign membership,
  with source order and ownership unchanged;
- deliberately missing original generation cannot clear/adopt successor links;
- queue abandonment retains membership and rejects a replacement queue;
- corrupt-cycle iteration rejects within its captured bound;
- 4,096 deterministic mixed operations over three queues/eight entries compared
  with independent preallocated standard FIFO models.

Current-test-thread allocation tracing covers the operations, rejection and
queue drops: **zero allocations and zero deallocations**. This is a structural
metadata test, not concurrent real-backing pressure or an essential-progress
measurement. The host suite now compiles/runs this production core directly.

Native probes use real never-scheduled thread owners and the production table
adapter. The existing heap-held selection fixture now also executes the same
migration queue/state/affinity helper while both fixture LP mutexes and the
actual master-table lock are held. Stale-generation and retained-executor
rejection precede source mutation; successful forward/backward transfer
preserves exact identity and FIFO append, with no global LP summary replaced.
Post-move abort-owner selection rejects an obsolete LP observation while
respecting its retained executor. Reused local TIDs reject stale insertion and
removal before touching the replacement. These private owners are explicitly
unlinked before complete post-guard release.

A separate actual stack fixture keeps an inline-linked complete thread through
both stack-release and unstarted-release rejection while holding the real heap,
master/retirement tables and physical allocator. Metadata completion remains
unstarted and free-frame counts unchanged. The original owner is returned,
reinserted only in its private never-admitted fixture table, explicitly
unlinked, and released after all guards. The exact original free-frame baseline
is restored. No production root, sponsor or live admission is substituted.

## Execution plan and failure policy

Plan: fresh four-LP Arm/SMMUv3, Intel VT-d and AMD-Vi no-network guests, each
with a 180-second host window and automatic failure capture. Require the new
inline-queue retirement and transfer markers plus authoritative zero failed /
zero pending and runner exit 0. After those pass, run Arm and Intel security
profiles at the same bounds with separate local test ports. Keep any failing
samples separate and publish their diagnostics rather than replacing them
with passing repeats. Signed service bundles remain unchanged.

## Native results

All five runners exit **0** with no kernel panic, persistence fail-stop or
authoritative failed/pending test. No native failed sample is hidden by a
repeat in this batch. Passing runs do not attach LLDB; their retained ELFs are
artifact identities, not captures of an older failed episode.

| Guest | Linked-owner cleanup fence | Heap-held transfer/selection | Authoritative result |
| --- | --- | --- | --- |
| Arm/SMMUv3 | 1.716318 s | 1.757538 s | **17/0/0**, 11.402026 s, bitmap `0x1ffff` |
| Intel VT-d | 1.134841 s | 1.156518 s | **15/0/0**, 14.181702 s, bitmap `0x1bbff` |
| AMD-Vi | 1.125848 s | 1.147492 s | **15/0/0**, 13.824741 s, bitmap `0x1bbff` |
| Arm/SMMUv3 security | 1.130162 s | 1.158424 s | **19/0/0**, 55.630646 s, bitmap `0x2003ffff` |
| Intel VT-d security | 1.123472 s | 1.145791 s | **17/0/0**, 61.399171 s, bitmap `0x2003bbff` |

Arm performs one boot move and then requires the runtime migration counter to
advance to 2 at 11.398102 s (ordinary) and 10.588672 s (security). AMD performs
one actual never-dispatched boot move. Intel's boot move count is zero in
both profiles; x86's started-context runtime phase is explicitly skipped and
its conservative gate is exercised by the selector controls. These results
must not be described as an x86 assembly handoff qualification.

Every guest completes 64 exact-generation IPC claim/endpoint-close episodes,
for **320 total**, plus **20 first-entry** and **20 forced Ready-producer**
controls across four LPs. Existing EL0 stack-growth success, retained self-abort
and foreign-leaf rollback also pass. Security confirms policy generations 1/2
at Arm 33.404356 / 55.625736 s and x86 38.885826 / 61.382489 s. Their concurrent
cancellation traffic retires 14,824 / 13,936 requests respectively.

No historical exit-watch/stack-lease timeout recurs in this bounded matrix.
Passing these probes does not reproduce or explain the historical failure,
qualify actual outstanding-I/O reset or establish essential progress under
combined ordinary resource pressure. No new linked deployment pair is claimed.

## Checks, reproduction and artifacts

The full host suite passes **494 Rust tests, zero failed, four ignored**,
including the six new production queue-core tests. Existing signing/tooling
checks and **13 automatic-diagnostics tests** also pass. Both kernel targets
pass strict Clippy with `--no-default-features --features acpi -- -D warnings`.
Workspace formatting, whitespace and changed-document local links pass. Native
runners verify executable/read-only kernel assembly sections. No assembly,
syscall ABI, userspace source, wire/storage grammar or signing input changes.

All guests use `CATTEN_SKIP_EMBED_BUILD=1 CARGO_BUILD_RUSTFLAGS='-D warnings'`.
The no-network kernels rebuild, and AMD reuses Intel's exact kernel with
`CATTEN_SKIP_KERNEL_BUILD=1`. Security profiles rebuild separately, with
`CATTEN_ALLOW_SANDBOX_NETWORK=1`; host HTTP/deployment ports are Arm 19080/19081
and x86 19180/19181. QEMU's private diagnostic sockets run with the required
host permission. Stores are fresh and instance-local. Signed service bundles
are unchanged from the
[persistence batch](2026-10-15-raft-persistence-diagnostics.md#artifact-identity);
no new service build or service Clippy result is claimed.

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance inline-run-queue-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance inline-run-queue-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance inline-run-queue-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance inline-run-queue-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance inline-run-queue-x86-security
```

| Executed artifact | SHA-256 |
| --- | --- |
| Arm no-network kernel | `5056a1bcfcee0489ee5b14bb0e8da8b8f7d1abec42251975c0edc0110a3422dd` |
| Intel/AMD no-network kernel | `a26f2b754bea746608782e77e18c5d13154b367412049e72bcb48fb54d606302` |
| Arm security kernel | `ca3cebbd24e8cde42646dddf06decac5d538488178c71b2fb43239622b21de9b` |
| x86 security kernel | `55faa4cd266ba355b022c0c5c43e7eda52509ef745c1ae95a36062e853570bfa` |
| Arm no-network final UART | `a93f68191470f7d7220b1eca052e64d30e0811d04befc15256d916cda8225c43` |
| Intel no-network final UART | `601a3faacc74d068a339922cee9707e1153723b32719375dd9d21124e14fdcff` |
| AMD no-network final UART | `677d83f1fb3bdeb5e5055c2499fa99876141325230aa71c08655397dd600387d` |
| Arm security final UART | `df9d2e12f60a3da52b0fd3652c50cb1f3b43498f7f2daef77808e9d8a482bbc4` |
| x86 security final UART | `d3bc0a204f573c039b8f92387afca71d05d0d996ebb8c4bbd6e9f9aee1f6ae8e` |

Per-run manifests retain immutable actual boot ELFs, target/features, bundle
identities and dirty-path qualification. Tracked-diff hashes exclude then-new
queue/reference files and are not complete source-tree identities; the
resulting commit and the executed ELF hashes are the source/artifact anchors.
Only comments and living documentation were reconciled after the native logic
was frozen; no operation or fixture behavior changed between these runs.
Relevant outcomes, failure qualifications and hashes are inline here rather
than relying on ignored artifacts as the published evidence.

## Disposition

The remaining growable Ready ring is removed, and migration has no fallible
destination storage step after source removal. All listed publication,
requeue, blocking and retirement paths use the same inline exact-generation
membership. Rejection/abandonment retains complete owners and original charges;
queue observations grant no physical cleanup, custody or replay authority.

Next scheduler work is the remaining waiter/waker preparation and implicit
registration destruction under outer scheduler guards, alongside table/vector,
heap/principal and full call-chain inventories. Translation/heap/image rollback,
shared recovery custody and integrated pressure/outstanding-I/O qualification
remain separate work. SEC-07/18 and historical failure attribution stay open
within their existing acceptance boundaries.
