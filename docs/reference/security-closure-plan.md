# SEC-07/18 closure work and batch acceptance

The work-package scope was baselined at `b15250b6` on 2026-11-04. Current
acceptance is recorded below. It maps existing
[R7/R18 requirements](security-remediation.md) to eight work packages;
it adds no findings and relaxes no acceptance gate. Existing admission and
cleanup qualifications remain valid in their recorded scopes. Production trust,
node authentication, security time and physical-platform qualification retain
their separate gates in the finding ledger.

The [inventory checkpoint](../reports/audits/2026-11-04-resource-inventory.md)
captures compiler-derived review candidates from both headless production
profiles. It is a starting list, not a finding count or a completed R7-1/R18-1
review. [Q1](../reports/audits/2026-11-04-syscall-snapshot-admission.md) and
[Q2](../reports/audits/2026-11-04-supervisor-launch-cache.md) are accepted; six
packages remain. This is a package count, not a weighted effort estimate.

## Existing coverage to preserve

The 24-pool [metadata inventory](metadata-admission.md), signed-principal
backing/stack/table admission, prepared payload nodes, original-class retention,
typed cleanup owners and producer-LP notification contracts are implemented.
The latest IPC snapshot/work-vector batch passed the full host suite, strict
test/minimal checks and four-CPU ARM 19/19, Intel 17/17 and AMD 17/17 campaigns.
These are local qualifications, not sustained pressure/recovery acceptance.

Do not treat an ordinary `Vec` or `Arc` spelling as a bypass. For example,
`completion/delivery/preparation.rs` already reserves CQ ring/backlog bytes via
`cq_budget`; object frame catalogs and IOMMU ledgers retain explicit capacity
reservations. Reconcile the inventory with their current owner contracts before
reopening them. Empty collection constructors and reference clones normally
allocate no backing at that site.

## Work packages

Statuses describe current acceptance, not estimated effort. Keep each package's
scope and exit test current; new discoveries must be assigned explicitly, with
an explanation if they change scope. Do not silently turn each source match
into a new patch or invent a new cleanup owner family.

