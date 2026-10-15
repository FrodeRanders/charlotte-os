# Scheduler selection storage and the x86 dispatch gate

Date: 2026-10-15. Baseline `5ace30f1`, following stack-growth execution
ownership. Scoped R7-1/2, C16 and G1/G4 work; SEC-07/18 remain partial.
No historical IPC exit-watch, stack-lease, persistence or NIC failure is
attributed to this correction.

## Source findings and correction

Both `try_rebalance` and `try_rebalance_sustained` allocated and sorted a
`Vec<(LpId, usize)>` while their caller retained the system scheduler's read
guard. The migration path additionally collected every source-queue candidate
into a vector while both LP scheduler mutexes were held. Searching and
destroying that vector happened beneath the master thread-table write guard.
These temporary allocations used the ordinary global heap, with no separate
sponsor or fallible admission. They were not necessary to select one move.

`LoadExtremes` now samples each tracked LP summary once and retains only the
least and most loaded scalar pairs. Lexicographic `(load, LP)` comparison
preserves the former sort's tie rule: lowest LP among minimum loads, highest
LP among maximum loads. No summary snapshot or sorting storage remains. The
source and destination's actual loads are still rechecked after both LP locks
are acquired; sampled summaries are not an authority to move a thread.

The per-LP trait now returns the first qualified queued identity by scanning
the existing queue against the caller's borrowed thread table. It neither
allocates candidate storage nor reacquires the master table. Exact thread
generation, source Ready LP, migration certification, pin/wait/abort/execution
fences and architecture ownership still qualify selection. The caller retains
the same LP pair and table guards through queue/state/affinity publication.
There is no caller-supplied callback, new registry, operation owner or retry
boundary. Selection still follows the existing source queue's order.

## The documented x86 restriction was not enforced

The caller trace found a second concrete discrepancy. x86's context ownership
predicate returns `false`, with a comment saying runtime migration remains
disabled until an assembly handoff exists. The living thread-retirement note
repeated that limitation. In source, both architecture yield tails nevertheless
call `maybe_sample_rebalance`, and the migration predicate did not reject an
already dispatched x86 context. Ready publication alone cannot prove that its
outgoing context save has finished after the scheduler guards leave.

x86 now rejects every context whose `dispatch_count` is nonzero. Dispatch
increments that counter under the same thread table before releasing the
chosen context. Thus only certified, never-dispatched Ready contexts may move
on x86; the existing sampler continues to run. This restores the conservative
boundary rather than adding a new CPU-ownership protocol. Arm retains its
assembly `on_cpu` handoff and may move qualified Ready contexts after prior
execution. No new policy permits moving a pinned userspace shard.

The live lifecycle fixture explicitly skips x86's started-context runtime
migration phase, while preserving timer-affinity and remote-abort/owner-LP
retirement checks. Its result text identifies the architecture qualification.
Arm still requires actual sustained-window runtime migration. The serialized
selector fixture directly distinguishes dispatched from never-dispatched
contexts. A passing x86 suite is not evidence of a new assembly handoff.

This is source evidence for an unenforced restriction, not a reproduction of
an unsafe actual x86 transfer or the cause of a historical panic. Earlier
reports remain unchanged; current living descriptions now name the enforced
gate and the remaining architecture boundary precisely.

## Scoped caller and allocation inventory

This inventories rebalance selection and its immediate containing move. It is
not R7-1's complete kernel allocation inventory or R18-1's complete call graph.

