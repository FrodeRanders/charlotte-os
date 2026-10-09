# Published DMA creation rejection retains its enclosing grant owner

This extends the [grant rollback correction](2026-10-09-security-dma-grant-rollback.md)
within C18/C14/C15/G3 of the [cleanup/recovery strategy](../../reference/cleanup-recovery.md).
The audit ledger remains **20 corrected, six partial and four open**. Published
creation rollback now uses the existing unlocked destruction boundary; initial
creation/reset waits, private construction and full outer contexts remain open.

## Correction and owner continuity

Previously, a backend returned a domain identifier only on successful creation.
After hardware descriptor publication, failed initial configuration maintenance
triggered another configuration/TLB/drain attempt and physical table release
under the backend guard. Failed rollback retained backend backing but returned
only an error. The enclosing grant could then refund its capability reservation
and finish its root without retaining the failed registered domain obligation.

Creation now borrows a non-copyable `DmaCreation` embedded in the enclosing
`PreparedDmaDomain`. This preparation already owns the exact root operation and
capability reservation. Each backend admits the complete domain into its
registry/source slot, marks table ownership published, and records the rollback
obligation in that preparation **before** a reachable context/DTE/STE write.
Creation returns `Result<(), Error>`; there is no success-only scalar ownership
return or compatibility branch. An already-armed creation owner rejects before
backend initialization, reset or allocation, without replacing its obligation.

On initial configuration error, the backend marks the registered domain retiring
and publishes its rejecting descriptor under serialization. SMMUv3 uses the
existing publication-only abort operation, leaving CFGI/TLBI/SYNC to explicit
rollback. None of the three backends retries hardware maintenance, frees table
backing or logs this published rollback beneath its registry. Its error returns
with the enclosing preparation's obligation still armed.

The production adapter preserves NVMe reset's current failure behavior: bus
mastering stays disabled, and the reset/config and device locals leave before
the grant's lifecycle guard leaves. Only then does the complete grant owner
invoke its one-shot explicit cancellation. It uses the same actual-engine
`Maintenance` and `DetachedDomain` path as ordinary destruction: exact command
state, registered slot/source fencing, backend-specific drain/completion and
unlocked physical release. Intel busy registers, AMD strict completion epochs
and Arm queue state retain their existing timeout rules. No command snapshot,
queue rewind, fabricated acknowledgement or new registry is introduced.

Confirmed destruction permits capability-reservation refund and root completion.
Rejected cleanup retains the original root/reservation/obligation together with
the backend's retiring domain and original table charges. Partial physical release
remains frozen/terminal. The enclosing publication owner cannot retry after
cancellation starts, even if a backend maintenance rejection restored its engine
and domain. That backend state alone is not a complete retry owner for the grant.
Abandonment retains the preparation without implicit resource field destruction;
there is no authorized custody/controller interface for this terminal state.

## Evidence

The fake-backend grant fixtures use actual root and capability admission:

- Backend error after recording its obligation invokes explicit destruction
  with caller IRQ state preserved and lifecycle/device/backend/root-table/
  physical/heap guards available. Confirmed cleanup refunds the reservation and
  permits exact root close; rejected cleanup retains the charge and root.
- An armed obligation rejects an invalid-stream creation attempt before its
  reset callback can run, preserving the original identifier and reservation.
- Earlier quota, ordinary pre-creation rejection, successful publication,
  publication rejection, one-shot cancellation and guarded abandonment fixtures
  remain enabled. Retained roots reject close with `OperationsInFlight` and
  keep their captured generation. Independently admitted roots use another slot
  and grant/close authority normally without touching retained charges.

These fake checks install no hardware domain. They add **one retained root and
one original capability reservation**, bringing grant retention to **five roots
and five reservations**. They add no IOMMU table charge or physical table frame;
the earlier IOMMU retention probes remain 22 original charges and 15 frames.
The guarded containing-owner drops remain explicit abandonment probes, not
actual panic unwinding.

The real pre-driver QEMU NVMe fixture creates a separate root after old-MMIO
authority is gone and domain-pressure checks finish. A serialized fixture flag
rejects creation **after real initial configuration maintenance succeeds**.
The backend installs its aborting descriptor and returns through the production
adapter, which must call the actual destruction path. Before rollback maintenance
and after real completion/before physical release, per-call probes verify:

- The original caller IRQ state and availability of lifecycle, device, backend,
  root-table, physical and heap guards.
- NVMe PCI config serialization is available and its bus-master-enable bit is
  clear. The probe reads config under `try_lock`; it mutates no hardware.
- After confirmed cleanup, both IOMMU charge counts match their baseline,
  the rejected namespace has zero capability charges and no device payload,
  and its exact root closes successfully.