| ID / status | Bounded scope and existing gates | Required exit evidence |
| --- | --- | --- |
| Q1 — accepted 2026-11-04 | Six primitive syscall input/scratch sites listed below; R7-2. Shared snapshot copying and byte admission implemented. | [Final qualification](../reports/audits/2026-11-04-syscall-snapshot-admission.md): captured original class, fallible complete prefixes, protocol-sized 12/16 MiB defaults, unchanged public input/status rules, full-capacity plaintext erasure and exact free/refund. Full host, strict test/minimal checks and ARM 19/19, Intel 17/17, AMD 17/17. Two failed arithmetic preflights preserved separately. |
| Q2 — accepted 2026-11-04 | Named supervisor launch/upgrade parents, the actual upgrade-syscall bridge, handoff arrays and fixed store-image cache; R7-2 and C18/G1/G4. Seeding distinguishes admission/I/O failure from repairable content. | [Qualification](../reports/audits/2026-11-04-supervisor-launch-cache.md): explicit one-shot cancellation and inert complete-parent abandonment, exact source/target generations, original-class arrays, pre-I/O cache publication/count admission and immutable byte ownership. Full host, final strict checks and ARM 19/19, Intel 17/17, AMD 17/17. Evidence distinguishes the final fixture-only gating correction. |
| Q3 — in progress | Remaining heap producer/factory candidates, library edges and profile/boot exclusions; R7-1/R7-2. Start from the frozen inventory and source screen, including PCI traversal, per-LP construction and launch encoders. The [boot allocation sub-batch](boot-allocation-boundaries.md) validates CPU counts early and corrects optional C-console ownership/allocation-failure cleanup. | Every relevant candidate assigned to an admitted owner, a required fix or a justified exclusion with trigger/bound/lifetime. Follow allocation-capable dependency calls and optional profiles. Compiled but unused helpers are not application-reachability findings. No unexplained raw allocator/factory or macro-origin gap. Framebuffer geometry/budget and the remaining factory assignments are still open; the sub-batch does not accept Q3. |
| Q4 — queued after Q3 | Aggregate heap and signed-principal coverage across the existing pool catalog, restart and quarantine; R7-3. | Application-triggered allocations have family/node and original-principal coverage or a documented applicable exclusion. Count allocator/control overhead explicitly. Transfer, weak retention, ASID reuse and uncertain cleanup cannot refund or redirect original charges. Show a coherent multi-pool capacity plan. |
| Q5 — review pending | Production caller/context and phase inventory for C01–C18; R18-1, G1/G3/G4. Use explicit/implicit Drop records plus callbacks and entry paths. | Record enclosing locks, IRQ state, allocation/notification/physical work, containing owner and unstarted/Pending/retryable/terminal outcome. Include ARM reaper context, scheduler/IRQ/observer callbacks and supervisor parents. Qualify actual callers; an inner post-guard helper is insufficient. |
| Q6 — queued after Q5 | Typed retained-owner recovery, supervisor correlation and claimed device-reset support; R18-3/R18-4, G2/G3/G5/G6. Scope starts with root custody and mapping/domain/device owners in C03/C05/C14. | Stable exact-episode custody and mutation authorization; reconcile cached supervisor failure only after all dependency proofs. Extend recovery only with an actual complete eligible owner; consumed errors stay terminal. Test live outstanding I/O, withheld completion/reset rejection, old-MMIO denial and restart/reassignment for each claimed backend. Keep unsupported paths fenced. |
| Q7 — queued, design required | Cleanup capacity and essential-service progress under saturation; R7-4 and SEC-06. IPC work admission at cleanup is a concrete known case. | Accepted work retains capacity needed for its promised cleanup, or a demonstrated bounded drainage policy establishes progress. No actor-based reclassification, force-refund or busy retry. Healthy management, timer/scheduler and ordinary teardown meet fixed budgets under real pressure. A larger byte limit alone does not close this package. |
| Q8 — blocked on relevant Q1–Q7 exits | Combined three-target pressure/recovery matrix and incident correlation; R7-5/R18-5, G7. | Execute the matrix below on fixed final sources, with real backing and exact retained/released accounting. Preserve failures separately from repeats. Investigate the Intel retirement-pin and ARM exit-watch incidents with exact owner/generation/phase diagnostics; passing repeats do not resolve them. Publish accepted gates and remaining blockers explicitly. |

Q3 and Q5 are review packages, not blanket requests to rewrite their subsystems.
If they expose authority-publication or physical-release changes, define the
specific call path and a separately reviewable sub-batch before editing. Keep
it within the parent package's acceptance record. Common custody extraction
waits for a second complete eligible adapter, as the cleanup contract requires.

Q3's [boot allocation sub-batch qualification](../reports/audits/2026-11-04-boot-allocation-edges.md)
records three corrections, full host/strict checks and ARM display 19/19,
Intel 17/17 and AMD 17/17. It preserves the failed upstream C allocation control
and unsupported device-tree compile separately. The remaining Q3 assignments
and framebuffer geometry/budget work are not accepted by those passes.

## Accepted batch Q1

These six sites are one batch because they produce bounded primitive byte
storage through shared code. The source paths and protocol bounds are fixed:

