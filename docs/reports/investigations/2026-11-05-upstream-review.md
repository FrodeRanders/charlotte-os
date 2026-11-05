# Upstream review before possible fork detachment

Reviewed 2026-11-05. This is a static comparison, not a merge or qualification
of upstream code on this fork. No kernel, service, dependency or remote
configuration was changed. Upstream history was fetched by URL into isolated
`upstream-review/*` references.

## Revisions and scope

| Item | Identity |
| --- | --- |
| This fork's `main` | `91893e1f` — framebuffer geometry/requested-byte admission |
| Upstream repository | <https://github.com/charlotte-os/charlotte-os> |
| Upstream `main` | `74def27ce686a07aaec621f15ef786b546c54f20` — November 3 scheduler changes |
| Shared ancestor | `ac8604e2c2a2e146b6e44b8c7046816047914eca` — July 9, 2026 |
| Divergence | 1,260 commits only on this fork; 93 only on upstream `main` |
| Patch comparison | 92 upstream patches are not patch-equivalent to ours; `28867e5d` removing `abi_custom` is equivalent |
| Other published heads | `interrupt_refactor` at `b62dd732`; `mdp-devel` at `18569ec0` |

All 93 commit subjects and changed-path groups were screened. The implementations
below were inspected against our current counterparts. This is not a line-by-line
audit of every upstream change. The upstream kernel was not built or booted here. A small
host probe compiled the actual address/map modules from both revisions, using a
48-bit configuration and a standard-library LazyLock adapter; it executed no
page-table or privileged CPU operations.

## Recommended selective work

