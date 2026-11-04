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

This establishes a count bound, not automatic qualification of lazy first use
or aggregate heap size. The current concrete dispositions are:

| Producer | Trigger and bound | Lifetime / disposition |
| --- | --- | --- |
| `RoundRobin` Box and scheduler BTreeMap | One installation per LP in BSP/AP initialization before `INIT_BARRIER`; at most 256 | Boot lifetime. Idle threads and observer children retain their existing admitted owners; scheduler container/allocator overhead remains Q4. |
| x86 AP interrupt/DF/NMI stacks, TSS/GDT/IDT arrays | Secondary ISA initialization; at most 255 AP entries | Boot lifetime; three 16 KiB stack arrays plus table arrays and temporary construction buffers. No runtime hotplug producer. Count total/peak overhead in Q4. |
| `INT_STATE`'s three vectors | First mandatory post-heap BSP logging before AP startup; each has validated LP-count length | Boot lifetime; later interrupt-save/restore does not grow these vectors. Allocator capacity/overhead remains Q4. |
| x86 `LAPICS` boxed shard | `LocalIntCtlr::init_lp` in bootstrap, before the interrupt-initialization barrier | One fixed shard per validated LP, boot lifetime. No later growth. |
| IPI command queues | Explicit BSP force before AP startup; 256 bounded slots per validated LP | Boot lifetime. Pinned `concurrent-queue` allocates the fixed slot buffer during construction; push/pop/force-push do not grow it. Eviction/progress policy remains Q7. |
| `TIMER_QUEUES`, ARM `GENERIC_TIMERS`, x86 `APIC_TIMERS` and per-slot Arc | First timer enqueue/rearm can initialize global storage; x86's Once constructs its Arc/calibrates on the calling LP | Fixed LP-count cardinality, boot lifetime once created. **Required Q3/Q5 work:** explicit bootstrap construction/owning-LP initialization before runtime first use. Local interrupt-controller initialization does not preconstruct the separate x86 Arc timer. Event nodes retain their existing independent admission. |
| x86 `DYN_IH_MATRIX` | First device vector registration/lookup; 219 handler slots per LP | Fixed boot-lifetime storage, but a boot with no earlier eligible vector registration lacks an unconditional initialization boundary. Include it in the bootstrap-storage batch and qualify caller/IRQ context. |
| `ShardLocal`, `ShardMailboxSet`, `try_run_on_lp` | Current concrete callers are native self-tests only | Compiled generic helpers are not application reachability. Production use requires a new trigger/bound/owner review. |
| `INT_DEPTH` and ARM `ACKED_INTID` | No current production caller found for the depth exports or the ARM record/argument-less EOI path; ARM dispatch calls `end_of_int(intid)` directly | Compiled unused helpers, not a runtime allocation finding. Review any future caller; symbol visibility is not reachability. |
| Deferred-work manager | Eager BSP construction, but `DeferredTask` is uninhabited and no worker is started | Pinned `ConcurrentQueue::unbounded()` stores null head/tail inline and allocates blocks only on push. There is no safely constructible current task or production submission. Adding a task variant requires bounded admission and an actual worker contract. |

These assignments describe the current call graph; they do not qualify every
outer guard or all implicit destruction. In particular,
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

Before calling C, `FramebufferLayout` requires RGB32, three nonoverlapping
8-bit channels within the 32-bit word, a nonnull 4-byte-aligned address, a
4-byte-aligned pitch covering each row, at least one complete glyph, and checked
dimension/stride/address arithmetic. The complete `height * pitch` span must
fit **256 MiB** without address wrap. Actual mapped backing still comes from the
trusted bootloader; this is not a page-ownership or mapping-provenance proof.

The plan fixes the backend's existing 1/2/4 font scaling explicitly and sums all
six requested allocation layouts before entering C. The sum, including the C
context and both font representations, must fit **12 MiB**. Cell counts and
sizes use checked arithmetic and the actual generated C type sizes. Canvas,
spacing and margins remain disabled. Unsupported metadata returns an unavailable
console before allocation or drawing.

These ceilings accept 1080p, 4K and ordinary 8K layouts with their existing font
scaling; 7680-by-4320 requires less than 1 MiB of requested heap. A 256 MiB
framebuffer aperture is not a heap reservation. The generous 12 MiB heap limit
also permits unusual tall/wide layouts without cropping, while bounding their
three cell catalogs. Very large apertures, excessive padding or cell catalogs
can be rejected even if a bootloader could display them. Raising these limits
requires reviewing arithmetic, boot latency and Q4's aggregate headroom; no
existing runtime pool limit or physical heap reserve changes here. Allocator
headers/alignment overhead for the six allocations remains Q4, separate from
the requested-byte bound.

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

Host controls reject malformed metadata without C entry, exercise both byte
ceilings, compare the six actual C allocations against the checked plan, and
verify minimum/padded/alternate-RGB/scaled rendering with guard words and
untouched row padding. A missing heap-limit control is rejected before entering
C. Native qualification remains scoped to the executed display profile; this
is a bounded trusted-boot producer, not application-controlled heap growth.

Current qualification (2026-11-05): full host and strict minimal/test display
checks pass on ARM and x86. Two ARM display/security campaigns accepted the
800-by-600/230056-byte console but failed the second grant probe's unchanged
45-second deadline. An identical-kernel/service/initial-disk control without
the framebuffer device passes 19/19. Full native display qualification remains
open; this does not identify the overrun's exact phase or cause. See the
[preserved comparison](../reports/audits/2026-11-04-framebuffer-bounds.md).

## Remaining review

The current supported firmware path is ACPI. An ARM `devicetree`-only strict
compile fails at four ACPI references in `cpu/isa/aarch64/power.rs` and
`device/smmu.rs`; the device-tree module is a stub and PCI segment discovery is
`todo!`. That optional profile has no operational qualification. Its existence
as a Cargo feature does not establish a reachable FDT allocation producer or a
supported boot exclusion. Complete discovery/power/IOMMU integration before
claiming support; this boot sub-batch does not implement it.

The next identified Q3 corrections are the runtime-reachable bootstrap storage
above and bounded PCI/MCFG enumeration. PCI's depth-eight check does not prevent
repeated visits/branch multiplication or enforce a segment's advertised bus
range. MCFG's generic SDT validation permits lengths shorter than its additional
eight-byte reserved header, before subtracting that header and allocating the
segment vector. Require complete entry geometry and a reviewed segment bound
before publication/mapping, and one bounded visited-bus identity set per segment
before descending. Treat this as a discrete firmware/traversal batch; do not
claim a bounded exclusion from depth alone.

Other PCI/ACPI collections, launch encoders and dependency factories still need
explicit trigger/bound/lifetime assignments. Compiled unused helpers need
call-site evidence rather than an application-reachability assumption. Address-space
frame-tracking vector overhead belongs in Q4's original-principal/capacity
review; enclosing guards and implicit destruction remain Q5. This page closes
neither the Q3 inventory nor SEC-07/18.