| Boundary | Code at this baseline | Retained bytes / existing rule |
| --- | --- | --- |
| Upgrade input image | `syscall::sys_spawn_upgrade`, `snapshot_bytes` at line 1853 | At most `MAX_ARTIFACT_ELF_SIZE`; original authorization and input cleanup remain applicable. |
| Scoped launch image | `syscall::sys_spawn_artifact_scoped`, line 1986 | At most `MAX_ARTIFACT_ELF_SIZE`; retain alongside the descriptor until launch validation finishes. |
| Scoped launch descriptor | Same handler, line 1991 | At most `deployment::MAX_DESCRIPTOR_LEN`; shares the image's complete operation. |
| Operational pickup | `syscall::sys_spawn_operational_connector`, line 2063 | At most `operations_pickup::MAX_PICKUP_LEN`; artifact/envelope slices borrow this storage. |
| Decrypted profile | Same handler, lines 2136–2138 | At most `operations::MAX_PROFILE_LEN`; erase initialized and spare plaintext capacity on ordinary disposal while retaining original admission. Abort erasure remains a separate SEC-15 limitation. |
| Signed shutdown envelope | `syscall::sys_request_node_shutdown`, line 2226 | Exactly `shutdown::ENCODED_LEN`; preserve signature, freshness and input-consumption checks. |

`memory::object::snapshot_bytes_in` already supplies the common validation/copy
boundary. Its remaining store caller belongs to Q2 because the image can enter
the permanent cache. Q1 must not accidentally refund a cached image or change
store ownership merely to simplify a buffer type.

