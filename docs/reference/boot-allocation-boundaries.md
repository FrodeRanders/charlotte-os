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
| x86 `LAPICS` boxed shard | Explicit BSP preparation before AP startup; controller setup fills each owning LP slot | One fixed shard per validated LP, boot lifetime. Timer state lives separately in its single prepared platform slot; controller initialization no longer calibrates a duplicate timer. |
| IPI command queues | Explicit BSP force before AP startup; 256 bounded slots per validated LP | Boot lifetime. Pinned `concurrent-queue` allocates the fixed slot buffer during construction; push/pop/force-push do not grow it. Eviction/progress policy remains Q7. |
| `TIMER_QUEUES`, ARM `GENERIC_TIMERS`, x86 `APIC_TIMERS` and per-slot Arc | Explicit BSP-only allocation before AP startup; hardware initialized once on each owning LP after its controller and before `INTERRUPT_INIT_BARRIER` | Fixed validated LP-count storage, boot lifetime. Runtime access observes prepared/ready state and clones existing references; it cannot initialize storage or hardware. Event nodes retain independent admission. Broader timer caller/destructor qualification remains Q5. |
| x86 `DYN_IH_MATRIX` | Explicit BSP preparation before AP startup; 219 handler slots per validated LP | Runtime registration/lookup cannot construct missing backing. The assembly symbol has the actual dispatcher receiver type, with the prepared table behind its Once; no LazyLock-layout reinterpretation. Boot lifetime, no runtime growth. |
| `ShardLocal`, `ShardMailboxSet`, `try_run_on_lp` | Current concrete callers are native self-tests only | Compiled generic helpers are not application reachability. Production use requires a new trigger/bound/owner review. |
| `INT_DEPTH` and ARM `ACKED_INTID` | No current production caller found for the depth exports or the ARM record/argument-less EOI path; ARM dispatch calls `end_of_int(intid)` directly | Compiled unused helpers, not a runtime allocation finding. Review any future caller; symbol visibility is not reachability. |
| Deferred-work manager | Eager BSP construction, but `DeferredTask` is uninhabited and no worker is started | Pinned `ConcurrentQueue::unbounded()` stores null head/tail inline and allocates blocks only on push. There is no safely constructible current task or production submission. Adding a task variant requires bounded admission and an actual worker contract. |

These assignments describe the current call graph; they do not qualify every
outer guard or all implicit destruction. In particular,
x86's three 16 KiB interrupt-stack arrays require up to 12 MiB at 256 LPs before
their tables and other per-LP storage. No resource ceiling was raised by this
check; changing the shared limit needs that capacity review.

### Bootstrap timer phases and firmware discovery

The BSP allocates queue tables and all platform timer Arcs after heap/policy
initialization and before releasing AP startup. Allocation failure stops mandatory
boot; these fixed, validated platform-count objects are not admitted runtime
pools. Timestamp-period initialization also precedes AP startup. ARM construction
only prepares metadata; it no longer writes the BSP timer once per remote slot.

Each LP initializes its own hardware with CPU interrupts already masked, after
its local interrupt controller is ready. The slot transitions Prepared to
Initializing to Ready. Hardware/setup failure retains Initializing and forbids
replay or use; only completion publishes Ready after the local timer guard is
released. Runtime get reads a completed slot and clones its existing owner. It
never allocates, spins on an initializer or programs hardware. Queue and dynamic
IRQ lookups likewise reject missing bootstrap storage. The BSP verifies every
LP is ready after the common interrupt barrier, independently of `kernel_tests`.

x86 calibrates its one authoritative timer state using ten complete, positive
samples and checked accumulation, with no sample vector. A timeout, zero sample
or overflow cannot publish a usable timer. The existing bounded hardware poll
remains boot-only; no scheduler readiness contract exists before timer startup.
The controller no longer calibrates a separate unused copy.

ACPI's catalog is also prepared before AP startup. Table lookup returns a static
borrow instead of cloning a vector for each DMA request. Its XSDT length bounds
address cardinality to at most 8187; catalog growth/control overhead remains Q4.
IORT discovery validates the node count against the actual remaining table bytes
and a minimum 16-byte node before reserving its exact vector. At the 64 KiB table
ceiling there can be at most 4093 nodes. Invalid node/mapping geometry returns
unsupported discovery before hardware setup; allocation failure stops boot.
Temporary node storage is released on that BSP-only parse, outside device/backend
publication guards. The immutable optional SMMUv3 configuration is retained once;
runtime stream-ID translation neither reparses firmware nor allocates metadata.
This changes no requester translation, backend reset or DMA completion policy.

The [bootstrap qualification](../reports/audits/2026-11-05-bootstrap-storage.md)
records actual tests and limitations. Serial host adapters qualify allocation
phases and parser/cache behavior, not IRQ masking or physical hardware. Native
minimal boots must observe mandatory preparation without a regression fixture.

### Hardware-ASID return storage

ARM prepares its tag allocator immediately after BSP heap initialization, before
user roots can acquire hardware ASIDs. The captured hardware width selects
256 or 65536 u16 return slots: **512 bytes or 128 KiB** of requested boot backing.
Zero and the firmware's kernel tag remain excluded. This generous complete
reserve removes growing allocation from tag return without limiting usable tags
or introducing a second reclamation mechanism. Boot allocation failure stops
initialization; runtime access rejects missing preparation. Global allocator
headers and aggregate headroom remain Q4.

