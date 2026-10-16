# Audit process and outstanding failure review

Date: 2026-10-15. Reviewed baseline: `1109d05a` (clean working tree).
This review reconciles the audit/remediation records and selected current
diagnostic paths. It introduces no new SEC finding, changes no numbered
finding's disposition and makes no kernel or service change.

## Assessment

The remediation campaign has substantial implementation and execution evidence,
but is incomplete. The living ledger records **38 findings: 29 corrected within
their documented scope, six partial and three open**. That accounting remains
appropriate for this review; it does not measure effort remaining or establish
production readiness. Original audit verdicts describe their reviewed revisions,
and old remaining-work lists must not be combined into today's backlog.

Recent panics have several distinct dispositions. Some have source-supported
corrections and negative execution; some expose invalid fixture assumptions;
others remain unexplained intermittent failures. Later passing boots do not
erase the latter. In particular, the IPC exact-exit timeout has reproduced on
**both architectures**, and neither scheduler correction establishes its
historical cause. Later Raft persistence errors remain separate observations.

Subsequent same-day work addresses the lost-error diagnostic gap described
below: [Raft persistence diagnostics](2026-10-15-raft-persistence-diagnostics.md)
retains typed per-call failures and distinct write/flush context, with host and
native persistence validation. The original observations and historical causal
limits in this review remain unchanged.

The later [automatic capture follow-up](2026-10-15-automatic-failure-capture.md)
addresses the cross-architecture capture tooling gap. It preserves immutable
run inputs and all-LP state on a failed current run; it cannot recreate the
earlier missing captures or close their causal questions.

The [current-build matrix](2026-10-15-captured-park-fixture-panic.md) subsequently
captured a spontaneous ARM panic in the park-delivery fixture. A forced-switch
negative control reproduces its invalid Running-only producer assertion;
incoming-context delivery may correctly observe a Ready publisher. This newly
attributed fixture defect is separate from the historical exit/persistence
incidents below.

## Scope and authoritative records

The directory contained 122 audit Markdown reports before this report was added,
including July/August reviews. This review inventories those records, follows
the October 3/5/10 security audits into their remediation ledgers, checks earlier
remaining-scope sections against later work, and traces the recent failure and
validation reports. This is not a fresh exhaustive security audit of every
source file or a revalidation of every historical closure.

Use these records for different purposes:

| Record | Authority and scope |
| --- | --- |
| [Security remediation](../../reference/security-remediation.md) | Numbered SEC dispositions and finite R7/R18 completion criteria. |
| [Cleanup/recovery](../../reference/cleanup-recovery.md) | Eighteen owner families C01–C18, seven cross-category gaps G1–G7, and exact continuation/retention rules. A row is not complete caller-context qualification. |
| [API simplification](../../reference/api-simplification.md) | Completed protocol/API batches and remaining candidates. A completed batch does not close SEC-07/18. |
| [Original audit](2026-10-03-security-audit.md), [October 5 audit](2026-10-05-security-audit.md), [October 10 audit](2026-10-10-security-audit.md) and individual follow-ups | Historical findings, failed executions, artifact identity and scoped validation. |

The July/August reports are historical inputs, not additional findings included
in the 38-SEC count. No new disposition for all their individual findings is
claimed here.

## Remaining security work

| State | Findings | Remaining acceptance boundary |
| --- | --- | --- |
| Partial | SEC-04 | Protected production boot/enrollment, rollback-resistant installed state, independent authorities and recipient-key custody. Signed policy and disposable x86 QEMU Secure Boot evidence exist; they do not supply production provisioning. |
| Open | SEC-08, SEC-10 | Authenticated/encrypted node and management channels, enrolled identities, authorization, replay/session fencing and revocation. Signed mutation payloads do not authenticate the transport or readers. |
| Open | SEC-09 | Authenticated security-time provenance, freshness/uncertainty and fail-closed expiry across loss, rollback and restart. |
| Partial | SEC-06 | Concurrent healthy-client fairness and sustained management/network pressure. Listener retention, bounded bodies and five-second publication/controller RPC policies are implemented; these do not establish controller-internal or transport progress. |
| Partial | SEC-07 | R7-1 allocation inventory; R7-2 remaining scheduler/registry storage; R7-3 general heap and aggregate principal budgets; R7-4 essential progress; R7-5 concurrent real-backing pressure matrix. Existing individual pools are substantive progress. |
| Partial | SEC-18 | Complete outer guard/IRQ/implicit-drop inventory, phase-correct retained owners, supervisor reconciliation/custody, wider device reset coverage with outstanding I/O, and integrated concurrency/failure qualification. R18-2 cooperative-page ownership is implemented; R18-3 final-root recovery is bounded and partial. R18-1–5 are the QEMU milestone; R18-6 physical-platform qualification remains separate. |
| Partial | SEC-14, SEC-15 | Advisory/license and release dependency/provenance enforcement; remaining secret-derived library/compiler-copy inventory. Locked builds, pinned actions, file-only signing keys and specific zeroizing owners already exist. |

