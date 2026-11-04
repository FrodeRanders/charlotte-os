# Boot allocation edges — Q3 sub-batch

Date: 2026-11-04. Base: `6e62a0e2`. This records a discrete producer-review
sub-batch within [Q3](../../reference/security-closure-plan.md), not acceptance of
the whole package. Q1/Q2 remain accepted, Q3–Q8 and all aggregate gates remain
open. Finding dispositions are unchanged.

## Corrections and limits

The firmware CPU count is checked against the shared scheduler capacity before
conversion or publication. BSP entry forces that allocation-free check before
ISA initialization or heap-backed per-LP factories. Counts zero and above 256
reject mandatory boot. The previous logical-ID check ran too late to bound
those earlier constructors. No limit was increased; aggregate storage at the
maximum remains a Q4 obligation.

The optional framebuffer console now retains a private opaque C context for
the boot lifetime. The previous `Box<flanterm_context>` adopted a base pointer
to a larger `flanterm_fb_context` allocated with the callback's alignment.
Abandonment now performs no partial Rust deallocation. A future retirement
path must explicitly consume the complete backend through its C deinit routine
after last use and qualify the outer context. There is no such production
retirement path in this batch.

Failure injection also exposed a dependency defect: the first context
allocation returning null jumped to `fail`, which accessed `ctx->canvas`
before checking `ctx`. The original 1.1.4 backend crashed the host control with
SIGSEGV. A local copy adds the null guard before any child access. All remaining
initialization and success-path C code is unchanged. The registry checksum,
original file hashes and license are preserved in
[`UPSTREAM.json`](../../../crates/flanterm-bindings/UPSTREAM.json) and
[`CHARLOTTE.md`](../../../crates/flanterm-bindings/CHARLOTTE.md). The lockfile
changes only this same-version dependency's source to the local path. Rust
callback zero/invalid layouts now return null rather than panic.

The current [boot allocation contract](../../reference/boot-allocation-boundaries.md)
records trigger, owner and lifetime plus the remaining geometry/budget work.
This does not qualify arbitrary framebuffer dimensions, pitch, address
arithmetic, hardware teardown, every lazy first-use context or all dependency
producers. Kernel `display` is compile-checked on both architectures; native
framebuffer execution here is ARM only.

## Qualification

The [self-contained evidence](evidence/2026-11-04-boot-allocation-edges.json)
records source/dependency identities, commands, diagnostic excerpts and native
artifact hashes. No temporary log is required to interpret the results.

The new host harness compiles the actual startup and console Rust sources and
the local C backend. CPU counts 1, 4 and 256 succeed; 0 and 257 reject. All six
C allocation-failure prefixes release every initialized allocation with its
original size/alignment. Successful initialization makes six allocations;
100 scrolling/escape sequences allocate no more, and Rust console abandonment
retains all six. Explicit host-only C deinit then releases all six exactly.
An unsafe base-context deallocator control is rejected before any attempt to
deinit an already partially released backend. Zero/overflow-layout callbacks
also reject. The host mutex adapter supplies serialization, not native IRQ
qualification.

Full host suite and workspace formatting pass. Strict minimal/test ACPI checks
pass on both architectures, as do optional display checks. Fresh four-CPU
native runs use the existing signed bundles, actual upgrade/handoff and
security verifiers with a 240-second outer deadline: ARM display **19/19**,
Intel **17/17** and AMD **17/17** pass. Services/SDK and persistent formats did
not change; existing signed test bundles were reused. Kernel assembly-section
checks also pass for all three images.

The final four-profile MIR census contains 2,812 coalesced records with zero
missing source spans. Display adds one allocator callback and three implicit
drop records to the 2,808-record minimal union. The evidence retains profile
hashes, counts, source identities and those four added records. Dependency C
bodies require the separate source/control review above; the census neither
counts actual allocations nor proves application reachability or lock safety.

## Failed probes and remaining work

The first host fixture incorrectly assumed C `max_align_t` had 16-byte alignment
on this Darwin host. It failed with `left: 8, right: 16` before entering the
backend. The harness now derives alignment from a compiled C probe. The next
run reached the original backend and died with SIGSEGV on the first allocation
failure. Those are distinct failures; the former is a fixture assumption, the
latter has a specific null-context failure path and a passing corrected
control. Temporary original binaries are unavailable; the source identities,
failure diagnostics and correction are retained here and in the evidence.

An additional ARM `--no-default-features --features devicetree` strict compile
fails with four E0433 errors: `power.rs:3` and `smmu.rs:48,623,639` refer to the
configured-out ACPI module. Device-tree discovery is a stub and PCI segment
discovery is `todo!`. That optional profile is unsupported; it has no successful
qualification or reachable FDT producer established by this review. Completing
its firmware/power/IOMMU integration is separate from these three corrections.

Q3 still requires framebuffer geometry and byte bounds, remaining concrete
producer/factory assignments, PCI/firmware/launch dependency review and
per-LP first-use/lifetime reconciliation. Frame-tracking/control and allocator
overhead remain Q4; caller/IRQ/implicit destruction remains Q5. The new defects
are corrected in their stated scope without accepting Q3 or closing SEC-07/18.
