# Q3: explicit bootstrap storage and immutable IORT discovery

Baseline `c473bc3d`. This batch addresses concrete runtime-first-use producers
identified in the Q3 review. Current rules live in
[boot allocation boundaries](../../reference/boot-allocation-boundaries.md#bootstrap-timer-phases-and-firmware-discovery).
It does not accept the remaining Q3 inventory or later aggregate/context gates.

The BSP prepares all timer/queue/IRQ backing before AP startup. Each owning LP
then initializes its hardware with CPU interrupts masked, after local controller
setup and before the shared interrupt barrier. A shared timer slot retains
Prepared/Initializing/Ready state; missing or uncertain setup cannot be used or
replayed. Ready publication follows guard release. Runtime access clones the
existing Arc and cannot allocate or run hardware initialization.

ARM metadata construction no longer resets the BSP timer for every remote slot.
x86 removes the controller's unused second timer/calibration copy and calibrates
the authoritative prepared slot once per LP. Ten complete positive samples and
checked accumulation replace a growing sample vector; incomplete or invalid
calibration cannot publish a timer. The existing bounded boot poll remains.

The x86 dynamic IRQ symbol now has the actual dispatcher receiver type passed
by assembly, with its storage behind an explicitly prepared Once. It no longer
relies on a LazyLock sharing the receiver's layout. Runtime access rejects absent
storage. No interrupt route, reset authority or teardown proof changes.

ACPI table lookup borrows the immutable catalog instead of cloning address
vectors during DMA requests. IORT validates declared node capacity against the
bounded table before reserving storage, validates mapping geometry and caches
its optional SMMUv3 configuration during BSP preparation. ARM runtime stream-ID
translation no longer reparses firmware or creates a node vector. Requester-ID
translation and unsupported-discovery policy are unchanged.

## Qualification

The adjacent [evidence](evidence/2026-11-05-bootstrap-storage.json) records final
source identities, commands, artifact hashes and relevant diagnostics.
Full host tests, formatting and strict minimal/test checks pass on ARM and x86.
All native campaigns use four CPUs, fresh storage and frozen final sources:

| Headless profile | Result | Budget |
| --- | --- | --- |
| ARM / SMMUv3 security | 19 passed, zero failed/pending | 240 seconds; unchanged 45-second grant probes |
| Intel / VT-d security | 17 passed, zero failed/pending | Same |
| AMD / AMD-Vi security | 17 passed, zero failed/pending | Same |
| ARM / SMMUv3 minimal | Mandatory validation and operational boot passed; no regression suite | 90 seconds |
| Intel / VT-d minimal | Same | 90 seconds |

Each capture confirms exactly one local initialization marker for LPs 0–3 and
the complete shared-storage barrier before the first EL0 log. Minimal runs have
`BOOT COMPLETE` and no `SELFTEST COMPLETE`; they do not claim regression success.

Six host controls compile the real slot phases, calibration and IORT parser/cache.
They cover all 256 prepared slots, allocation-free local initialization and
reference access, missing/repeated/uncertain setup, all ten partial-calibration
positions, zero/overflow rejection and malformed IORT counts/offsets/mappings.
The cache control removes its firmware fixture after preparation and performs
1000 translations without allocation. An allocating runtime-get control fails.
These serial controls use nonallocating borrow and standard Once adapters; they
do not qualify the kernel mutex's IRQ behavior or physical hardware.

Two host preflights are preserved separately: a packed-field reference in the
header adapter failed compilation, and Darwin's standard Mutex allocated lock
backing on first use in the tracing control. Copying the packed length before
borrowing and using a nonallocating serial borrow adapter corrected the harness;
neither failure came from the production timer or IORT implementation.

## Remaining scope

No runtime resource ceiling or test deadline changes. Static platform backing
remains bounded by the validated LP count, with aggregate allocator/control
overhead assigned to Q4. Broader timer/IRQ caller contexts remain Q5. This batch
adds no recovery or physical-release authority, and no new cleanup owner family.
Firmware still supplies actual table backing; caching metadata is not mapping
provenance, authentication or an IOMMU completion proof.

Launch encoders, the kernel Kafka-profile validator's allocating decoder,
other producer/dependency/profile assignments and the final Q3 reconciliation
remain. Existing display, Intel retirement-pin and ARM exit-watch
incidents retain their open status; passing headless campaigns cannot explain
them. Q4–Q8 retain their existing acceptance requirements.