The 29 scoped corrections are SEC-01/02/03/05/11/12/13/16/17 and SEC-19–38.
SEC-12 was legitimately reopened by verification and corrected again; the
[follow-up](2026-10-10-security-object-store-follow-up.md) preserves both states.
Do not reopen implemented table/stack admission, hidden IPC delivery or scoped
launch checks merely because an older report lists them as missing. Conversely,
the scoped SEC-11 disposition does not qualify every separate live-upgrade or
platform-service trust path.

## Failures with a concrete correction or corrected fixture premise

| Observation | Evidence and current disposition |
| --- | --- |
| `live_upgrade_test` boot panic, SMMU `Unsupported` | [Initialization regression](2026-10-10-security-live-upgrade-iommu-init.md): removal of lazy initialization left this feature without required early initialization. Boot gating corrected; ARM upgrade and ordinary security execution passed. x86 upgrade was compiled/linted, not guest-executed in that batch. |
| Timer-sponsor equality panics (ARM 4 versus 3; x86 5 versus 2) | [Raft batch](2026-10-13-protocol-simplification-raft.md): kernel threads shared their address-space-generation sponsor; masking one LP did not isolate the aggregate. A private trusted fixture sponsor is prepared before publication/event capture. This corrects the test premise without explaining individual historical deltas or proving a production leak. |
| Global lifecycle `try_lock` assertion; one-ms timer admission; missing CQ | [Mutex probe correction](2026-10-10-security-object-store-follow-up.md), [isolated timer probe](2026-10-13-protocol-simplification-launch.md), [checked CQ fixture](2026-10-13-protocol-simplification-cq-wait.md): replace false global-lock/isolation expectations and supply required fixture resources. These failures do not by themselves establish production lock leaks or invalid wait semantics. |
| Retired NS metadata requests and ASID-zero status-fixture retirement | [Bearer removal](2026-10-15-ns-bearer-removal.md) and [status snapshot](2026-10-15-ns-status-snapshot.md): current CNSGEN01 fixture and valid captured caller replace obsolete requests/invalid identity. Production rejection checks remain. |
| CI SITAS result deadline, with missing panic frames | [CI pin/diagnostics](2026-10-15-ci-sitas-runtime-pin.md): old Sitas `94eca685…` called retired unchecked waits 12/42 under runtime ABI 3. All six checkouts now pin `d27f7e7b…` using checked 88. Global RUSTFLAGS had also shadowed forced frame pointers. An old-source negative run reproduced 17/1/1; corrected local equivalent profile passed 19/0/0, and the failing run produced symbolized frames. A subsequent GitHub job is not verified by this review. |
| Producer delivery stranded at park publication | [Park correction](2026-10-14-park-deferred-delivery.md): drain admitted producer-LP delivery under the guard-free park mask before Blocked publication. ARM negative execution with the drain omitted fails `park stranded producer delivery`; corrected linked runs pass. Historical IPC/persistence attribution stays open. |
| Fresh context bypassed post-switch yield continuation | [First-dispatch correction](2026-10-15-first-dispatch-delivery.md): both switch assemblies complete staging/deferred delivery on the incoming stack before restoring its frame. ARM negative execution fails before entry; four corrected guests pass 256 IPC close/exit episodes and 16 first-entry controls. This establishes the ordering defect and tested correction, not the historical timeout's cause. |

The last two deliberately failing controls are expected negative evidence.
They must not be counted as unexplained current regressions or substituted
for a reproduction of the original timeout.

## Unresolved incident register

These review-local entries group observed failure families; they add no SEC
numbers. Each linked version-controlled report retains diagnostics and artifact
hashes. Similar timing or a shared downstream verifier is not proof of one cause.

