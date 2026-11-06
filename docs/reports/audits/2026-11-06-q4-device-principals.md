# Q4 device-node principal funding

Baseline `1d09b64a`. Device namespace and MMIO/IRQ/DMA payload nodes now fund
actual requested layouts against the original verified principal. This advances
one existing catalog family. Q4 remains open: allocator excess, other families,
joint heap fit and sustained cleanup/service progress still require qualification.

[Version-controlled evidence](evidence/2026-11-06-q4-device-principals.json)
contains frozen source/input identities, selected policy, native artifacts and
inline milestones. The initial Intel boot failure is preserved separately from
its identical-kernel repeat. No finding disposition, production trust or
physical-platform acceptance changes.

## Capture and ownership

`GrantAdmission` captures exact root generation, immutable principal and trusted
backing class together, then acquires the generation-qualified operation before
allocating either registry node. Both candidates use the existing
`NodeCharge<DeviceNodes, MetadataSponsor>` and shared funded factory. Funding
precedes family admission and allocation; either failure returns the unstarted
prefix. The old class-only capture adapter has no consumers and is removed.

Original sponsorship survives platform promotion, root close, transfer and ASID
reuse. Device resource admission, authority records, backend creation and
hardware completion remain separate. Backend/IOMMU storage still has its existing
class admission; this batch does not add its principal coverage.

Actual node deallocation precedes family and principal refunds. Existing complete
grant, close and namespace owners retain their nodes through uncertainty. Their
inert abandonment also remains inert beneath the principal history guard.
No new registry, cleanup controller, retry authority or deliberate leak is added.
Native fixtures use a trusted prebound-principal constructor; signed launch
verification is exercised separately by the combined security campaigns.

## Capacity and limit choice

The existing shared **8 MiB principal metadata allowance** now funds these nodes
alongside migrated root, namespace, authority and mailbox records. No history
field, pool, SDK, protocol, logical limit or global byte/count limit changes.

| Actual native layout | Requested bytes |
| --- | ---: |
| Funded inline charge | 32 |
| Device namespace node | 64 |
| MMIO/IRQ/DMA enum payload node | 88 |
| Separate authority node | 96 |
| Empty two-node candidate | 152 |
| Complete initial grant metadata | 248 |

The former namespace/payload layouts were 48/72 bytes. At the unchanged
104448/139264 ordinary/total count ceilings, even all-largest populations require
9191424/12255232 requested bytes, below the existing 12/16 MiB device byte ceilings.
Raising those ceilings would allow more heap use without increasing the current
count-bound capacity; reducing counts would unnecessarily restrict existing
mixes. Neither adjustment is needed for this layout growth. Allocator excess and
nested backing are not included in these figures.

A bare funded root costs 928 metadata bytes. A fixture reaches the 8 MiB allowance
with 95314 actual 88-byte empty device payload candidates, using 8388560 bytes and
leaving 48, while ordinary device-family slots remain. This is serialized real
allocation pressure, not sustained client fairness. The 24-pool catalog still has
19 independent byte ceilings totaling 205.5/154.125 MiB total/ordinary against the
default 72 MiB mapped heap. These are overlapping ceilings, not a joint-fit proof.

## Controls and qualification

Sibling roots share principal funding; a distinct healthy principal can still
obtain and close a real grant. 128 funded allocator failures restore principal
and family baselines. 96 heap-held calls through the real MMIO, IRQ and DMA grant
adapters reject without allocation or hardware callbacks. Six actual headroom
prefixes (0, 63, 64, 151, 152 and 247 bytes) exercise namespace, payload and
separate authority admission through all three adapters without changing live
capabilities, interrupt routes or original charges.

Existing empty-candidate retention crosses confirmed root close and ASID reuse
by a different funded principal. Releasing the old candidates refunds the old
principal only. Mapped scratch/direct MMIO and IRQ close retain funding after
physical cleanup until metadata release; complete root close returns the rest.
Existing rejected/abandoned explicit close and grant owners retain original
principal bytes under publication, heap, physical, device-pool and history guards.

