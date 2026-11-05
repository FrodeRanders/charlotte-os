# Q3 producer review and final corrections

Baseline `240d1456`, after the timer/firmware bootstrap batch. Current producer
assignments live in [kernel allocation review](../../reference/kernel-allocation-review.md).
The adjacent [review evidence](evidence/2026-11-05-q3-producer-review.json)
contains source identities, all selected site dispositions and dependency/profile
supplements. [Qualification evidence](evidence/2026-11-05-q3-producer-qualification.json)
records executable controls, artifacts and preserved preflight failures.

## Implemented corrections

Kernel Kafka-profile validity checking used the owned decoder and allocated
three vectors after decrypting already-admitted input. One borrowed parser and
field validator now serve both callers: `validate_encoded` uses 3840 bytes of
fixed list scratch, with a compile-time 4096-byte ceiling, and owned decode
materializes its vectors only after validation. No credential bytes are copied
into another temporary owner. Wire version, authentication, rights, integrity
rules and the 64 KiB/64-route/31-additional-broker/64-authority limits stay fixed.

ARM hardware-ASID retirement previously pushed returned tags into a growing
vector. Boot preparation now reserves all 256 or 65536 u16 return slots before
user tag publication, costing 512 bytes or 128 KiB of requested permanent
backing. Runtime allocation/return cannot grow it. This keeps the full hardware
tag range and avoids a new cleanup allocator/retry policy. It changes no root
ownership, invalidation, physical-release or quarantine condition.

Compiled cluster service input and encoding enforce the shared 16-service
bound before collecting. Unknown ACPI signatures format four borrowed bytes
instead of allocating an unchecked UTF-8 String. Unused allocating PCI capability
walkers and their obsolete header/feature support were removed after a whole-tree
caller search; active claim-aware bounded driver lookups remain unchanged.

## Review scope and remaining obligations

The refreshed two-profile capture has **2766 coalesced records** and no missing
source spans. Q3 reconciles all **738 non-drop records**: 144 allocator/growth/
copy/factory, 505 owned-result and 89 empty/reference operations. The remaining
2028 explicit/implicit Drop records belong to Q5. Source identities cover 614
kernel Rust files; named library and optional-profile supplements are retained.
The review groups alias existing owners/contracts and create no pools or cleanup
owner families. Counts are neither defect counts nor readiness percentages.

The concrete uncompleted Q4 assignments include aggregate/original-principal
admission for diagnostic snapshot scratch and heap/image frame-tracking vector
capacity. Existing per-family admission still needs coherent signed-principal,
allocator/control-overhead, retention/restart/quarantine and heap-envelope review.
A known per-request bound is not recorded as aggregate coverage. Q5–Q8 retain
their caller, recovery, progress and combined acceptance requirements.

## Qualification

**Q3 is accepted within its producer-review scope.** Every selected current
producer has an owner, an explicit Q4 correction or a justified bounded/inactive
disposition. This accepts neither Q4 accounting nor a finding-wide gate.

| Final frozen-source campaign | Result |
| --- | --- |
| Full host suite and formatting | Passed |
| Strict ARM/x86 test and minimal profiles | All four passed |
| Rebuilt signed ARM/x86 service bundles | Test and minimal passed |
| Four-CPU ARM SMMUv3 security | 19/19, unchanged 240-second budget |
| Four-CPU Intel VT-d and AMD-Vi security | 17/17 each, unchanged 240-second budgets |
| Four-CPU ARM and Intel minimal operational boot | Both passed, 90-second budgets |
| Four-CPU ARM Kafka TLS/mTLS/SCRAM integration | 20/20, 300-second budget |

Host controls accept a complete 65536-byte profile at every list-count ceiling
with zero validator allocations; the owned-decoder control allocates three times.
The shared decoder and validator agree across all four authentication choices,
byte mutations with/without refreshed integrity and every truncated prefix.
Existing 18 codec tests and two aggregate-Debug rejection doctests pass.

Hardware-tag controls cover full 8/16-bit exhaustion and recycling, kernel/zero
exclusion, allocation failure before issuance and unchanged return capacity.
Restoring growing return storage fails the allocation tracer. Native security
fixtures run the actual maximum-profile validator on a scheduled kernel stack,
including tamper and authenticated over-count rejection. That is a codec/stack
control. The separate live Kafka campaign also exercises userspace owned decode,
TLS/mTLS/SCRAM, transactions and the planned route-leader kill/recovery. Its
transport failures, retries and terminal counters are retained alongside final
success in the evidence. Neither campaign qualifies complete encrypted
operational pickup admission.

Compile preflights caught an incorrectly removed telemetry helper still imported
by a child module, an obsolete capability header made unused by walker removal,
and the now-unused `likely_unlikely` feature. The helper was restored; its actual
caller uses the existing bounded telemetry buffer. Removing the unused header
and feature completed the walker cleanup. These failures are preserved separately
from final qualification; no native failure or deadline extension is hidden.

The earlier ARM display, Intel retirement-pin and ARM exit-watch incidents remain
unresolved. Headless results do not close them. Optional device-tree/RISC-V/HVF
limitations remain explicit; this review does not create production trust or
physical-platform qualification.