| Entry or storage | Context, owner and admission | Current disposition |
| --- | --- | --- |
| Initial `main::finish_boot` rebalance loop | Boot continuation; system-scheduler read guard enters numeric-order LP pair, then master table. Individual mutexes preserve the enclosing IRQ state. | Calls the same allocation-free selectors and architecture gate. This does not make bootstrap construction or the containing move allocation-free. |
| Arm/x86 `cond_yield_lp` → `maybe_sample_rebalance` → sustained selection | Architecture tails have released local switch guards but still mask IRQs before restoring the enclosing state. LP0's bounded-time sampler holds the system read guard; a move adds both LP mutexes then the master table. | Temporary load/candidate allocation removed. No IRQ is enabled to manufacture an allocation context. |
| Load extrema | At most the configured tracked-LP set; previously a temporary global-heap vector, released under the system guard. | Two scalar pairs replace storage, with unchanged deterministic ties and one sample per LP. No allocation or charge remains. |
| Migration candidates | Previously one temporary copy per queued source handle, allocated under both LP guards and disposed while the master table was held. | Existing queue is borrowed under the supplied table; no temporary heap or destructor remains. |
| Destination `RoundRobin::run_queue` | `VecDeque<ThreadHandle>` belongs to the LP scheduler. `add_ready_from_migration` may grow it under system/LP/master guards after source removal. `add_thread` and ordinary requeue are other callers. Domain thread limits are not an aggregate queue-byte admission policy. | Still open. Nonfallible queue growth needs prepared storage and phase-correct publication; these selection changes do not qualify the whole transaction or essential progress. |
| Thread-table vectors and publication | `IdTable` prepares payload/slot/free-ID capacity fallibly, but the containing lifecycle/publication/master context and general metadata bytes remain separate. | Still open under R7/G1/G4. No new admission or destructor proof here. |
| Scheduler LP map, observer, fixture storage | LP map/quantum-observer construction belongs to platform initialization. New probe queues/Arcs and local thread-table storage are trusted, bounded boot fixtures prepared before held-heap checks. All fixture contexts remain local and never scheduler-admitted. | Not a runtime application allocation exemption. Wider scheduler, statistics, waiter/timer, kernel-arena and aggregate-principal accounting remain separate inventory work. |

The remaining destination queue is the next concrete scheduler storage boundary.
Any correction must admit backing before mutation without losing the exact
thread/generation or accepting a move that cannot publish its destination.
Readiness, a sampled load or a busy code supplies no cleanup/replay authority.

## Regression probes

Native load tests cover empty/singleton inputs, noncontiguous LP identities,
permuted minimum/maximum ties and maximum scalar loads while holding the actual
kernel heap lock. The production summary iterator is also called beneath that
hold without overwriting live summary counters.

Candidate probes prepare two never-scheduled contexts, a private thread table
and two fixture LP queues before locking the heap. Selection then runs while
both fixture LP mutexes, the real master-table write guard and the heap are
held. Controls cover missing/stale identities, first-valid queue order,
pinning, missing certification, abort and retained execution, wait constraints,
foreign Ready LP, Running/unassigned states, architecture dispatch ownership,
no eligible candidate and an empty queue. Arm additionally rejects its nonzero
CPU-ownership flag. Local TID reuse leaves the old generation in the queue and
must select the other exact live owner. Scans do not mutate the queue.

These controls exercise selection, not the destination queue push. Complete
fixture contexts are explicitly released only after every local guard leaves;
original constructor state is restored solely for those private, never-admitted
owners. No live thread state, root, timer sponsor or platform classification is
replaced. Existing actual migration, abort, IPC close, first-entry, park-delivery
and EL0 growth fixtures remain enabled with their architecture qualifications.

## Failed execution retained separately

The first ARM execution booted selection-only kernel
`9a2f9a5cdae4bd5f5b3e96646e2caf3247f060e708bd455ebaf1e212d979fc16`.
At guest 1.147232 s, domain-abort probes had passed; the new selector fixture
then panicked at its original `selection_tests.rs:115:62` during
`release_unstarted().unwrap()`. The returned complete thread recorded
`state=Ready(0)`, `generation=45`, `dispatch_count=0`, `on_cpu=0`, an intact
kernel stack and its sixteen-page reservation. `release_unstarted` correctly
rejects a state other than `NeedsLpAssignment` before physical teardown.

This was a mistake in the new fixture's cleanup: its synthetic Ready input
was never published to a live scheduler, but had not been restored to the
constructor state before consuming the unstarted adapter. The corrected
fixture restores only that local owner, verifies its exact generation and
zero dispatch/CPU ownership, and releases after guards. Production cleanup is
not weakened. This failed run precedes the x86 dispatch-gate correction and
is not a negative execution of that gate.

