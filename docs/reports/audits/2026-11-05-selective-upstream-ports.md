# Selective upstream ports: address map and bounded PCI boot

This checkpoint starts at `91893e1f4380fb69c608201cc699d70134b16c17` and
implements the first batch selected by the
[upstream review](../investigations/2026-11-05-upstream-review.md). It adapts
upstream ideas to this fork's current ownership and reset contracts; it does
not merge the upstream kernel. Current rules and limits live in
[boot allocation boundaries](../../reference/boot-allocation-boundaries.md#address-regions-and-pci-firmware-topology).

## Source and resulting behavior

- Address-map invariants derive from upstream
  [`aec5f68d`](https://github.com/charlotte-os/charlotte-os/commit/aec5f68df26e5eeb932f6431a477260d354468fd).
  Checked offset containment and raw exclusive ends correct the lower-region
  wrap and top-region overflow. Layout validation checks contiguous software
  regions. This fork retains its active arena bases and raw ARM TTBR1/HHDM
  addresses; 39/57-bit model controls do not add native paging-mode support.
- Bus range/revisit rules derive from upstream
  [`72cc1417`](https://github.com/charlotte-os/charlotte-os/commit/72cc1417b8ddf09013caab1d80dfc389ccfbdf18).
  A fixed 256-entry worklist replaces recursive topology construction. Flat
  bus ownership and borrowed endpoint iterators remove allocating lookup
  stacks and expose all eight multifunction positions. Endpoint Boxes remain
  stable for existing reset claims; lookup preserves depth-first sibling order.
- MCFG validation is adapted from upstream
  [`74def27c`](https://github.com/charlotte-os/charlotte-os/blob/74def27ce686a07aaec621f15ef786b546c54f20/crates/catten/src/environment/acpi/sdt/mcfg.rs).
  All table/entry geometry, checksums, reserved fields, ranges and aliases are
  checked before mapping. Limits are 32 windows and independent global
  16,384 device/function quotas. Rejection stops mandatory boot. Private
  metadata drops normally; permanent kernel ECAM mappings grant no rollback,
  hardware teardown or reset replay authority.

The bus-zero ECAM convention also applies to nonzero starting buses, as
described by the [Linux PCI firmware documentation](https://www.kernel.org/doc/html/v6.6/PCI/acpi-info.html).
The existing full 256 MiB mapping is retained; 1 MiB-aligned bases accepted by
validation use 4 KiB leaves when necessary. Actual firmware backing and resource
reservation reconciliation remain outside this geometry check.

## Qualification

Final qualification and artifact identities are recorded in the adjacent
[evidence](evidence/2026-11-05-selective-upstream-ports.json). Full host tests,
formatting and strict minimal/test checks on both architectures pass. The host
controls compile the actual map, parser and worklist code: 11 tests pass,
including allocation-free preparation, exact capacity and all 256 bus identities.
Restoring old containment arithmetic fails the boundary control.

All final native campaigns use four CPUs, fresh storage, the frozen source
hashes in the evidence and unchanged 240-second outer/45-second probe budgets:

| Headless profile | Deferred tests | New real-RAM builder and existing reset controls |
| --- | --- | --- |
| ARM / SMMUv3 | 19 passed, zero failed/pending | Passed |
| Intel / VT-d | 17 passed, zero failed/pending | Passed |
| AMD / AMD-Vi | 17 passed, zero failed/pending | Passed |

The additional synchronous builder fixture is not counted as a deferred test.
It covers cycles, advertised-range fencing, multifunction position seven,
stable endpoint/reset-cell identity and rejection between two functions.
No hardware claim is created by that private-RAM fixture. The ordinary campaigns
also exercise real PCI discovery, storage/network services, signed launch,
grant cleanup and live-upgrade/handoff controls with the existing owners.

The first strict-check invocation omitted the required service-bundle environment
variables and failed compilation; configured repeats pass without source changes.
An earlier parser preflight required the `as_chunks` lint correction. The first
host harness expected an assertion string but the unsafe control instead failed
on the old arithmetic overflow; accepting that precise expected failure fixed
the harness. These were preflight/tooling outcomes, not passing native runs.

## Remaining boundaries

This batch assigns the bounded PCI/MCFG boot producer within Q3. It does not
accept Q3, close SEC-07/18 or alter aggregate gates. Bootstrap timer/IRQ first
use, other factories, aggregate/principal/allocator overhead and caller-context
qualification remain open. Runtime hotplug, a real 1 MiB-only-aligned ECAM
platform and allocator-OOM injection are not qualified here. The existing
QEMU-NVMe reset scope is unchanged.

On both native architectures, the conservative requested metadata formula is
5,768,704 bytes at the configured ceilings (136-byte device, 200-byte endpoint,
32-byte bus). This is a type/count bound, not measured ordinary-boot usage.
Allocator and page-table overhead remains Q4; the compile-time 16 MiB ceiling
also catches later type growth. Larger limits trade capacity for scan latency
and backing overhead and need that same review.

The earlier ARM display second-grant-probe deadline remains unresolved in its
[separate capture](2026-11-04-framebuffer-bounds.md). These headless campaigns
cannot close it. No test deadline or runtime resource ceiling was increased.
Flanterm/uACPI upgrades, optional xAPIC support and fork detachment are outside
this batch.