| Priority | Upstream work | Value and adaptation required |
| --- | --- | --- |
| First | [Address-map correction, `aec5f68d`](https://github.com/charlotte-os/charlotte-os/commit/aec5f68df26e5eeb932f6431a477260d354468fd) | Corrects canonical-half extents, checks layout tiling, avoids canonicalizing exclusive lower-half endpoints and makes containment use an offset. Our shared map still contains the old oversized extents and endpoint calculation. Port validation/correctness deliberately; do not copy their entire new layout, which moves arenas and adds an ACPI aperture. |
| First | [MCFG validation, current source](https://github.com/charlotte-os/charlotte-os/blob/74def27ce686a07aaec621f15ef786b546c54f20/crates/catten/src/environment/acpi/sdt/mcfg.rs) | Checks the complete MCFG header, exact entry size, bus order, base alignment and address overflow before collecting regions. Our parser subtracts the larger MCFG header after generic SDT validation and lacks these entry checks. Directly relevant to Q3. Retain an explicit segment/storage bound, duplicate/overlap policy and our mapping ownership. |
| First | [Iterative PCI enumeration, `72cc1417`](https://github.com/charlotte-os/charlotte-os/commit/72cc1417b8ddf09013caab1d80dfc389ccfbdf18) | Iterates buses, restricts them to advertised ranges and skips already-enumerated buses. This addresses our depth-only traversal limitation. Adapt the identity/range rules with a bounded worklist/visited set; upstream's HashMap/VecDeque rewrite is not itself our admission policy. Preserve claimed device-reset access and exact grant ownership. |
| Next, separate dependency batch | [Owned Flanterm wrapper, `ac287986`](https://github.com/charlotte-os/charlotte-os/commit/ac2879861c4971a63379b35f3c7596c5b77a013e), and [`ft-wrapper` at the reviewed tip](https://github.com/charlotte-os/charlotte-os/tree/74def27ce686a07aaec621f15ef786b546c54f20/crates/ft-wrapper) | Vendors Flanterm 3.1.2, uses an opaque context and architecture-specific C/bindgen flags. The newer backend checks allocation multiplication overflow and null context on failure. We independently fixed the wrong Box owner and initial-allocation failure in the older registry source. This is a useful refresh candidate, but the API adds rotation, makes context layout private and changes free to an unsized callback. Preserve/requalify our six-allocation plan, byte ceiling, geometry rejection, prefix cleanup and inert retention. No claim that it fixes the current display deadline. |
| Conditional hardware work | [xAPIC fallback, `f23c80b7`](https://github.com/charlotte-os/charlotte-os/commit/f23c80b7) | Adds MMIO xAPIC alongside x2APIC, including mode-aware local interrupts and timer access. Our boot explicitly requires x2APIC. Valuable if supporting machines/VMs without exposed x2APIC becomes a goal; it needs qualified IPI destination routing, timer behavior and DMA/interrupt-remapping interaction. Not an urgent audit fix. |
| Larger future subsystem | [uACPI 7 integration, `44ce0746`](https://github.com/charlotte-os/charlotte-os/commit/44ce0746) | Our AML module is a stub. Upstream has an interpreter, namespace/event initialization, PCI/SystemIO services, SCI/IOAPIC routing, persistent deferred workers and host-interface self-tests. Useful foundation for real firmware support, not a small cherry-pick. Its 43 host callbacks need our allocation admission, original sponsorship, mapping retirement and caller/IRQ contracts. Power-management enums are not a completed suspend/shutdown implementation. |
| Design idea only | [LP sets and exit separation, `74def27c`](https://github.com/charlotte-os/charlotte-os/commit/74def27ce686a07aaec621f15ef786b546c54f20) | Multi-LP affinity sets could be useful later. Our generation-safe exit, watches and reaping already cover the exit/reclamation concern more deeply. Upstream uses per-thread HashSets and creates/sorts a Vec/Box during LP selection; do not import that allocation path. A future affinity-set feature should fit our bounded LP model and signed-placement rules. |

### Confirmed address-map discrepancy

Our 48-bit map declares 256 TiB starting at `0x1000` for the application region.
The existing VAddr conversion canonicalizes its computed endpoint back to the
start. The host probe produced:

```text
ours:
  application base=0x1000 bytes=281474976710656
  region_exclusive_end=0x1000 contains_64KiB=false
upstream:
  application base=0x1000 bytes=140737488351232
  region_exclusive_end=0x800000000000 contains_64KiB=true
```

This confirms a shared-helper/layout defect worth fixing. It does not establish
an exploit or prove every active allocation path uses that helper. In particular,
`region_type` currently has no in-tree caller. Review actual arena/range consumers
and architectural HHDM placement before changing boundaries. Our active-paging
width detection is already corrected and deliberately rejects unsupported LA57;
retain that behavior rather than adopting upstream's optional 57-bit report.

## Work that should not replace our implementations

- **IPI/TLB changes (`7539cee9`):** upstream uses unbounded IPI queues and local
  invalidation dispatch without the acknowledged cross-LP retirement protocol
  our current code/documentation requires. Its ACPI documentation explicitly
  says synchronous cross-CPU shootdown is absent, so it never reuses retired
  ACPI virtual addresses or paging structures. Our owned epochs, exact-root
  retirement and admission contracts must remain authoritative.
- **Stack caching and shared-frame bookkeeping (`716a5523`, `52e2ad85`,
  `808015f3`):** these solve a different lifecycle model. Upstream's global
  stack cache grows through a Vec; shared-frame management is a separately
  locked HashMap with scalar counts. Do not replace admitted stack/frame
  preparations, complete retirement owners or existing shared-memory objects.
- **Small CPU compatibility changes (`ef751397`):** upstream reads TSC_AUX with
  RDMSR instead of RDTSCP. Our RDTSCP requirement is explicit, and our GS base is
  a per-CPU entry area rather than a scalar processor ID. Investigate only as a
  deliberate CPU-support change; never copy the surrounding FS/GS setup.
- **Logging rewrite (`3b7ebb8a`):** bounded 512-byte formatting chunks and panic
  paths are useful reference ideas, but the replacement introduces its own
  global emission/IRQ/console contract. We already have early serial, panic
  reporting and SBSA-specific support. Compare those contracts individually,
  including the current display-overrun incident, rather than replacing them.
- **ECAM access wrappers (`a65abbb3`, `72cc1417`):** centralize access widths and
  write readback/ordering. Compare the device-register contracts with our
  volatile access and MSI-X fences before porting; assembler spelling alone is
  not evidence of a correctness improvement.
- **PCI IDs/pretty printing, CI and formatting:** useful optional tooling, but
  low priority for the audit. Our CI also boots native security profiles and
  tests host libraries; upstream image-build jobs explicitly do not establish
  boot/runtime support. Upstream AArch64/RISC-V code still contains incomplete
  platform implementations; it is not a replacement for our working ARM port.

The two branch-only additions do not change the recommendation:
`interrupt_refactor` adds an older entry-frame/dispatcher skeleton;
`mdp-devel` adds an unfinished stage0/stage1 startup split. Neither is a ready
integration candidate for our current entry/boot ownership model.

## Recommendation about detaching

The implementations have diverged enough that a wholesale merge would be a
large integration project and would not preserve our audit contracts by
construction. Maintain this as an independent project, port the address-map
and MCFG/PCI correctness ideas in discrete reviewed batches, and keep the newer
terminal/firmware work as explicit future candidates. Independent GitHub
hosting does not prevent later fetching/comparing the upstream Git repository.
Keep the existing upstream attribution and commit provenance.

GitHub's current [detachment documentation](https://docs.github.com/en/pull-requests/how-tos/work-with-forks/detaching-a-fork?platform=linux)
says leaving the fork network is permanent, preserves Git commit metadata but
does not preserve issues, PRs and other repository metadata, and has eligibility
conditions. Inventory metadata before choosing a detachment method. No
detachment, merge, cherry-pick or remote-configuration change was performed in
this review.