Automatic diagnostics captured all four CPUs through QMP and LLDB, with exit
0, no reported errors, and watcher elapsed time 7.520942 s. The UART raw
backtrace reported that its frame chain was unavailable in the current stack
page; complete returned-owner diagnostics above still identify the rejection.
Failed UART SHA-256:
`a8de84533ec769cd51fb0722166a9db5c086d890fea11e6ef7a3ff9a8abfca74`.
Its manifest's tracked-diff digest was
`573260654ad7e1e5d4e9108bfe59cd4a2dd4548e21033139fce9ca43bf8a1c1a`;
the actual boot kernel identity above also covers the then-untracked fixture.
Relevant evidence is inline, without relying on temporary logs for publication.

Two earlier sandboxed launches (ARM and Intel) failed to bind QEMU's private
QMP Unix socket with `Operation not permitted`, before guest execution. They
are host-environment failures, not passing or failing kernel suites. Execution
was retried with the required socket permission; the ARM failure above was
then preserved rather than replaced by a passing repeat.

## Validation plan

Use fresh four-LP no-network guests on Arm/SMMUv3, Intel VT-d and AMD-Vi, each
with a 180-second host window and automatic failure capture. Require the new
selector marker, authoritative zero-failure/zero-pending results and successful
runner exit. After those pass, run Arm and Intel security profiles with fresh
stores at the same bounds on separate local HTTP/deployment ports. Keep failed
runs separate. Reuse unchanged signed service bundles; no userspace ABI,
protocol format or service source changes belong to this batch.

## Final-source execution

All five final-source runners exit 0, with no authoritative failed/pending
result, kernel panic or Raft persistence fail-stop. Passing runs request no
LLDB snapshot. They are not diagnostic captures of historical failures.

| Guest | Heap-held selection probes | Scheduler lifecycle | Authoritative result |
| --- | --- | --- | --- |
| Arm/SMMUv3 | 1.145362 s | 10.208271 s; certified runtime migration advanced to 1 | **17/0/0**, 10.208562 s, bitmap `0x1ffff` |
| Intel VT-d | 1.700555 s | 14.500711 s; never-dispatched-only policy, runtime phase explicitly skipped | **15/0/0**, 16.425492 s, bitmap `0x1bbff` |
| AMD-Vi | 1.187339 s | 12.965887 s; same x86 policy, two actual certified pre-dispatch boot moves | **15/0/0**, 14.744378 s, bitmap `0x1bbff` |
| Arm/SMMUv3 security | 2.063913 s | 12.392168 s; certified runtime migration required | **19/0/0**, 57.366532 s, bitmap `0x2003ffff` |
| Intel VT-d security | 1.282123 s | 13.650591 s; never-dispatched-only policy, runtime phase explicitly skipped | **17/0/0**, 61.853794 s, bitmap `0x2003bbff` |

Every guest completes its 64 exact-generation IPC claim/endpoint-close episodes:
**320 episodes**, together with **20 first-entry controls** and **20 forced
Ready-producer controls** across the four-LP matrix. Existing EL0 stack-growth
success, deferred-abort and foreign-leaf rollback modes also pass. No historical
IPC/stack timeout recurs in this bounded matrix; absence is not attribution.

Arm security confirms policy generations 1/2 at 35.130783 / 57.359470 s and
retires 14,900 concurrent cancellation requests. x86 confirms them at
39.597963 / 61.849330 s and retires 14,892 requests. These are scoped policy,
publication and exact-root retirement probes, not actual outstanding-I/O
hardware reset, broad pressure/fairness or physical-platform qualification.
No new linked deployment pair was run in this batch.

## Checks and artifact identities