| Failure family | Strongest retained observation | Status and next useful evidence |
| --- | --- | --- |
| IPC close worker exact-generation exit | [Investigation](2026-10-14-ipc-close-exit-investigation.md), [reproduction](2026-10-14-protocol-simplification-name-generations.md): original x86 at 20.265305 s; later x86 at 21.321480 s had returned=1/stage=4, closed source, zero registrations, no live entry and a staged unstarted owner. ARM at 13.952917 s also had returned=1/stage=4 but remained Running on LP2, with no staged node. | Open. The original 192 passing episodes preceded the later reproductions. Park/first-dispatch corrections and subsequent passes improve confidence but do not establish historical attribution. Capture exact exit publication/observation, dispatch/retirement phases and LP state on a failing current build. Keep the five-second completion requirement and original owners. |
| Memory-close worker exact exit | [DMA-map ownership report](2026-10-12-dma-map-failure-ownership.md): Intel at 23.429878 s failed `memory/object/close/tests.rs` five-second exit watch after dropping the transfer that should produce its wake. No authoritative complete result; HTTP progress also failed. | Open. Related scheduler mechanisms are plausible investigation targets, not an established common cause with IPC. Retain this separate source/worker episode and capture its exact phase on recurrence. |
| User-stack retirement lease | [Lease investigation](2026-10-10-security-user-stack-lease-timeout.md): Intel and AMD root close returned OperationsInFlight until the fixture's ten-second deadline expired. Nineteen pre-correction stress runs did not reproduce it. | Open attribution. Three bounded fresh invalidation rendezvous attempts before terminal pair retention address a possible transient class; they do not identify the historical phase. Use retained node generation/root/LP, started/error, stack counters and shootdown observations to distinguish deferral from failed release. |
| Raft persistence fail-stop | [Registration failure](2026-10-14-protocol-simplification-dns-registration-relay.md), [NS reproduction](2026-10-14-protocol-simplification-name-generations.md): `FATAL: failed to persist Raft log state` at 30.528249 s and 28.191274 s, after distinct IPC failures. | Open independently. This is a service fail-stop, not itself a kernel panic. Current persistence code still merges write and flush rejection and loses underlying client errors; capture operation, object, phase and exact status before inferring storage corruption or scheduler causality. |
| NIC DMA grant/map startup | [Startup preparation](2026-10-12-dma-startup-preparation.md), [map diagnostics](2026-10-12-dma-map-failure-ownership.md), [map readiness](2026-10-14-dma-map-readiness.md): older Intel grant and AMD three-page-map causes were collapsed; later x86 records identify BACKEND/OPERATION_IN_FLIGHT (`0xc030e`). | Captured later map-admission gap corrected for qualified scheduled callers, with real controls and two passing linked pairs. Original contender and older failures remain unattributed. Masked ARM SVC busy waiting, unmap/close readiness, AMD-Vi runtime and broader pressure remain separate qualifications. Never replay a map based on diagnostic status. |
| AMD NIC early exit/router deadline | [Publication report](2026-10-12-security-network-publication.md): NIC ASID 106/thread 14 exited at 11.581 s; router later waited at lookup stage 1 and expired at 42.121 s. | Open historical cause. Endpoint-preserving publication and first-failure diagnostics correct source-confirmed behavior but the failed capture lacks the NIC's actual rejection branch. Do not merge this older exit with later diagnosed DMA contention. |
| Intel object-store readiness/read failure | [Readiness report](2026-10-12-security-object-store-readiness.md): `0xd1`, supervisor panic, 15 passed/3 pending; exact read/device result absent. | Open historical cause. Explicit Serving publication, first-I/O records and unreadable-superblock rejection correct concrete policies. No new failing first-I/O trace was obtained in that follow-up. Passing disk reuse does not establish the missing read cause or power-loss durability. |
| ARM silent stall | [CQ batch](2026-10-13-protocol-simplification-cq-wait.md): output stopped at 14.454350 s; no final result in the 300-second window. No contemporaneous LP/IRQ/stack capture. | Open. A debug-enabled passing repeat is not a snapshot of the failed run. Preserve all-LP registers/stacks and exact payloads on recurrence; x86's consequent missing-peer deadline is a separate downstream symptom. |
| x86 HTTP EOF/reset recovery | [Registration/profile batch](2026-10-13-protocol-simplification-registration-profiles.md): guest suite and rollout succeeded, host EOF recovery exhausted retries with connection resets; failed run exited 1. | Open transport/harness qualification. Later success establishes eventual progress, not the failed reset's cause. Record TCP/socket/owner phases and host-probe timing separately from guest self-test completion. The runner's tick budget excludes synchronous probes and is not a hard wall-clock bound. |

## Diagnostic and process gaps

