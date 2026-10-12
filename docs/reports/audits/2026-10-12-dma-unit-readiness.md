# DMA unit admission readiness — 2026-10-12

Scope: SEC-18 / C01, C14, C15, C18 / G1, G3, G4, G7. Baseline: `89986631`.

Scheduled driver preparation now registers an owned interest and parks when the
existing IOMMU unit or command engine is busy. It retains its original complete
DMA grant, exact root operation and authority reservation through the wait.
Hardware creation runs once after successful admission. This supplies an
availability notification contract; no hardware polling or failed physical
cleanup is converted into retryable work.

## Ownership and producer contract

The existing VT-d, AMD-Vi and SMMUv3 fixed slots now contain `UnitSlot`, retaining
their original Vacant/Claimed/Installed state and an optional charged readiness
source. This is not another domain registry or recovery family. First-wait
classification comes from the complete grant's captured root and matching
platform identity, outside backend serialization. Source/list-disposal storage
is prepared fallibly outside guards, then relinked after current slot validation.
Losing preparation is explicitly disposed after guard exit.

Registration touches only the independent observer list under scheduler
serialization. A closed-source recheck covers completion before registration.
The transient interest is retained in `DmaCreation` behind ManuallyDrop, beside
the containing grant's original root/reservation. Successful admission releases
that interest outside backend serialization, after the extracted complete unit
has entered its retaining owner. Ordinary unarmed rejection cancels the original
preparation explicitly. Admission failure returns WaitRegistrationFailed through
the existing per-return creation diagnostic/cancellation contract; it does not
publish a capability or invoke hardware.

Creation restoration and mapping/destruction restoration signal only when the
actual command engine is present and every admitted domain cell contains its
owner. In particular, destruction returns its engine before physical table/data
finalization, while the original domain cell remains empty. That interval is
nonready. Confirmed finalization removes the cell; rejection restores the exact
frozen domain. Either can allow fresh creation admission without granting any
permission to retry that domain's cleanup. Notifications and source disposal
use the existing producer-LP deferred metadata transport after local guards
leave; no inline scheduler callback, allocation or backing destruction occurs
in a backend hold.

Wake grants availability, not ownership. The waiter rechecks the current slot
and may register for a new busy episode if a competitor claimed it first.
Published units have no production replacement path; no numeric root or domain
lookup substitutes for the original grant owner. Abandoned complete-unit or
detached-domain claims remain nonready, with their source and allocations retained.
Neither Drop nor an error code fabricates availability or physical completion.

Both supervisor DMA preparation paths use the explicit scheduled grant API.
The existing rejecting API remains the serialized/reentrant fixture adapter:
a checkpoint inside the unit producer cannot park waiting for itself. Masked
or nonscheduled waiting rejects explicitly without enabling interrupts.
An already armed creation rejects before source admission or hardware work.
The pre-thread boot grant batch remains in place.

## Executed ownership evidence

Serialized private-slot fixtures use the production complete grant admission
and exact root/reservation, checking:

- Root close rejects with OperationsInFlight and the original reserved device
  authority remains charged during the wait.
- Complete-unit Claimed and absent-engine states admit interest. Returning an
  engine with a detached cell does not close the source or wake its registration.
- Completing the slot closes the source, while callbacks remain deferred until
  after its guard. Callback probes check device/lifecycle/scheduler availability.
- Late registration on a closed source is immediately ready. A competitor
  winning the next claim forces another admitted episode before extraction.
- Injected source rejection, 64-entry source saturation and a producer winning
  the preparation race dispose ordinary unarmed admission and close the root.
  Losing preparation and completed sources recover their original list usage.
- The production wait rejects an enclosing IRQ mask without enabling IRQs or
  allocating a new source, then explicit cancellation releases admission.

The existing guarded complete-unit/grant abandonment fixture additionally
retains its source and transient interest. No synthetic readiness is emitted;
one original ordinary list allocation remains charged alongside its existing
root/reservation, unit/domain tables, data pins and metadata. It adds no new
abandoned root or physical recovery owner.

A scheduled kernel worker uses a private fixture slot and the production
grant/publication path. The parent observes exactly one registration and actual
Blocked thread state, zero creation work and busy exact-root close. Availability
wakes the worker; creation work executes exactly once, publishes one fixture
authority and closes that same authority through the typed DMA close adapter.
Original authority usage returns to zero and the exact root closes. The bounded
one-millisecond sampling belongs only to the fixture parent; production grant
waiting has neither a polling timer nor a watchdog.

All final guests emit these assertion-backed diagnostics:

```text
[DMA creation readiness] exact root/reservation retained; detached cell gates engine wake; deferred notification, late registration, source rejection, saturation, preparation race, competing epoch and masked rejection passed
[DMA creation readiness] scheduled grant parked with exact root/reservation, woke on unit availability, created once and closed its original fixture authority
```

