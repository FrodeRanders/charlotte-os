# Security status source review

Date: 2026-10-17. Reviewed baseline: `3a0454f86f15195766a950b41f21911f5cfd501b`.
The working tree already contained documentation edits; they were preserved.
No kernel, runtime, service or test source was changed by this review.

## Assessment

The security work is substantive, but CharlotteOS remains an experimental
system with explicit production blockers. The current ledger's **38 findings:
29 corrected within their documented scope, six partial and three open** is
consistent with the sampled source. This review does not change dispositions
or introduce a new numbered finding. Correction counts are not a readiness score.

Use the [living ledger](../../reference/security-remediation.md) for current
acceptance criteria. The October 3/5/10 audits are historical checkpoints;
combining their old remaining-work lists would incorrectly reopen corrections.
The [process review](2026-10-15-audit-process-review.md), its later follow-ups
and the October 16 ownership reports preserve both progress and failed samples.

## Corrections checked directly

| Boundary | Current source observation | Limit of this check |
| --- | --- | --- |
| SEC-31, fragment reassembly | [`relmsg::handle_frame`](../../../crates/catten-services/src/bin/relmsg.rs#L304) rejects invalid/over-ceiling fragments before peer admission or session replacement; assembly remains behind the ceiling check. | This does not authenticate peers or establish fairness under a sustained frame flood. |
| SEC-32, TCP/IP transmit and receive backing | [`CharlotteEthDevice`](../../../crates/charlotte-smoltcp/src/lib.rs#L215) returns no TX token until scratch backing exists. [`tcpip`](../../../crates/catten-services/src/bin/tcpip.rs#L770) adopts moved caller buffers for receive/status into `PendingReceive`, rather than allocating service-sponsored reply objects. | General heap pressure, socket progress and slow/hostile-client fairness remain separate. The adapter still has a documented raw hardware/protocol boundary. |
| SEC-36, discovery growth | [`learn_peer`](../../../crates/catten-services/src/bin/disco.rs#L452) checks the shared admission rule before insertion; the rule caps new peers at [64](../../../crates/catten-services/src/lib.rs#L2090). | Spoofing, occupying all slots and trusted peer enrollment remain SEC-08 work. A bounded map is not peer authentication. |
| SEC-37, public naming | Both [immediate and deferred public lookup](../../../crates/catten-services/src/bin/ns.rs#L733) deny policy-published registrations. | Raw receive/watch/transport contexts and the rest of NS authorization were not exhaustively re-audited. |
| SEC-38, upgrade image ceiling | [`sys_spawn_upgrade`](../../../crates/catten/src/syscall/mod.rs#L1942) bounds supplied ELF size before snapshot allocation and restricts the caller to the current manager identity. | Separate platform/live-upgrade trust and physical device recovery are not qualified by this check. |
| Borrowed RPC lifetime | [`PendingCall`](../../../crates/catten-rt/src/owned/ipc.rs#L1102) retains itself on close rejection and timed waiter-admission rejection. Drop aborts the domain if close cannot safely end its borrow. | This is fail-closed ownership, not recovery or a guarantee that teardown always progresses. |
| Persistence diagnostics | [`persist_record`](../../../crates/catten-services/src/disk_raft.rs#L334) distinguishes encoding, write and flush failure, retains exact operation context and does not replay a write after rejection. | The earlier lost-error diagnostic gap is corrected. Historical persistence failures and power-loss durability remain separate questions. |

These are source checks, not newly executed exploits or revalidation of all
29 corrected findings. In particular, no new claim is made about every ELF,
IOMMU, authorization or capability-transfer path.

## Remaining boundaries confirmed in source

**Trust and channels (SEC-04/08/10).** [`build.rs`](../../../crates/catten/build.rs#L15)
explicitly rejects production trust mode. Normal boot selects
[`PreparedBootTrust::development`](../../../crates/catten/src/service/launch.rs#L979).
The [one-shot handoff](../../../crates/catten/src/service/admission.rs#L29)
checks policy/key binding but deliberately requires its caller to supply
authenticated boot, protected installed state and key custody. Disposable
Secure Boot fixtures do not provide those production facilities.
The reviewed reliable-message/discovery paths have session/sequence and size
checks, but no enrolled cryptographic peer authentication. The HTTP services
accept ordinary socket requests: [`httpd`](../../../crates/catten-services/src/bin/httpd.rs#L1508)
routes read-only state requests without operator authentication, while
[`deployd`](../../../crates/catten-services/src/bin/deployd.rs#L356) forwards signed
mutation payloads and status queries. Payload signatures do not authenticate
the transport or its readers. An Internet-facing management security claim
would therefore exceed the implemented boundary.

**Security time (SEC-09).** The time service still parses
[SNTP responses](../../../crates/catten-services/src/bin/time.rs#L551) using
protocol/originate/timestamp checks, without authenticated security-time
provenance. The [operational launch gate](../../../crates/catten/src/syscall/mod.rs#L2285)
still compares expiry with `pickup.now_unix_seconds`. That value is supplied in
the pickup package by the authorized deployment agent, not derived from a
kernel-authenticated wall clock. This is an existing trust/time limitation,
not evidence that an arbitrary unprivileged caller can invoke that gate.

**Resource admission (SEC-07).** Individual stack/table/completion/timer pools
and their lifetime owners exist. General heap and aggregate logical-principal
admission are still missing acceptance boundaries. Two concrete remaining
allocation paths deserve entries in R7-1/R7-2:

- `SPAWN_THREAD` reaches `Thread::try_new`, the architecture's user context,
  `Stacks::user`, and [`allocate_stack_locked`](../../../crates/catten/src/memory/allocators/stack_allocator.rs#L178).
  After mapping stack backing, that helper inserts both guard addresses into a
  `BTreeMap` through infallible `entry(...).or_insert(...)`, while the stack-arena
  mask/lock and guard map write hold are active. Stack admission bounds backing;
  it does not make these metadata allocations fallible. This path cannot yet
  substantiate graceful rejection under actual kernel-heap exhaustion. No OOM
  exploit was executed in this review.
- [`Ledger::account`](../../../crates/catten/src/memory/budget.rs#L72) lazily
  inserts generation accounts in another `BTreeMap` and creates waiter/timer
  sponsors. `WaitSponsor` uses an infallible `Arc::new`; the scheduler timer
  sponsor uses the mandatory account constructor. Callers such as
  `reserve_inner` and `waiter_sponsor` hold the address-space table and ledger
  while acquiring this metadata. Existing object count/page limits do not
  qualify allocation failure or aggregate principal lifetime for it.

The [global heap](../../../crates/catten/src/memory/allocators/global_allocator.rs#L39)
has a finite boot-mapped arena/reserve. That physical bound is not a
per-principal budget or proof that essential platform allocations remain usable
when ordinary work pressures the same allocator. These observations continue
the existing SEC-07 finding; they do not reopen corrected stack/table admission.

**Cleanup composition (SEC-18).** The newest
[`WaiterSource::Preparing`](../../../crates/catten/src/klib/observer/waiter_source/preparation.rs)
retains sponsor, weak callback, candidate, selected list and child preparation
together behind `ManuallyDrop`, with explicit ordinary finish. This matches
the scoped [lazy-source report](2026-10-16-lazy-waiter-source-ownership.md).
Active destruction is a different boundary:
[`Registration::drop`](../../../crates/catten/src/klib/observer/registration.rs#L298)
removes an entry, and `NotificationBatch` disposal releases weak references,
node storage and charges. [`WaiterSource::drop`](../../../crates/catten/src/klib/observer/waiter_source.rs#L105)
closes/disposes its list. Releasing each helper's own lock does not qualify all
enclosing guards or IRQ masks. This confirms the reports' stated remaining
context inventory; it is not a demonstrated deadlock or historical attribution.
Terminal retention protects safety but consumes capacity and supplies no
supervisor reconciliation, physical completion or restart permission.

**Availability (SEC-06).** Both HTTP services process a connection through
their serving loop before accepting another. Deployment header/body and
controller deadlines limit particular phases, but do not demonstrate a healthy
client's progress while hostile clients continuously occupy the service. Socket
accept/receive still includes protocol polling/backoff; this review does not
describe those paths as wholly event-driven. Concurrent hostile pressure and
essential-service latency remain R7-4/R7-5/G7 work.

SEC-14/15 also remain partial: pinned/locked builds and specific zeroizing
owners are implemented, but CI-wide advisory/license/provenance enforcement
and complete secret-derived copy/erasure coverage are not established here.
No fresh advisory scan was run; the October 10 dependency result is historical.

## Validation and evidence

Fresh `bash scripts/run-host-tests.sh` completed with exit **0**: **509 Rust
tests passed, zero failed, four ignored**, across 57 harness summaries, plus
signing-policy, Python/tooling controls and cluster-sign self-tests.
`cargo fmt --all -- --check` also exited **0**. Report link and changed-file
whitespace checks pass. Exact source hashes and concise results are recorded
in the accompanying [review evidence](evidence/2026-10-17-security-status-review.json).

The [latest native evidence](evidence/2026-10-16-lazy-waiter-source-ownership.json)
records five successful four-LP runs: ARM/SMMUv3, Intel VT-d and AMD-Vi without
network, plus ARM and Intel security profiles. This review independently
matched **all eight reviewed-source hashes and six unchanged-tooling hashes**
in that manifest against the current files. These matches support continuity
for those exact files, not a whole-tree attestation or a fresh guest run.

No new guest matrix, physical hardware execution, forced backing exhaustion,
network attack, concurrency model exploration or power-loss test was performed.
The kernel disables the ordinary host test harness; most kernel integration
fixtures require native guests. Host success cannot substitute for those runs.

The [unresolved incident register](2026-10-15-audit-process-review.md#unresolved-incident-register)
still matters: historical exact-exit/stack timeouts, Raft fail-stop, NIC/storage
startup failures, ARM stall and HTTP reset observations are not explained by
later passes. Separately diagnosed scheduler and fixture corrections must keep
their own evidence. No causal closure is claimed here.

## Next work

1. Finish a finite R7-1 allocation/caller-context inventory, starting with the
   concrete stack guard and sponsor-account sites above. Qualify published
   observer/token/source destruction alongside transient preparation.
2. Establish aggregate principal/heap limits and run concurrent real-backing
   pressure with predefined management/timer/teardown progress deadlines.
3. Keep failure captures and exact operation identities for unresolved incidents;
   retain safety fences and original charges until typed completion permits
   release. Complete R18 composition/reconciliation and outstanding-I/O tests.
4. Treat protected enrollment/custody, authenticated node/operator channels and
   authenticated security time as explicit production milestones. Local cleanup
   successes cannot discharge them; QEMU does not qualify physical devices.
