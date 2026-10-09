# DMA grant publication rollback and containing-owner retention

This continues C18/G1 of the [cleanup/recovery strategy](../../reference/cleanup-recovery.md)
after the [command-maintenance correction](2026-10-09-security-iommu-command-maintenance.md).
The audit ledger remains **20 corrected, six partial and four open**. This
qualifies the grant adapter's publication rollback and abandonment, not the
backend's internal creation/reset rollback or the complete SEC-18 call chain.

## Correction

`PreparedDmaDomain` previously invoked backend destruction and logged rejection
from its destructor. On capability-publication failure, that destructor could
still run beneath the grant's lifecycle guard. Its capability reservation was a
separate local, so its implicit registry/namespace metadata cleanup did not
follow the hardware rollback result. Backend quarantine alone did not retain
the enclosing publication transaction's root or staged authority charge.

The preparation now holds one `ManuallyDrop<DmaGrantResources>` containing the
exact user-root `AddressSpaceOperation`, capability `Reservation`, unpublished
registered-domain identifier and backend rollback function. The identifier is
the kernel adapter's exclusive rollback obligation; the actual domain remains
owned by its admitted backend slot. It is never reconstructed by address or
treated as a recovery ticket. Kernel grants borrow the permanent kernel root;
missing user roots reject rather than silently bypassing lease admission.

Root admission precedes lifecycle/device/backend serialization. The operation
prevents slot reuse while lifecycle captures the matching capability namespace.
Capability admission precedes hardware creation. Publication uses borrowed
`publish_batch` under device serialization, so rejection cannot destroy the
reservation before hardware rollback. Payload metadata insertion remains the
existing allocation path and is not newly qualified by this change.

On ordinary quota/backend rejection before successful creation, explicit unused
cancellation refunds any reservation and completes the root after local guards
leave. Once creation succeeds, ordinary publication rejection explicitly invokes
backend destruction after the grant's local lifecycle/device guards leave.
Only confirmed destruction permits reservation refund and root completion.
Success transfers hardware ownership to the published device payload, then
disposes inactive reservation metadata and finishes its root outside those guards.

Rollback marks its one-shot fence before invoking the backend. Rejection returns
the same containing preparation internally; the grant adapter terminally retains
it. A second cancellation rejects without another backend call. Abandonment also
retains the preparation before or after successful creation, including implicit
reservation/namespace field destruction. It acquires no guard, frees no backing
or metadata, invokes no callback/hardware operation and emits no log. Failed
hardware cleanup cannot refund its capability charge or authorize root reuse.
This is terminal retention with no operator custody/retry interface. An error
code or retained numeric domain identifier is not a retry owner. An impossible
root-lease completion rejection also remains terminal through the original slot
lease policy; no count is force-cleared.

## Evidence

Fake-backend boot fixtures exercise the actual grant adapter and real root and
capability admission without installing a hardware domain:

- Quota and retired-namespace rejection occur before backend creation. Ordinary
  creation rejection refunds admission; successful publication still composes
  with existing device namespace retirement fixtures.
- Whitebox namespace retirement after successful fake creation rejects
  publication. Confirmed rollback refunds the staged capability and permits
  exact root close. Failed rollback retains one original capability charge and
  rejects close with `OperationsInFlight`.
- Destruction verifies the caller's original IRQ state and availability of
  lifecycle, device, backend, root-table, physical and heap guards. The fixture
  does not enable interrupts or manufacture hardware completion.
- Abandonment before and after fake creation holds lifecycle, both compiled
  backend guards, device registry, both CPU table guards, capability registry,
  physical and heap allocators. Physical availability and destruction counters
  remain unchanged. Subsequent checks confirm the original charge/root remain.
- A rejected explicit rollback is attempted again and rejected before its
  callback, then abandoned under the same guards. Ordinary unused cancellation
  restores its reservation and root-close baseline.
- Every retained root remains the exact captured generation and cannot return
  its software slot. An independently admitted root receives another slot,
  grants/closes its MMIO capability and closes normally without disturbing the
  original retained charge.

These fixtures deliberately retain **four roots and four original capability
reservations**: failed grant rollback, two unstarted abandonment boundaries and
one failed one-shot cancellation. They do not add an IOMMU table charge or frame;
the earlier IOMMU retention fixtures remain 22 charges/15 frames. Fake backend
rejection is not evidence of an actual hardware timeout or physical release
failure. Guarded explicit drops exercise fallback, not actual panic unwinding.

Real Intel VT-d, AMD-Vi and Arm SMMUv3 NVMe fixtures continue to exercise normal
grant publication, domain retirement, rejected maintenance, old-MMIO exclusion
and supported reset/reassignment. Previously installed command-engine timeout
and physical-release fixtures remain enabled. Cross-LP contention, outstanding
I/O and physical-platform reset qualification remain separate requirements.

## Validation and remaining boundaries

The complete `scripts/run-host-tests.sh` harness passed, including 29 slot/lease
tests, four retirement-list tests and thirteen signer CLI tests. Clippy passed
on both custom targets with `--locked -- -D warnings`, using staged service
bundles. Initial lint runs rejected the fixture's deliberate explicit drop of a
non-`Drop` containing owner; a documented local fixture allowance corrected that
lint, and both targets were rerun before QEMU. The failed lint logs are retained.

Intel VT-d and AMD-Vi each completed **15 passed, zero failed, zero pending** in
fresh QEMU instances. Arm SMMUv3 completed **19 passed, zero failed, zero pending**,
including security probe `0xffff`, policy publication generation two and **4,880
cancellation requests**. Both x86 runs used kernel SHA-256
`b9f4f1d0c477455c88a9c814cea8f3011535305c63d793753b4e923decec5596`;
Arm used `7beab1f78de9ee1d103b49aa0dee3adec8d1e7c84e22b74f226921ede6876b0d`.
Assembly permission checks verified 249 x86 and one Arm native entries.
`cargo fmt --all -- --check`, diff whitespace and local documentation
links/anchors passed; eighteen owner-family rows and seven gates are unchanged.
These are kernel-only changes; runners used the existing bundled services with
`CATTEN_SKIP_EMBED_BUILD=1`. Instances were `dma-grant-rollback-{intel,amd,arm}-20261009`,
with fresh storage and a 180-second timeout; x86 was headless/no-network and Arm
used `--security-test`, HTTP port 18609 and deployment port 17909. QEMU ran
sequentially after the host/lint checks.

Logs: `/private/tmp/charlotte-dma-grant-rollback-host.log`,
`/private/tmp/charlotte-dma-grant-rollback-clippy-{x86,arm}-final.log`, and
`/private/tmp/charlotte-dma-grant-rollback-{intel,amd,arm}.log`.
Initial lint failures: `/private/tmp/charlotte-dma-grant-rollback-clippy-{x86,arm}.log`.

Backend creation still performs its private allocation/publication rejection
and hardware rollback under backend serialization. A backend creation error
does not return the failed domain to this adapter: its existing backend
quarantine contract remains a distinct, incomplete containing-owner boundary.
NVMe reset still owns PCI config serialization and disabled bus mastering
through new-domain creation; its failure/destructor context remains separate.
Simply releasing the backend guard would not preserve those outer owners.
Creation/map/unmap/initialization waits, private rollback, general payload
metadata allocation, unrelated caller masks/guards and generalized recovery
custody remain G1/G3/G4/G6/G7 work. No family, registry or finding state is added.

The previously reproduced Intel user-stack lease timeout remains unresolved.
Passing fresh runs do not identify its cause, justify retry/deadline changes or
close C16/G1/G2/G7.