The accepted defaults share the existing pool's byte dimension at **12/16 MiB
ordinary/total**, retaining 6144/8192 frame-buffer counts. Primitive buffers add
bytes and zero frame-buffer counts. Maximum artifact is 4194304 bytes; maximum
pickup is 4269180 bytes, and pickup plus plaintext is 4334716 bytes. Two maximum
pickup/plaintext lifetimes plus the former 3 MiB frame-snapshot allowance total
11815160 bytes and fit the new ordinary ceiling. The baseline 3/4 MiB bytes
would reject legal inputs. This permits an extra 9/12 MiB heap retention without
preallocation; shared competition and aggregate heap/principal sizing remain
open. See the [current capacity and ownership contract](snapshot-buffer-admission.md#primitive-syscall-inputs)
for the reviewed cost, heap envelope and alternatives.

Focused controls cover actual simultaneous maximum-sized copies/plaintext,
ordinary/trusted original class, generation reuse, pre-allocation quota denial,
partial-prefix failure, public input consumption and disposal. Optimized host
tracing observes full-capacity erasure while admission remains charged before
backend free, and rejects an unwiped control. The combined final host, strict
checks and three-target campaigns pass on frozen sources. Counter/error
adapters and the maximum-copy fixture are distinguished from authenticated
connector deployment and concurrent EL0 pressure in the
[acceptance record](../reports/audits/2026-11-04-syscall-snapshot-admission.md).
Q1 supplies no persistent-cache, supervisor-parent, physical/recovery or
saturated-progress qualification.

## Accepted batch Q2

The [launch/cache contract](service-launch-ownership.md) defines the accepted
parent, allocation and disposal boundaries. This deliberately scoped restructuring
shares actual syscall and supervisor preparation rather than adding a generic
cleanup registry. Handoff arrays use existing IPC bytes; permanent cache backing
has its own 16 MiB/64-entry pool so it cannot consume transient snapshot capacity.
The appended 24th pool requires statistics version 11 and matching rebuilt/signed
test/minimal service bundles on both architectures. Register/runtime ABI and
persistent object/signature formats remain unchanged.

The cache work also required fail-closed seeding: a quota/I/O/IPC/snapshot or
malformed reply cannot authorize overwriting installed state. That necessary
read/repair boundary belongs to Q2's cache preparation scope. Native controls
exercise signed store reads, unchanged cached hits and rejection under an
explicit serialized byte-pressure adapter.

Named launch/handoff parents retain profiles, exact roots, transfers and buffers
on abandonment, with ordinary cancellation tested separately. They do not make
bare `PreparedTransfer` or every containing destructor safe. Earlier loader
`PreparingDomain` cleanup, mandatory boot-only constructors, store IPC-cap/
lookup helpers, observer startup-claim publication and broader caller/IRQ/
termination paths remain assigned to the Q3 producer/exclusion review and Q5
context inventory. Typed continuation and saturated progress remain Q6–Q8.
Q3 must reconcile the old compiler census with accepted Q1/Q2 owners before
classifying residual candidates; do not reopen them from collection spelling.

## Combined acceptance matrix

This is a planned matrix, not executed qualification. Freeze exact case commands,
artifacts, seeds, offered load, deadlines and backend support before its first
run. The budgets below are initial fixture acceptance targets; failing them
requires diagnosis and an explicit revised plan, not a passing-repeat overwrite.
Production deadlines and invalidation policy do not change to satisfy a fixture.

- Targets: fresh four-CPU AArch64/SMMUv3, x86/Intel VT-d and x86/AMD-Vi security
  profiles. Retain the existing native controls. Add an eight-CPU AMD repeat for
  scaling/incident comparison; it cannot substitute for the three base targets.
- Load: 120 seconds after readiness, with at least eight ordinary EL0 clients
  from at least two independently signed principals. Choose domain/principal
  counts from the target pool's per-sponsor ceilings before execution; eight
  single-domain workers alone need not reach a node ceiling. Exercise actual object/image/heap,
  call/endpoint/CQ/observer and snapshot backing at ordinary and aggregate
  ceilings. Include cancellation, weak retention, retirement/restart and
  generation reuse. Record the first binding budget rather than bypassing it
  to force a different ceiling. Counter adapters supplement real backing
  pressure and explicitly cover otherwise unreachable counter dimensions.
- Progress: already-admitted healthy status responses within the existing
  three-second fixture bound; a fresh healthy status connection must finish
  within ten seconds while the client pool is occupied. Use a 100 ms
  timer/scheduler heartbeat with no gap over one second. Healthy ordinary
  fixture-domain teardown must finish within ten seconds after its stop request.
  Injected uncertainty must expose its retained Pending/failed owner state within
  that bound, never manufacture completion. Keep cooperative completion,
  waiter-admission rejection and physical failure outcomes distinct.
- Faults: combine live loan/copy/DMA work and thread switching with peer/root
  close, genuinely withheld/stale CPU acknowledgement, hardware completion
  timeout/reset rejection, interrupted publication and partial physical-release
  rejection. Every claimed device recovery backend must have actual outstanding
  I/O; merely enabling NVMe queues is insufficient.
- Outcomes: confirmed cleanup returns exact original family/principal charges;
  uncertainty retains the correct owner/fence/charge and forbids reuse. Check
  guard/IRQ state and later healthy progress, not just an error return. Capture
  exact generation, target, phase and cause for exit-watch/retirement failures.
- Outer budgets: 300 seconds per base campaign after guest start, including a
  30-second pressure-stop/drain phase. Any separate whole-node retirement case
  has a declared 90-second outer budget. Register all expected cases before
  execution; no missing, pending or failed case is an acceptance pass.

## Gate decisions and reporting

| Gate | Required packages | Current decision |
| --- | --- | --- |
| I — complete allocation/principal inventory with no unexplained application bypass | Q1–Q4 | Open; compiler census captured, manual/dependency/profile review and accounting remain. |
| C — qualified production cleanup contexts and typed continuation/recovery | Q2, Q5, Q6 | Open; current C01–C18 local controls are preserved, broader calls and recovery remain. |
| P — sustained essential-service and accepted-cleanup progress | Q7, Q8 | Open; finite isolated controls and safe quota denial do not establish saturation progress. |
| M — integrated QEMU pressure/recovery acceptance | Q8 and preceding required exits | Open; the combined matrix above has not run. |

Report package acceptance criteria completed, gates accepted, unresolved failures
and changed scope. Candidate counts, package counts and passing test totals are
not weighted readiness percentages. The earlier conversational implementation
estimate is provisional; this checkpoint supplies no measured percentage.
Completing this QEMU milestone still leaves R18-6 physical qualification and
the production trust/authentication/time/custody gates. Do not close SEC-07/18
or explain historical incidents merely by consolidating this checklist.
