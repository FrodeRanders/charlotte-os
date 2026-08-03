# EL2 Capability Root — Design Sketch

> A possible security design for CharlotteOS: root the OS's capability
> authority in the EL2 layer, so that even a compromised EL1 kernel cannot
> forge, mint, or escalate capabilities. Companion to
> `docs/real-hardware-roadmap.md` (Phase 1 / "Why EL2").

## Goal

CharlotteOS is a capability OS: every authority (memory objects, connections,
device MMIO, interrupts, DMA domains, name-service bindings) is a capability,
and authority flows only by delegation. Today the capability table lives in
the EL1 kernel. If the kernel is compromised, the whole authority model is
compromised. This sketch moves the *root of authority* to EL2 so a
compromised kernel can only use the capabilities it was granted — it cannot
mint new ones.

## Threat model assumed

- **In scope**: a memory-corruption / code-injection bug anywhere in the EL1
  kernel or an EL0 domain is contained — the attacker cannot forge
  capabilities, cannot revoke-on-behalf, cannot gain access beyond what the
  kernel already held.
- **Out of scope**: a bug in the EL2 layer itself (the EL2 code must be small
  and auditable), physical attackers, and side-channel attacks that do not
  violate capability integrity.

## Design outline

### The split

- **EL2 owns**: the authoritative capability table (the "root"), the
  capability *mint* authority, and a small capability-check interface.
- **EL1 (the kernel) owns**: scheduling, drivers, the syscall ABI, the name
  service — everything else, but it must ask EL2 to create or extend any
  capability it does not already hold.
- **EL0 domains** keep their current stage-1 isolation; with VHE, EL2 adds a
  stage-2 grant layer on top (defense-in-depth).

### The interface (capability operations become EL2 calls)

Because CharlotteOS already funnels authority through a small set of
operations, the EL2 surface stays tiny:

- `cap_mint(parent, rights) -> cap` — create a child capability with a subset
  of the caller's rights (EL2 checks the parent exists and the rights are a
  subset).
- `cap_delegate(cap, domain) -> cap` — hand a capability to another domain's
  table (EL2 updates both tables atomically).
- `cap_derive(cap, template) -> cap` — the existing capability-derivation
  rules, enforced at EL2.
- `cap_check(domain, op, args) -> ok/deny` — EL1 asks EL2 whether a
  capability-backed operation (map page, bind IRQ, open device window) is
  permitted *before* it is performed.

The key property: **there is no EL1-writable path to add a row to the root
table.** `cap_mint` and `cap_delegate` are the only writers, and both are EL2
services. Even if EL1 has a use-after-free that lets it write arbitrary
kernel memory, the root table is at EL2 and not mapped at EL1 (or mapped
read-only / stage-2-restricted).

### What stays at EL1

The kernel keeps its current behavior for capability *use* (it reads its own
granted subset, performs the actual MMU/GIC/device operations), but every
operation that *changes* the authority set crosses to EL2. This preserves the
existing architecture and syscall ABI; only the authority layer moves.

## Why this fits CharlotteOS well

- The capability model already has the right abstraction — this is not a new
  security mechanism, it's relocating its root.
- The operation set is small and well-bounded (the `ipc`, `device`, and
  `service` layers already reduce authority to a few call shapes), so the EL2
  surface can be audited.
- It pairs naturally with the SMMU: EL2 stage-2 polices CPU accesses to
  granted memory, the SMMU polices DMA — capability enforcement is enforced
  on both sides of every transfer.
- It extends the existing trust argument: EL0 drivers are already isolated
  from the kernel; EL2 isolation makes the kernel itself a "trusted but
  bounded" component rather than the root of trust.

## Open questions / risks

1. **VHE vs separate hypervisor.** Under VHE the "EL2 layer" is a mode of the
   same kernel — simpler, but the kernel still controls both sides of the
   interface, weakening the "compromised kernel" guarantee unless the
   capability code is kept deliberately separate and stage-2 protects the root
   table. A thin, fixed hypervisor gives the stronger property at higher
   complexity.
2. **Performance.** Every mint/delegate crosses an exception boundary; the
   interface must batch and avoid per-syscall overhead where possible.
3. **Bootstrapping.** The EL2 root must be initialized before EL1 runs and
   measure/verify EL1 (attestation), otherwise the guarantee is moot.
4. **Migration path.** The emulated targets run at EL1; the capability layer
   must keep working in a "root at EL1" fallback mode so development and the
   18/18 suite stay green while EL2 support lands.

## Relation to the roadmap

Phase 1 (EL2 readiness) enables the layer; capability-rooting is the
distinctive security payoff and is intentionally listed first in the
"why EL2" section. It should be staged after VHE works and before guest
hosting, because the isolation it provides is the foundation the rest can
build on.
