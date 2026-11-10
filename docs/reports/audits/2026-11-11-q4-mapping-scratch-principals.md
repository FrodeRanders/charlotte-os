# Q4 mapping and scratch principal funding

Baseline `ec105948`. CPU memory-object mappings and scratch namespace and
extent nodes now reserve their requested layouts from the original principal's
8 MiB metadata allowance. The existing mapping and scratch owners retain the
same nodes through publication, confirmed release and uncertain cleanup.
[Mapping admission](../../reference/mapping-node-admission.md)
and [scratch admission](../../reference/scratch-admission.md).

## Capture and disposal

Preparation uses the shared exact-generation resource sponsorship capture.
That capture combines the original principal with the explicit platform grant.
Scratch takes it once for both candidates. Mapping snapshot buffers still use
their own backing classification and buffer allowance. Both node factories use
the shared funded charge, so allocator or family rejection returns unused
principal bytes. Final publication relinks prepared nodes under the existing
checks.

Mapping and scratch metadata retain their original charges after detachment.
Confirmed release disposes each actual node before refund. The existing
mapping pin and final-root custody retain charges when maintenance is uncertain.
MMIO cleanup can confirm and refund a scratch extent while a device node and
its authority remain owned. The scratch namespace survives an ordinary extent
release until its own retirement.

## Capacity

| Requested node layout | Earlier bytes | Funded bytes |
| --- | ---: | ---: |
| Scratch extent | 48 | 64 |
| Scratch namespace | 72 | 88 |
| CPU mapping record | 120 | 136 |

The additional 16 bytes per slot raise each family's ordinary and total byte
ceilings by 96 and 128 KiB at the existing 6144 and 8192 slot ceilings.
Scratch uses 480/640 KiB ordinary/total and mapping uses 864/1152 KiB. For
any formerly admitted mix, its extra bytes are 16 times its node count,
bounded by the increase. Scratch mixtures with many namespaces still reach
the byte ceiling before the count ceiling. A full mapping count consumes
816/1088 KiB ordinary/total, retaining 48/64 KiB of layout headroom.

The two total ceilings add 256 KiB of potential requested heap use without
preallocation. Keeping the former byte ceilings would make the new layout
reduce usable count or mixed-layout capacity. The updated capacity census
records the revised policy envelope. Simultaneous heap use across families
and allocator excess remain open.

## Qualification

The controls reconcile 128 funded scratch and 64 funded mapping allocation
failures. Sibling roots share principal headroom. Scratch preparation rejects
at the first node or after allocating only the extent candidate. A separate
map-any attempt admits both scratch candidates, then rejects at mapping
admission. The unused extent refunds with the original empty namespace
still owned. An independent principal maps successfully. Successful mapping
and unmapping reconcile the exact extent and mapping charges.

An unused mapping preparation retains 136 original bytes across root close
and ASID reuse, then refunds the old principal independently of the
successor. The existing uncertain rollback fixture holds its detached
136-byte record and backing pin through rejected maintenance under the
principal history guard. The final-root fixture retains 152 scratch bytes
through rejected invalidation and guarded attempt abandonment. Confirmed
disposal and successor isolation are checked in the same fixture. Existing
MMIO controls distinguish scratch refunds from device and authority refunds.

Both initial security runs reached the new mapping controls and stopped at a
fixture assertion after staged close. The fixture expected 1160 principal
bytes and observed 1016. The earlier confirmed IPC drain had freed a 144-byte
namespace before mapping cleanup encountered the retained pin. The corrected
fixture checks the full charge before close and separately accounts for the
confirmed IPC release afterward. The mapping record and pin stay retained.

The full host suite, four strict test/minimal profiles, formatting and capacity
census pass. Fresh four-CPU security campaigns pass ARM SMMUv3 19/19, Intel VT-d
17/17 and AMD-Vi 17/17. ARM and Intel minimal boots pass. Both x86 security
campaigns use the same kernel, embedded security fixtures, signed bundle and
initial disk. Each signed bundle matches its qualified completion-batch
counterpart. Security and minimal deadlines were 240 and 90 seconds.

[Versioned evidence](evidence/2026-11-11-q4-mapping-scratch-principals.json)
records the sources, limits, native artifacts, failed captures and checks.
The wider principal coverage, joint heap sizing, cleanup caller review,
physical completion and sustained progress gates remain open.