The existing complete synthetic unit/grant/pin abandonment retains 1368 metadata
bytes along with its two real table frames, pinned memory and unit obligation.
Existing partial namespace failures reconcile remaining device/authority bytes:
240/192 after detach, invalidation or scratch rejection; 152/96 after DMA rejection;
328/288 on abandonment. Confirmed siblings refund, while exact roots and remaining
work stay funded. These synthetic controls do not establish actual reset support
or physical-platform isolation; native backend campaigns retain their own scope.

The full host suite, formatting, four strict test/minimal profiles and capacity
census pass. Four-CPU fresh-storage security campaigns pass ARM SMMUv3 **19/19**,
Intel VT-d **17/17** on the controlled repeat and AMD-Vi **17/17**. ARM and Intel
minimal boots pass. Security deadlines remain 240 seconds; minimal deadlines
remain 90. Signed service bundle hashes match the preceding qualified container
batch. Native ARM measurements and x86 DWARF agree on 64/88-byte nodes and the
32-byte charge; the evidence binds DWARF to the AMD campaign kernel hash.

A strict preflight initially rejected the now-unused `capture_address_space_storage`
adapter with `-D dead-code`. Removing it after migrating its last consumer fixes
the error; final strict checks pass. The failed diagnostic and log identity are
preserved separately in the evidence.

## Preserved Intel boot failure

The initial four-CPU Intel run stops during local interrupt-controller startup,
around 0.053 seconds, before device controls. Serial writes from AP startup
interleave the panic. Read across those writes, the diagnostic identifies:

```text
crates/catten/src/cpu/isa/x86_64/timers/apic_timer.rs:91:24
incomplete or invalid APIC timer calibration: Zero
text=0xffffffff80000000..0xffffffff805edaca
sp=0xffff80001c009b80 fp=0xffff80001c009c20
#00 0xffffffff800f94ae
#01 0xffffffff804da1ab
#02 0xffffffff804dfebb
#03 0xffffffff8015a12b
#04 0xffffffff801cfba6
```

This is a reconstruction for readability; the original interleaved lines are
inline in the evidence. The runner records `reason=deadline` at 240.056599 seconds,
not a recognized panic, and captures QMP, LLDB and a snapshot. LLDB places
`determine_timer_resolution` beneath `bsp_main`; the other three processors wait
at the startup barrier in `ap_main`. No device funding test or deferred-suite
completion was observed in that run.

The failed kernel SHA-256 is
`14207a4806874ffb2ecf567d19f48f57fe13fe7174d7401b7a696ace1bf48080`.
The repeat uses that exact kernel (including embedded signed security fixtures),
identical signed services and identical initial disk
`6dc9db7495e27f2ab53a04b0f62747f4fd7a63f24f62fa0024c6a7939b0df278`.
It skips kernel rebuilding, preserves the four CPUs, Intel backend, fresh-storage
policy and 240-second deadline, and passes all 17 deferred tests. Newly generated
host fixture files are unused by the unchanged kernel; fixture bytes are embedded
at compilation. Separate runner/boot-image identities remain recorded.

The timer/calibration/TSC sources are unchanged by this batch. The fixed ten-sample
calibrator rejects any zero sample before publishing a timer resolution. The bootstrap
processor is identified, but no sample index, TSC start/end/period or APIC counter
state is captured. Arming before taking the start timestamp is a possible timing window to
investigate, not an established cause. The capture cannot attribute the failure
to host scheduling, the funding change or another incident. A passing repeat does
not explain it or qualify sustained boot reliability. Preserve zero rejection
and capture those values on recurrence.

This incident remains in the [current incident register](../../reference/security-remediation.md#unresolved-incidents),
alongside the ARM display second-probe, Intel retirement-pin and ARM exit-watch
incidents. Q5–Q8 and wider Q4 remain open; observer/list capture bridges require
a discrete cross-owner review before adding their principal funding.