Diagnostics are materially better than the earliest captures. Current CI retains
kernel frame pointers; runners hash executed kernels, save symbol sidecars and
reject panic/incomplete self-test results. IPC fixtures retain exact source
interest and log worker phase, TID/generation, live LP and staged owner. Stack
fixtures log retained-release state. NIC and object-store first failures now
have explicit stages. These mechanisms cannot reconstruct earlier missing data.

Selected current source confirms two useful limits:

- [Raft persistence](../../../crates/catten-services/src/disk_raft.rs) uses
  `write(...).and_then(flush).is_err()` and one fail-stop message;
  [ObjectStore/Client](../../../crates/catten-services/src/objstore_client.rs)
  reduces errors to `()`. A symbolized fail-stop alone cannot identify allocation,
  submission, wait, reply, device write or flush rejection. Typed per-operation
  diagnostic propagation is the next bounded investigation task here; preserve
  fail-stop and accepted-request ownership, without adding persistence replay.
- ARM's [runner](../../../scripts/run-aarch64.sh) provides opt-in
  `--debug-snapshot`/scheduler trace and captures near the end of its test window.
  The [x86 runner](../../../scripts/run-x86_64.sh) exposes GDB options but no
  equivalent `--debug-snapshot` flag. Both share panic symbolization. Automatic
  timely capture on failure, cross-architecture parity and durable publication
  of relevant evidence remain useful tooling work; a late successful snapshot
  does not diagnose an earlier failing sample.

The living security ledger still said the IPC investigation had not reproduced
the timeout; the API candidate row repeated that earlier checkpoint. Those
statements are superseded by the NS generation reproductions. This review
updates those summaries and links the later park/first-dispatch/CI evidence.
Earlier reports and failed samples remain unchanged. The cleanup map's header
also lagged its later entries; its C16/C18 summaries now point to this sequence.

For each subsequent failed run, publish the exact reviewed revision/dirty-state
qualification, target/features/runtime/dependency revisions, kernel and signed
bundle identities, storage freshness, LP/peer topology, first upstream failure,
raw/symbolized diagnostic frames or all-LP stall capture, authoritative result
and runner/host-probe outcomes separately. Record what a correction proves and
the acceptance boundary still missing. Keep failed samples separate from
repeats; never derive a cleanup receipt or retry permission from a wake, worker
return, absence or timeout. Use version-controlled evidence or concise inline
captures rather than links to temporary logs.

## Recommended next work

1. Make the next IPC/memory-close/stack failure diagnosable on both architectures
   with current payload identities and exact phase/exit/retirement state.
   Keep both scheduler negative controls enabled. Define a bounded current-build
   Intel/AMD/ARM run matrix and completion criteria before execution; passing
   repeats qualify that matrix, while historical attribution may remain unknown.
2. Preserve distinct write/flush and underlying object-store failure diagnostics
   through the existing Raft operation owner. Diagnose the retained fail-stop
   independently before relating it to IPC or reformatting any store.
3. Resume SEC-07/18 against R7/R18 and G1–G7 acceptance gates: outer-context and
   allocation inventories first, then owner-preserving recovery where justified,
   outstanding-I/O device tests and integrated pressure/progress qualification.
   Further API removal should have a specific boundary and scoped completion.
4. Keep SEC-04/08/09/10 as explicit production milestones. The living ledger's
   order remains trust/enrollment, authenticated channels, then security time;
   isolated cleanup successes cannot discharge these missing boundaries.
5. Refresh dependency/security evidence when assessing a release. The latest
   checked-in advisory result is [October 10](2026-10-10-security-audit-dependencies.json),
   zero matches for 177 dependencies, using database commit
   `7eebec69c352c7191b1f13eb95dd510eeca5d1de` updated October 9. No fresh advisory
   database scan or license audit was performed in this review.

## Validation performed for this review

At `1109d05a`, `scripts/run-host-tests.sh` completed with exit 0:
**483 Rust tests passed, zero failed, four ignored** across 55 harness summaries.
Signing-policy checks, five boot-result tests, four secure-boot-result tests,
four CI runtime-input tests, six tooling tests, five store-format tests, the
86-call-site/nine-arity emitter inventory and signer self-tests passed.
The CI-input checks verify the pinned Sitas backend and target frame-pointer
configuration from source; they are not a new GitHub execution.

SHA-256 identities for this run's inputs:

- `Cargo.lock`: `c50350ae2dcde20088d5ce11869c1b8a821c57d6c4000414b83c0ac574298cdb`
- `scripts/run-host-tests.sh`: `7d6408a8b677e24c3c0cc68c19e1cdb59ede68ceac93058ac98abbcae5f10ebb`