Normal requester reset/reassignment then succeeds using the existing successor
fixture. Rejected drain, actual subsequent retirement, sparse-prefix cleanup,
data-pin retention, ordinary user isolation and scheduler fixtures remain enabled.
No I/O command is submitted. This injected rejection exercises the real rollback
path without withholding a hardware acknowledgement; actual initial-command
timeout, outstanding I/O and concurrent cross-LP contention remain G7 work.

## Validation and remaining scope

The full `scripts/run-host-tests.sh` harness passed, including 29 slot/lease
tests, four retirement-list tests and thirteen signer CLI tests. Clippy passed
on both custom targets with `--locked -- -D warnings`, using staged service
bundles. After adding armed-owner rejection before initialization and its fixture,
both target lint checks were rerun before QEMU. The host harness sources were
unchanged by that follow-up.

Intel VT-d completed **15 passed, zero failed, zero pending**. The first AMD-Vi
run passed both new creation checks but completed **14 passed, one failed**:
the user-isolation fixture's ten-second root-lease deadline expired after a
divide-by-zero fault in ASID 95, generation three. Its log is preserved at
`/private/tmp/charlotte-dma-creation-rollback-amd.log`, SHA-256
`b647d804f0e4c6fc28d0cd15ca3e736d834e2bcb781bc453de1070d1f895519e`.
User-retirement counters changed from `[185,183,0,0,1,1]` to
`[192,190,0,0,1,1]`; global shootdown observations were
`[2713,0,0,0,0,0]`. No exact staged-thread snapshot was printed. These independent
observations do not identify which root lease remained or prove pair completion.

An unchanged-artifact AMD repeat in a fresh instance completed **15 passed,
zero failed, zero pending**. It is separately recorded at
`/private/tmp/charlotte-dma-creation-rollback-amd-repeat.log`; this does not erase
the failed run or resolve its cause. Intel and both AMD runs used kernel SHA-256
`d5a8578ea6d983db3ad0d7d9c81e5987ca4b35f6b80c3fd42c32d62ccaa6e4aa`.
Arm SMMUv3 completed **19 passed, zero failed, zero pending**, including security
probe `0xffff`, policy publication generation two and **4,904 cancellation
requests**. Its kernel SHA-256 was
`715f56f868900a18d63ca23f7bff8e30c2a8ce346493cc6a32aa91444a254998`.
Assembly permission checks verified 249 x86 and one Arm native entries.
Formatting, diff whitespace and all 97 local documentation links/anchors passed;
eighteen owner-family rows and seven gates remain unchanged.

These are kernel-only changes; runners reused existing bundled services through
`CATTEN_SKIP_EMBED_BUILD=1`. Fresh-storage instances were
`dma-creation-rollback-intel-20261009`, `dma-creation-rollback-amd-20261009`,
`dma-creation-rollback-amd-repeat-20261009` and
`dma-creation-rollback-arm-20261009`, each with a 180-second timeout. x86 was
headless/no-network; Arm used `--security-test`, HTTP port 18619 and deployment
port 17919. QEMU ran sequentially after the host/final target lint checks.

Logs are `/private/tmp/charlotte-dma-creation-rollback-host.log`,
`/private/tmp/charlotte-dma-creation-rollback-clippy-{x86,arm}-final.log` and
`/private/tmp/charlotte-dma-creation-rollback-{intel,amd,arm}.log`.

Initial creation/configuration, map/unmap, controller initialization and NVMe
reset waits still hold their existing serialization. Private `Domain::new`/
context-table allocation rejection still uses explicit private cancellation
under the backend. Reset fallback still writes disabled command state and
releases its config guard; its full destructor/outer context inventory is not
qualified here. Reset activation still precedes device capability publication
on ordinary success. General metadata allocation/destruction under subsystem
guards, enclosing loader/supervisor contexts and authorized retained-owner
recovery remain separate work. Removing only a backend guard cannot preserve
the reset claim, config ownership and old-MMIO exclusion needed for those phases.

No owner-family row, custody registry or finding state is added. The earlier
Intel root-lease timeout now also recurs on AMD; passing fresh runs do not
identify its cause or authorize retry/deadline/count changes. Source review
identified a candidate ownership gap: `DomainAbortSweep::run` can mark its
executing thread aborted before releasing its own root operation. Scheduler
preemption after that request could retire the executor before the sweep
finishes. The missing staged snapshot and aggregate user-stack releases are
consistent with this possibility, not proof that it caused this run's failure.
This root-operation handoff needs its own correction and deterministic evidence;
no scheduler/abort logic is changed by this creation rollback commit.