The full host suite passes **488 Rust tests, zero failed, four ignored**, plus
its signing/tooling/boundary checks and **13 automatic-diagnostics tests**.
Native selector probes are kernel execution, not new host test counts. Strict
kernel Clippy passes both custom targets with `--no-default-features
--features acpi -- -D warnings`. Workspace rustfmt and whitespace checks pass;
boot runners verify native assembly sections. All local links in the changed
Markdown records are checked before completion.

The no-network kernels are rebuilt; Intel and AMD use the same x86 payload,
with AMD execution setting `CATTEN_SKIP_KERNEL_BUILD=1`. Security kernels are
rebuilt separately. Environment common to execution:
`CATTEN_SKIP_EMBED_BUILD=1 CARGO_BUILD_RUSTFLAGS='-D warnings'`.
Security runs additionally use `CATTEN_ALLOW_SANDBOX_NETWORK=1`, with Arm host
HTTP/deployment ports 18880/18881 and x86 18980/18981. All stores are fresh and
instance-local; QEMU's private diagnostic sockets require permission beyond the
filesystem sandbox. No service build or service Clippy result is claimed;
unchanged signed bundles have the identities retained in the
[persistence batch](2026-10-15-raft-persistence-diagnostics.md#artifact-identity).

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance scheduler-selection-final-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance scheduler-selection-final-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance scheduler-selection-final-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance scheduler-selection-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance scheduler-selection-x86-security
```

| Artifact | SHA-256 |
| --- | --- |
| Arm no-network kernel | `550a6ef823e8e385da324ee8711b4c175caaf82cf247c292cf84e7139906720f` |
| Intel/AMD no-network kernel | `2b123762da2a1c53bd88a9e7f64bf89fdb2809d3e9d085f3335d2e51493d34fb` |
| Arm security kernel | `7214663ce5cee0df5a3388822b676c7832fc1f7ac3d4a4dabf959c52888d06dc` |
| x86 security kernel | `101414f047ad3f47fd47284e4619d75f40d3335f061b89de85ce2e652bc9a376` |
| Arm no-network final UART | `39f6cf649185e273e21cf0c326696782abd36d2afa517f4171ea055607facdb7` |
| Intel no-network final UART | `0a8ddcdc9af88fe2874acfcd62a2b9873f4e3d2d1cc7fbf817f40f05d8d5345d` |
| AMD no-network final UART | `bb0ebb34d2af34da15100ac0333942171908629521060823792bbf8b1ca7f1b3` |
| Arm security final UART | `c2a826fab3d65bddd66363259680b713015867b2541acee0d36de255adbc6bae` |
| x86 security final UART | `eb4eefc23011fbf5843fa4c09a94c12d7797ac38a763e8116105b528f5c52eb6` |

Per-run manifests retain immutable executed ELFs. Kernel identities above are
for the final source, not the earlier fixture-error or sandbox-only launches.
The no-network manifests record tracked-diff SHA-256
`99c856d4a2a5abf0922915603b0ff698bcc2ce302d21e7ed676be4870465b1b5`;
that digest excludes then-untracked fixture files, so it is not a complete
source-tree identity. The eventual committed revision and actual ELF hashes
are the reproducible source/artifact anchors. Reports retain relevant results
inline rather than depending on ignored run outputs.

## Disposition and next boundary

Temporary selection storage is removed and the conservative x86 migration
restriction is enforced. This does not qualify destination queue allocation,
thread-table preparation under outer guards, statistics snapshots, general
kernel heap/arena metadata, byte/principal admission or essential-service
progress. Arm CPU ownership and same-LP stack reaping remain distinct proofs;
no x86 runtime handoff is added.

Next, prepare destination run-queue storage before queue mutation, preserving
the exact owning thread/generation and existing IRQ policy through rejection
and publication. The whole move must not remove its source before its
allocation-free destination publication is assured. Broader memory walker,
heap/image rollback, recovery custody and combined pressure/outstanding-I/O
qualification remain separate work. Eighteen owner families, seven gates and
the 38-finding accounting are unchanged; SEC-07/18 remain partial. A passing
matrix grants no physical cleanup, replay or restart authority and does not
resolve historical failure causation.