This review ran no new guest boots, service rebuild/signing, Clippy, physical
fault injection or advisory update. Native results above are explicitly linked
historical execution evidence. Documentation links and diff whitespace were
checked after editing; no production behavior or ownership contract changes.

## 2026-10-16 IPC park-observation fixture follow-up

The [backlog refund batch](2026-10-16-completion-backlog-refunds.md) preserves a
fresh Intel failure at the IPC claim fixture's registration-implies-Blocked
assertion, including concise all-CPU diagnostics and artifact hashes. Scheduler
registration intentionally precedes Blocked publication. The corrected fixture
waits for both observations for the exact worker generation under one original
five-second deadline; it still requires exact exit completion before root close.
A deterministic registered-but-Ready scheduler control exercises that legitimate
window. The capture lacks the exact worker state/generation at the assertion,
so later idle CPU stacks cannot reconstruct that instant. This source-confirmed
fixture defect is separate from the older exact-worker exit timeouts and does
not establish their cause or physical quiescence. Failed and passing runs remain
separate in the linked report.

A second Intel sample in that batch reached its 180-second deadline. All-CPU
stacks identify a heap/registry lock cycle introduced by a heap-held CQ waiter
probe after concurrent boot workers started. The probe moves to the serialized
pre-scheduler phase without removing its checks. The production detached-map
allocation under the registry remains open. This is a newly diagnosed fixture
cycle, not attribution of the older intermittent incidents; its own failed
capture and input hashes remain separate from passing repeats.


The [detached metadata batch](2026-10-16-detached-completion-metadata.md)
subsequently removes that captured detached-map allocation/removal path using
prepared admitted nodes, preserving them with original charges through backlog
and post-guard retirement. Complete namespace and operation fallbacks retain all
fields; ordinary close/replacement explicitly drains nodes. Capability/CQ/global
maps and wider contexts remain open; this does not attribute older incidents.


The [capability completion metadata batch](2026-10-16-capability-completion-metadata.md)
subsequently converts the other per-operation map, carrying payload and detached
authority in one original owner through post-guard disposal. It also corrects a
source-proven terminal-abort backlog gap and authority charge/deallocation order.
CQ/global maps and wider contexts remain open; none of these source fixes
attributes the historical intermittent incidents.

The [CQ metadata batch](2026-10-16-cq-metadata.md) continues the same local
boundary with prepared admitted queue nodes. Fresh publication and replacement
allocate/free no queue metadata under COMPLETIONS. One retired owner keeps
node/source/ring/backlog/original charge until explicit post-guard disposal;
guarded fallback retains all fields. The global namespace map remains the next
metadata boundary. Finding counts and historical intermittent attribution are
unchanged; this adds no physical proof, recovery custody or replay permission.

The [global completion namespace metadata batch](2026-10-16-completion-namespace-metadata.md)
replaces the last completion BTreeMap with shared admitted nodes. Preparation
includes fallible account allocations; replacement/close keeps the global node
and complete payload until explicit post-guard disposal, with global metadata
freed before original accounts. The local map-storage sequence now covers
capability records, detached records, CQs and global namespaces. This does not
bound empty namespaces/account storage or qualify every enclosing timer,
observer, caller and IRQ context. Those are the remaining boundaries; historical
intermittent causes and finding counts remain unchanged.

The [timer queue-disposal batch](2026-10-16-timer-queue-disposal.md) follows the
four completion maps by handing removed event nodes and inline keyed payloads
out of the local queue borrow in one retaining receipt. Explicit disposal is
iterative; fallback retains source/callback fields and original admission.
This closes the local node-destruction boundary, while callback execution,
outer scheduler/producer guards, IRQ disposal and unpublished preparation
remain separately qualified. The private negative control retains one event
without resetting its counter. Finding counts and historical failure causes
are unchanged.

The [timer callback-delivery batch](2026-10-16-timer-callback-delivery.md) moves
the next local boundary beyond the queue borrow: a captured due prefix owns
callback/waiter delivery, temporary observer destruction and disposal on its
original masked LP. An inline claim fences recursion, new enqueue waits for
the next pass, and cancellation still checks the exact retained state before
signal. Original charges and abandoned selected/purged owners remain retained.
IRQ execution, outer caller/producer guards, unpublished preparation, final
handles and integrated pressure are still separate work. Finding counts and
historical failure causes are unchanged.