Existing real complete-unit creation checkpoints continue to require a nested
rejecting grant to preserve Backend(OperationInFlight), Confirmed cancellation
and unchanged original authority usage. Existing real map/unmap/destruction,
reset, private-prefix and hardware-failure tests remain enabled across all
three backends. The scheduled readiness fixture owns no live hardware unit;
these samples do not prove arbitrary concurrent live-hardware progress or
kernel unwinding behavior.

## Validation and artifacts

Both signed service bundles were rebuilt before guest execution. Host suite:
**384 passed, 0 failed, 3 ignored**. Strict `-D warnings` Clippy passed for the
bundled services and both final-source kernel profiles. Formatting, diff and
changed Markdown-link checks passed.

Fresh-storage guests use a 180-second runner deadline. Deferred test totals
below include the CQ-wait test that executes the scheduled grant fixture;
serialized synchronous assertions are additional.

| Profile | Passed / failed / pending | Final bitmap | Completion time |
| --- | --- | --- | --- |
| ARM SMMUv3, security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | 56.800360 s |
| x86 VT-d, HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 18.751889 s |
| x86 AMD-Vi, HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 23.280492 s |
| ARM shutdown ingress | 19 / 0 / 0 | `0x103ffff` | 10.383130 s |
| x86 VT-d identical-artifact cold repeat | 18 / 0 / 0 | `0x123bbff` | 17.963388 s |

All five runners exited 0 with zero failed/pending bitmaps. HTTP/deployment
ingress controls passed; shutdown requested PSCI power-off at 14.317806 s.
The existing assertion-backed pre-thread driver grant batch marker remains in
every guest. No guest failure was captured in this matrix. The Intel repeat
addresses the historical intermittent concern as a limited additional sample;
it is not a reliability or pressure-soak claim.

Commands use `CATTEN_SKIP_EMBED_BUILD=1` after both bundle builds:

```sh
scripts/run-aarch64.sh --security-test --http-test --dhcp-test --fresh-storage --timeout 180 --instance dma-readiness-arm
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance dma-readiness-intel
scripts/run-x86_64.sh --http-test --dhcp-test --iommu amd --fresh-storage --timeout 180 --instance dma-readiness-amd
scripts/run-aarch64.sh --shutdown-ingress-test --fresh-storage --timeout 180 --instance dma-readiness-shutdown
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance dma-readiness-intel-repeat
```

HTTP/deployment ports were 19331/18631, 19332/18632, 19333/18633 and
19334/18634 and 19335/18635. The repeat additionally used
`CATTEN_SKIP_KERNEL_BUILD=1` only after the current ELF matched the immutable
initial Intel boot hash. AMD's rebuilt ELF and the repeat's loaded artifact
match that same hash. Immutable per-boot kernel hashes are recorded before
later profile builds overwrite the development ELF:

| Artifact | SHA-256 |
| --- | --- |
| ARM combined kernel | `6bf99db201cb3886d4f4ae455182b72593b26bdebe285a4cea57118785e8ef32` |
| Intel / AMD / Intel-repeat kernel | `d6bff6e44b1b577e90968567f0301d117202446e32e938a9d75d9b3c01d44e1e` |
| ARM shutdown kernel | `42aa4e3f8279b201ce0a5b44848db1b63c16e0996b179b0da8d9810ecfc152ce` |
| ARM bundled service ELFs | `e8bf681a4f01d71c30cecfe9d13f1362e3b7e9f8c79a04e078d3befbcd915621` |
| x86 bundled service ELFs | `2bf773dfb18cbd457ddbe31a7c2d81816e5d8d147d9e57a6425f11f4ee5bc999` |

Bundle hashes use sorted newline-terminated `basename:ELF-sha256` lines hashed
with SHA-256. Results, diagnostics and hashes are inline; temporary logs and
ignored build artifacts are not evidence dependencies.

## Remaining limits

The [historical Intel startup failure](2026-10-12-ipc-close-readiness.md) remains
causally unresolved. The [boot preparation correction](2026-10-12-dma-startup-preparation.md)
and this availability contract identify and address specific contention
boundaries; passing runs do not identify that old failure's cause.

Public DMA map/unmap/close and grant cancellation retain their existing busy
rejection policies. In particular, contention during cancellation can retain
the complete original grant; fresh availability is no promise of successful
rollback. Indefinite claimed-unit retention and sustained admission pressure
have no deadline or recovery controller here. Hardware register polling,
broader launch rollback/implicit Drop/outer IRQ qualification and production
custody remain separate. SEC-18 and SEC-07 remain partial; production enrollment
restrictions and all uncertain/partially released physical retry prohibitions
remain intact.