Allocate/return only moves values in that prepared vector. A capacity assertion
precedes push. The root still owns its tag linearly, and its existing confirmed
TLBI/DSB boundary still precedes return. Preparation supplies neither quiescence
nor reset/retry authority. Host controls exhaust and recycle 8/16-bit ranges,
reject backing failure before issuance, and reject a growing-return control.
Native campaigns qualify only their captured hardware width.

### Boot launch encoders and firmware diagnostics

Compiled cluster service input and its encoder reject more than the shared
protocol's 16-service limit before collecting bindings. These temporary vectors
and encoded pages belong to steady-state boot composition, not an application
input API. Kernel S3/Kafka/Kafka-step builders have only optional fixture callers;
production operational launch uses the admitted encoded profile instead.

CPU identification produces fixed-size 12/48-byte strings during finish_boot.
The one-time ACPI table diagnostic is bounded by the catalog; unknown signatures
format their four borrowed bytes without allocating or assuming UTF-8. The
inactive allocating PCI capability walkers have been removed; active driver
lookups retain their existing bounded, claim-aware paths.

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

## Address regions and PCI firmware topology

The software address map validates contiguous, nonempty lower/upper regions
before publication. Containment uses checked unsigned offsets; exclusive ends
are raw, checked addresses rather than canonicalizing arithmetic. The 48-bit
stack/MMIO/heap bases stay unchanged. ARM retains raw TTBR1/HHDM addresses;
the shared lower software region ends at the sign-bit boundary. Model controls
cover 39/48/57-bit layouts, but do not add native LA57 or device-tree support.
Virtual extents neither reserve physical heap nor raise a runtime pool ceiling.

Mandatory ACPI boot validates all MCFG tables before any ECAM mapping. Require
complete 44-byte headers and 16-byte entries, matching declared length, checksum,
zero reserved fields, ordered bus ranges and checked physical extents within
the architecture mask. At most **32 windows** are accepted across all tables;
same-segment bus overlap and overlapping advertised physical ECAM slices reject.
A rejected table leaves the parser's prior private prefix unchanged. Firmware
still supplies actual mapped table backing; this is not a provenance proof.

ECAM bases remain bus-zero-relative, including nonzero starting buses. The
mapper retains its full 256 MiB boot window and accepts 1 MiB-aligned bases,
using 4 KiB leaves when 2 MiB leaves cannot encode the base. These are permanent
shared-kernel mappings. Partial mapping or metadata failure stops mandatory boot;
it does not unmap, replay or adopt a failed hardware prefix. Firmware resource
reservation reconciliation remains outside this validation boundary.

Each window has one fixed 256-entry bus worklist and visited set. Mark identities
at discovery, including empty buses; ignore repeated and out-of-range bridges.
A flat immutable bus collection replaces recursive ownership. Reserve its exact
advertised bus capacity before mapping; each bus prepares at most 32 devices
privately before one exact vector reservation. Separate global quotas cap
accepted device slots and functions at **16,384 each**, including bridges.
Endpoint Box and vector allocation are fallible; a rejected private metadata
prefix drops its heap storage without hardware cleanup. Endpoint Boxes stay
stable for reset-claim cells. Borrowed iterators replace allocating lookup stacks
and cover all eight multifunction positions, keeping depth-first sibling order.
The existing reset backend/claim/retirement scope is unchanged.

These generous limits support many more functions than current test machines
without complicated incremental boot admission. Their compile-time requested
metadata bound is at most **16 MiB**, plus allocator overhead and page tables
assigned to Q4. Raising a ceiling requires reviewing that formula, scan latency,
page-table cost and aggregate headroom. The 32 full ECAM windows consume at most
8 GiB of virtual MMIO space, not 8 GiB of heap. This boot-only producer has no
runtime hotplug growth path.

Host controls exercise range boundaries, malformed and overlapping tables,
atomic rejection, exact quota exhaustion, all 256 buses and allocation-free
preparation/worklists; restoring old containment fails. Native fixtures run the
real builder on private RAM with cycles, out-of-range bridges, function seven,
stable endpoint identity and rejected function prefixes. They do not inject
allocator OOM or qualify a real 1 MiB-only-aligned ECAM platform. See the
[selective-port checkpoint](../reports/audits/2026-11-05-selective-upstream-ports.md)
for executed native profiles and artifact identities.

## Remaining review

The current supported firmware path is ACPI. An ARM `devicetree`-only strict
compile fails at four ACPI references in `cpu/isa/aarch64/power.rs` and
`device/smmu.rs`; the device-tree module is a stub and PCI segment discovery is
`todo!`. That optional profile has no operational qualification. Its existence
as a Cargo feature does not establish a reachable FDT allocation producer or a
supported boot exclusion. Complete discovery/power/IOMMU integration before
claiming support; this boot sub-batch does not implement it.

The complete current producer/dependency/profile assignments live in the
[kernel allocation review](kernel-allocation-review.md), including bounded boot
storage, fixture-only helpers, existing admitted owners and explicit Q4 work.
Address-space frame-tracking vectors and diagnostic snapshot scratch still need
aggregate/original-principal backing coverage; enclosing guards and implicit
destruction remain Q5. These assignments do not close SEC-07/18.
