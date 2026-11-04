# Boot allocation boundaries

This is the current Q3 producer-review contract, not a complete heap inventory
or an exclusion for everything initialized through `LazyLock`. The
[closure plan](security-closure-plan.md) retains the remaining review and
aggregate-capacity gates.

## Logical processors

`startup::LP_COUNT` validates the firmware response as **1 through 256** before
conversion or publication. The limit comes from
`system_scheduler::MAX_TRACKED_LPS`, also used by logical-ID assignment and the
fixed scheduler/reaper tables. BSP entry forces this allocation-free count
before ISA initialization, heap-backed logging or scheduler construction.
All per-LP factories obtain their size through this validated count. An invalid
count stops mandatory boot; it does not allocate a truncated prefix or start
secondary processors. Runtime hotplug is not implemented.

This establishes a count bound for per-LP scheduler objects, interrupt-save
arrays, boxed shards, hardware timer holders and boot IPI queues. It does not
alone qualify lazy initialization context or aggregate heap size. Q3 must still
record each concrete factory's first-use phase and lifetime; Q4 must count
per-LP storage and allocator overhead together with other pools. In particular,
x86's three 16 KiB interrupt-stack arrays require up to 12 MiB at 256 LPs before
their tables and other per-LP storage. No resource ceiling was raised by this
check; changing the shared limit needs that capacity review.

## Framebuffer terminal

The optional `display` profile initializes one terminal from the bootloader
framebuffer on first ordinary logging after heap initialization, before EL0
services. The fixed 8-by-16, 256-glyph font and no-canvas configuration feed the
local `flanterm_bindings` adapter. Successful initialization allocates six
objects: the complete backend context, font bits, font booleans, cell grid,
pending-update queue and cell-to-queue map. The last three depend on framebuffer
geometry; they are not independently admitted runtime pools.

`BootContext` retains the C context and all its child allocations for the boot
lifetime. Its pointer is private and console operations are serialized by
`FT_CTX`. Abandoning the Rust handle is inert. Do not adopt its base pointer as
`Box<flanterm_context>`: C allocated a larger `flanterm_fb_context` with the
callback's alignment. A future replacement/retirement path must consume the
complete owner, call the backend's size-aware `deinit` after its last use, and
qualify its outer guards/IRQ context before release. The current static owner
has no such path.

The local dependency returns null when its first allocation fails, before
reading any child pointer. Later failures free their complete initialized
prefix through the same size/alignment callbacks. The Rust callback returns
null for zero or invalid layouts rather than panicking. The pinned C sources
contain allocation calls only in initialization, not the write/escape parser;
host controls also exercise scrolling and escapes without new allocations.
These controls qualify local heap cleanup, not hardware teardown or arbitrary
caller/IRQ context. Provenance and the narrow C patch are recorded in
[`CHARLOTTE.md`](../../crates/flanterm-bindings/CHARLOTTE.md).

**Remaining Q3 requirement:** validate framebuffer format, pitch, dimensions,
checked address/size arithmetic and a reviewed byte bound before entering C.
Current nonzero/null checks are insufficient to justify a fully bounded boot
exclusion; the C backend assumes valid geometry. No application-triggered
allocation claim follows merely from the presence of these compiled callbacks.

## Remaining review

The current supported firmware path is ACPI. An ARM `devicetree`-only strict
compile fails at four ACPI references in `cpu/isa/aarch64/power.rs` and
`device/smmu.rs`; the device-tree module is a stub and PCI segment discovery is
`todo!`. That optional profile has no operational qualification. Its existence
as a Cargo feature does not establish a reachable FDT allocation producer or a
supported boot exclusion. Complete discovery/power/IOMMU integration before
claiming support; this boot sub-batch does not implement it.

Per-LP first-use/capacity accounting, PCI/ACPI traversal and firmware-derived
collections, launch encoders and other dependency factories still need explicit
trigger/bound/lifetime assignments. Compiled unused helpers need call-site
evidence rather than an application-reachability assumption. Address-space
frame-tracking vector overhead belongs in Q4's original-principal/capacity
review; enclosing guards and implicit destruction remain Q5. This page closes
neither the Q3 inventory nor SEC-07/18.
