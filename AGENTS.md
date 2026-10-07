# CharlotteOS contributor instructions

These instructions apply to the entire repository.

## Preserve the capability model

CharlotteOS uses linear capability ownership. In userspace services and
applications, do not store an owning capability as a bare integer and do not
build manual cleanup ladders around early returns.

- Use the types in `catten_rt::owned`: `OwnedMemory`, `MappedMemory`,
  `Completion`, `ReadOperation`, `Endpoint`, `Connection`, `ConnectionRef`,
  `PendingCall`, `IncomingMessage`, `ReplyToken`, `MmioRegion`, `Interrupt`,
  and `DmaTransfer`.
- Use protocol-specific owners, such as `catten_services::socket::OwnedSocket`,
  for remote resources represented by scalar IDs.
- Make ownership transfer consume the Rust owner. Use `call_move`, `send_move`,
  or `reply_move`; do not call `into_raw` and then reconstruct cleanup logic.
- Keep borrowed memory behind a Rust borrow until the pending call terminates.
- Put every transient resource for a multi-step operation in one owning struct.
  Dropping that struct must cancel/release the whole operation.
- Use `Drop` for local, infallible release. Give fallible or blocking remote
  teardown an explicit consuming `close(self) -> Result<...>` and retain a
  best-effort `Drop` fallback.
- Use `from_raw` only at a documented ABI boundary where ownership transfers
  exactly once. Never adopt the same handle twice or use it after adoption.
- Borrow launch-owned capabilities through `Context` (for example,
  `bootstrap_connection()`); do not adopt them as owned capabilities.

Direct resource-owning calls from `catten_syscall` belong in `catten-rt`, the
kernel/runtime boundary, or hardware/protocol adapter code that cannot yet be
expressed by the owned API. New exceptions require a comment explaining the
missing abstraction. CQ operations, status/config access, logging, and terminal
`thread_exit` do not by themselves own a closeable resource.

See `docs/guides/resource-ownership.md` for examples and the review checklist.

## Editing and validation

- Preserve unrelated work in the tree; do not discard or rewrite user changes.
- Use `cargo fmt` rather than hand-adjusting rustfmt output. Prefer raw strings,
  named constants, or `concat!` for long protocol/HTML literals that otherwise
  depend on line-continuation whitespace.
- Build bundled AArch64 services with `scripts/build-catten-services.sh`.
- Scope `global_asm!` sections with `.pushsection`/`.popsection`; blocks share
  assembler state within a codegen unit. Native kernel trampolines must remain
  in executable, read-only ELF sections and load segments. Boot runners enforce
  this through `scripts/check-kernel-asm-sections.py`.
- Run `cargo fmt --all -- --check` after Rust edits. All Rust packages belong
  to the root workspace even when they use separate build targets.
- For ownership changes, test success, submission failure, mapping failure,
  cancellation/drop, and returned-capability cleanup where practical.

## Architectural boundaries

- Kernel admission policy is published once through
  `service::admission::PreparedBootTrust`. Cluster/deployment manifests borrow
  its `BootTrust` view; never configure their policy independently of the kernel
  gate or overwrite an installed boot policy. Signed preparation consumes a
  zeroizing recipient-key owner and checks its public binding. It does not
  authenticate the platform, install rollback-resistant state or provide
  custody; production remains disabled until those boundaries are implemented.
- Internal APIs and wire formats have no backward-compatibility requirement.
  Remove compatibility-only branches when a coherent replacement is ready;
  do not preserve an unsafe allocation path for older callers.
- New kernel capability publication uses `capability::Reservation`: reserve
  before payload/ownership mutation and publish under the subsystem's
  serialization. Retain the captured namespace identity, not just its ASID.
  `reserve_captured` requires that registry's captured generation and guard;
  do not acquire lifecycle under a subsystem guard. A source move or loan needs an
  owning payload transaction as well as capability escrow; use `PreparedTransfer`
  and `commit_transfers` rather than reconstructing scalar cleanup. There is
  no unbounded allocator or retirement bypass for any of the six kinds.
  The scalar restoration API is removed. See
  `docs/reference/capability-admission.md` for the current enforcement scope.
  IPC calls use `PreparedCall`/`PreparedConnection` to own metadata and fresh
  authority alongside their attachments. Compose reservations through
  `commit_undelivered_transfers_with_authority` while retaining every affected
  payload registry; do not publish memory and then attempt fallible IPC admission.
  IPC moves/copies, including asynchronous sends and returned memory, remain
  inaccessible until receive or first reply observation. Ordinary memory lookup
  must reject their pending delivery state, including explicit close, mapping,
  copy/move/loan and DMA. Publish delivery under IPC with the exact queue/result
  still owned; readiness waiting alone does not transfer ownership. Non-IPC
  transfers use `commit_transfers`/`commit_transfers_with_authority`. Loans retain
  their owned revocation contract. Queued and unobserved returned connections
  also retain pending delivery in their admitted IPC entry. Ordinary IPC lookup
  rejects them, including send/call, mint/delegation, watches, management target
  resolution and explicit close. Publish their delivery under the same IPC hold
  as dequeue/first observation. Only internal cleanup may consume hidden grants;
  endpoint-reference accounting must still retain their backing metadata. Direct
  mint/delegation and launch grants remain immediately usable. Do not restore a
  queue-scan qualification fallback for hidden connection or memory sources.
  Returned memory uses ordinary visible-source `PreparedTransfer` admission;
  hidden or borrowed sources reject before loan mutation and retain their
  original queue/call ownership.
  Device grants take lifecycle before device/backend registries, reserve before
  hardware creation and retain a `PreparedDmaDomain` until publication. Failed
  hardware rollback must quarantine reachable backing, never recycle it.
- Demand-backed heap frames use the embedded address-space `heap_account`.
  Retain the exact generation and table guard across admission/mapping, prepare
  frame tracking fallibly, and retain charges until physical teardown. Never
  refund live heap backing at logical retirement or by reusable ASID lookup.
  Use `PreparingUserBacking` for heap/image provisional backing. It owns the
  frame, reservation and exclusive address-space borrow through fill/mapping
  or rollback. Do not pair a standalone charge with `PreparingUserFrame` or
  commit a charge separately in service/loader code. Failed release consumes
  the original domain ceiling and node pool even after root destruction; an
  unconfirmed published leaf must retain backing without deallocation.
- ELF/runtime frames use the independent `image_account` in `backing_budget`.
  Bound layout validation and aggregate image planning before namespace
  creation; use `PreparingUserBacking` and fallible mapping rather than scalar
  loader helpers or panic-on-failure backing allocation. Charges survive until
  physical teardown, including partial launch preparation failures.
- Runtime address-space creation uses `AddressSpace::try_new_user`; trusted
  mandatory kernel fixtures may use its panic wrapper. Keep a new translation
  root behind `PreparingTable` until its final architecture ownership
  transfer; root allocation failure must precede namespace publication.
  Both walkers use `PreparingTable` for root/intermediate publication. Keep
  fallible work before its consuming `publish`; an interrupted publication must
  never recycle potentially reachable backing. Private/lower-half tables
  preserve the physical progress floor for ordinary domains. Shared higher-half
  kernel tables and trusted platform private tables may consume that reserve;
  derive scope from validated architecture mapping context, never application
  input. Private roots/intermediates reserve the embedded `table_account` before
  allocation. `PreparingTable` keeps its exact borrow through publication/rollback;
  retained empty or partial trees remain charged. Classify trusted platform
  translation admission before initial root allocation, never from artifact
  roles/manifests or a later ASID lookup. Physical teardown refunds tables only
  after a fully successful walk, excluding prior quarantined provisional pages.
  Failed release retains the original domain/node charge; snapshots cannot
  allocate private branches. Fresh shared kernel tables reserve an owning node
  charge before allocation; publication retains it for the kernel lifetime.
  Never charge copied shared links again or refund them on user-root teardown.
  Refund unused reservations, and unpublished backing only after confirmed
  physical release; failure/abandonment retains charges. Inherited boot tables remain outside this
  runtime pool, not retroactively adopted ownership.
  See `docs/reference/translation-admission.md`.
- Dynamic unmap removes leaves, not intermediate-table ownership. Keep empty
  tables linked for reuse until quiescent address-space teardown; table charges
  must follow their actual lifetime, not mapped-leaf counts. Initialize backing
  and complete entry permissions before publishing a valid table/leaf link.
  Returning a leaf frame does not authorize reuse before cross-LP invalidation.
  See `docs/reference/page-table-lifetime.md` for the current policy and gaps.
- Kernel-range rollback/teardown uses `RetiredKernelRange`, supplied outside
  arena/page-table guards. Detach first, release guards, then explicitly release
  backing after invalidation. Its Drop quarantines; it must never rendezvous or
  free unconfirmed backing under an unknown lock. Arm the receipt's terminal
  physical-release phase before allocator calls; only invalidation failure before
  that phase may retry. A physical rejection/interruption must never revisit
  returned addresses or allow receipt reuse. Release at most sixteen 4 KiB frames
  per allocator hold, including large/huge extents; retain whole stack reservations
  on incomplete cleanup. Early-boot metadata is inline and bounded. Never turn
  failed IPI delivery into an acknowledgement. See
  `docs/reference/kernel-frame-retirement.md` for scope and remaining x86 work.
  Memory-object mapping work retains `MappingRetirementPin` through its
  invalidation; the last DMA/copy unpin must not bypass that ownership. Preserve
  installed-prefix records on failed rollback and verify leaf identity before
  detach. Abandoned/failed cleanup retains backing and its original charge.
  Prepare mapping-retirement records under the object registry, then release
  it before walking address-space tables. Preparation consumes its registry
  guard; do not restore a borrowed-guard API that lets temporary guards survive
  through chained detachment calls. Use the pin's fixed-size frame
  batches; do not allocate teardown snapshots or nest registry/table guards.
  Scratch admission prepares live-extent metadata fallibly before publication.
  Release one exact live reservation without allocation; a failed scratch
  completion must not discharge the mapping pin or clear loan restrictions.
  Range checks do not replace generation/reservation ownership. See
  `docs/reference/scratch-admission.md`.
  Public memory-object map/map-any/unmap operations own an
  `AddressSpaceOperation` from before registry access through scratch completion
  and TLB invalidation. Complete it explicitly on ordinary success/error; panic
  or abandonment retains the root. MMIO also claims its capability in-flight
  until invalidation; close returns busy without consuming it. Explicit device
  close leases its live root and detaches authority before releasing lifecycle
  for invalidation. Direct loan revocation owns both namespace leases, the
  existing borrower state and a backing pin through detach, invalidation, scratch
  release and authority removal. Failed or abandoned revocation retains its
  Revoking fence and pin; never restore usable loan authority after uncertain
  cleanup. Borrowed-memory IPC replies, including returned connections or memory,
  compose both root leases, loan receipts and an
  exclusive reply claim in `PreparedReply`. Admit leases before IPC, revalidate
  exact identities and prepare every loan before claiming. Detach/invalidate
  outside IPC; close of either call/reply cap waits outside IPC until completion.
  Roll back only unstarted loan receipts, never uncertain physical cleanup.
  Returned connections retain their exact minting source through the reply claim,
  with caller-sponsored destination authority hidden until publication under IPC.
  Only delivered/observed sources qualify: queued or unobserved-result connections
  remain indirectly reclaimable. Source-close waits outside IPC too. Refund
  unpublished grants before completing leases on ordinary failure. Returned
  memory retains a `PreparedTransfer`: qualify delivery/observation, escrow its
  exact source, and publish it jointly with any returned connection before
  result visibility. Memory close waits outside registry/IPC for source escrow
  and pin completion; serialized cleanup uses `try_retire_cap` without waiting,
  then consumes the backing owner after IPC unlock.
  Restore escrow, release its pin and clear its transfer fence under one memory
  registry hold; never touch a successor's reused ASID/capability. Abandonment
  restores unstarted returned-memory escrow but retains the reply/connection
  source claim, roots and uncertain loan backing; do not force-clear them.
  Explicit pending-call/reply close uses `PreparedCancellation`: admit both roots
  before IPC, prepare every loan, then claim the token with `completing`.
  Queued messages remain in place; receive returns Pending and endpoint close
  waits outside IPC while their cancellation claim is live. Revoke outside IPC
  and record each success before consuming capabilities or notifying waiters.
  Endpoint readiness must exclude claimed/failed queue fronts; after queued
  removal re-signal endpoint waiters/CQ outside IPC so later work is not stranded.
  Failed cleanup must leave the call/reply cap live and publish no terminal
  result; fence the failed token from delivery/reply and restore only unstarted
  receipts. Abandonment retains queue/claim/roots
  and loan pins. `PendingCall::close` returns its owner on error; its Drop/wait
  fallback aborts the domain on rejected close rather than ending an unsafe Rust
  borrow. Explicit endpoint close with queued loans owns a `PreparedEndpointClose`
  claim and exact server-root lease. Fence enqueue/receive/mint/resize while it is
  live; process one queued call at a time with `PreparedCancellation` borrowing
  that server owner and leasing the captured caller. Never acquire a fresh server
  lease after a staged server close, clear the endpoint claim before final
  publication, or allocate a whole-queue teardown snapshot. Complete each loan
  outside IPC/lifecycle; publish each call's result only after its cleanup. Keep
  endpoint close watches/readiness parked until final close, and re-signal readable
  work after ordinary rejection. Abandonment retains the endpoint claim/root and
  any active call's claim/caller root/pins. Whole-domain IPC loan cleanup borrows
  the exact `ClosingAddressSpace`, fences endpoint/record admission, and uses
  bounded per-token cancellation receipts outside lifecycle/IPC. Retain peers
  through closing-owner cleanup leases before claiming, including already-closing
  peers; ordinary operation admission stays fenced. Seal cleanup admission after
  IPC and owned memory drain and zero leases, before root backing teardown. Never admit a
  peer after sealing, acquire lifecycle beneath IPC, consume uncertain receipts,
  or allocate namespace snapshots. Pending returns the closing owner; failure or
  abandonment retains its fence/root and uncertain pins. Serialized namespace
  loan cleanup is confined to raw boot fixtures. Whole-domain memory cleanup
  borrows the closing root and retains mapped peer generations in admitted mapping
  records before moving them out of the registry. A preparing pin keeps mappings
  visible and competing cleanup pending until every peer is retained. Never acquire
  lifecycle under the object registry or allocate root/mapping snapshots. Roll back
  only unstarted admission; physical failure/abandonment retains backing, mapped
  peer leases, scratch and the closing root. Unmapped caps wait for live revocation,
  DMA/copy or transfer fences before borrower state/authority removal. Only exact
  completion receipts permit cleanup sealing and final root teardown. The serialized
  whole-domain memory adapter is confined to raw boot fixtures. Published
  move/copy/result attachment cleanup removes authority under IPC into
  `RetiredMemory`, then releases backing outside IPC. Admit `MemoryAttachments`
  retirement storage before publication; carry bulk queues in their existing
  admitted storage, never allocate a teardown snapshot. Undelivered owners cannot
  acquire application mappings/pins. Retain original charges through physical
  release; Drop quarantines, and rejected release never refunds or retries.
  Frame release holds the physical allocator for at most sixteen frames per
  batch. Copy/vector submission captures both exact roots, prepares outside
  IPC and returns failed `PreparedCall` ownership before dropping copied frames.
  Revalidate captured connection, endpoint and namespace identity at publication.
  General metadata allocation/destruction under IPC remains separate work.
  x86 invalidation uses bounded epoch-fenced acknowledgements, including global
  translations; never enable IRQs beneath an unknown mask or count failed/stale
  deliveries. Scheduled same-LP reapers perform physical stack release outside
  IRQ tails. Retry only a retained retirement owner, never abandoned backing.
  Provisional kernel data frames follow the same rule: `PreparingKernelFrame`
  Drop only retains backing and updates its atomic diagnostic. Success consumes
  it into mapping ownership; ordinary rejection consumes it into
  `RetiredKernelRange`. Never free, invalidate, acquire allocator/table guards or
  log from that destructor, even for apparently unpublished backing. Do not
  re-adopt an abandoned frame by address.
  DMA teardown fences map/unmap before hardware detachment. Require VT-d read/
  write drains, AMD strict completion-store epochs, and SMMU ASID TLBI/SYNC
  before releasing data pins. Preserve unconsumed command queues after timeout.
  A completed domain destroy retains its requester fence until confirmed reset;
  supported QEMU NVMe reset owns config serialization and disabled bus mastering
  through new-domain creation. Reject old MMIO authority before reset; uncertain
  MMIO cleanup retains its claim, root and mapping/scratch record. Explicit
  MMIO close detaches authority but keeps its claimed descriptor reset-visible
  until confirmed completion. Unsupported reset targets remain fenced.
  See `docs/reference/hardware-quiescence.md`.
  Bulk reply-token cleanup must not report failed loan revocation as terminal.
  Whole-domain device cleanup borrows the exact closing root through
  `PreparedNamespaceDevices`. Detach admitted registry storage and remove IRQ
  routes under lifecycle/device serialization; release those guards before
  MMIO invalidation, scratch completion or DMA destruction. Release device
  authority only after confirmed cleanup. Failure/abandonment retains unfinished
  records, authority, scratch and the closing root; no physical cleanup in Drop.
  Require the exact completion receipt before progressing to IPC loan cleanup.
  Loan revocation rejects DMA pins under the memory registry; CPU invalidation
  never substitutes for DMA completion. Backing pins do not lease an ASID; see
  `docs/reference/memory-object-retirement.md`.
  Final user-root close detaches into `RetiredAddressSpace`/`RetiredEntry`,
  leasing the software slot through post-guard invalidation and destruction.
  Never take/drop a published user root under the address-space table guard or
  return its slot before quiescence. Invalidate ARM using the owned hardware
  tag, not a detached numeric-ASID lookup. Abandonment quarantines the complete
  root/accounts/slot; only explicit release completes its lease. This does not
  replace thread quiescence or solve earlier mapping/IPC/device locking. See
  `docs/reference/address-space-retirement.md`.
  `AddressSpaceOperation` retains an exact live generation through explicit
  completion; abandonment retains its count/root. Busy close must reject
  before subsystem mutation. Acquire lifecycle before table/subsystem guards,
  never under IPC/device serialization; release takes only the table. Do not
  replace/drop a leased root through mutable table access. Production split-phase
  use also needs backing/scratch/authority owners and caller busy-close policy;
  do not remove masking guards merely because this foundation exists.
  `ClosingAddressSpace` owns a staged operation-lease admission fence after
  thread quiescence. Pending poll must return that owner; older leases may
  complete but new leases and competing close requests must reject. Dropping
  the request or timing out retains the fence/root, even after the last lease
  finishes. Never clear closing or force-decrement abandoned counts. Poll/wait
  must release their lifecycle/table guards before sleep or final invalidation;
  callers must also avoid unrelated masking guards. This fences lease admission,
  not every legacy resource path or the supervisor's deployment policy.
  See `docs/reference/live-address-space-operations.md`.
  Owning-root physical teardown uses `FrameRelease`: disarm the root and make
  heap/image accounts nonrefundable before release starts. Only a fully
  successful private-tree/data walk permits account refund. Rejected release
  retains the whole charge; never retry partially freed tables or restore a
  quarantined account through a successor ASID. Successful root teardown must
  exclude previously quarantined provisional pages from its refund. Platform
  promotion cannot reclassify an account with quarantined pages. Private tables
  have their own lifetime admission; fresh shared kernel tables retain a separate
  node charge for the kernel lifetime, independent of private-root refunds.
- Kernel `IdTable` publication prepares payload, generation and return-ID
  capacity together before mutation. Use `try_add_element` for runtime roots and
  threads; rejected payloads remain owned and must be destroyed after table and
  lifecycle/scheduler guards leave. `take_element`, close preflights and final
  slot completion must not allocate or repair missing capacity during teardown.
  Missing capacity rejects before extraction/fencing or retains the existing
  closing owner. The panic wrapper is confined to mandatory kernel initialization
  and trusted fixtures.
- Runtime threads prepare an owning retirement-list node fallibly before
  generation claim, stack backing and publication. Staging, detached-batch
  filtering and reinsertion use those nodes and inline LP heads, never growing
  maps/vectors. `ReapBatch` owns its nodes and transition marker through explicit
  completion; abandonment retains both. Release detached nodes only on the
  owner LP after the executing-stack and architecture ownership checks, outside
  thread/staging guards. Self-exit retains its current handle/context through
  the outgoing save and CPU-ownership handshake; mark a requested abort rather
  than removing that handle before switching. Owner-LP abort cleanup scans a
  captured slot ceiling and claims each exact occupant under the table; do not
  restore a teardown snapshot there. See `docs/reference/thread-retirement.md`.
- Whole-domain thread abort retains an exact `AddressSpaceOperation` and closes
  the root's inline thread-admission fence under publication serialization.
  Sweep a captured slot ceiling with captured thread generations, outside that
  gate; never grow an abort map or snapshot numeric TIDs. Fence installation
  and force-request publication require the retained root, not a fresh ASID
  lookup. Drop retains the root lease/fence. User thread publication takes
  lifecycle before its gate and rejects aborting or closing roots through
  publication; stack preparation also rejects the abort fence. Forced deployment
  abort retains a registry claim while releasing its guard before lease admission.
  Failed abort remains failed without force-success counters. Rejected root
  admission publishes no force request.
- Runtime thread stacks use `thread_stack::Stacks` for both user/kernel backing.
  Reserve the full configured user capacity plus sixteen kernel pages before
  allocation; kernel-only threads reserve sixteen pages. Keep the original
  `StackSlot` root lease, bitmap slot and captured classification until both
  ranges are detached, invalidated and physically released. Partial cleanup,
  failed provisional release and interrupted publication retain the whole
  reservation, slot and root, even after ordinary committed pages are freed.
  Growth borrows that owner and revalidates its exact generation; it never
  reconstructs cleanup from ASID or adds backing outside captured capacity.
  Kernel mapping rejection moves unpublished frames into `RetiredKernelRange`
  too; only confirmed post-guard release authorizes a completed rollback result.
  Raw stack allocation is confined to that admitted memory adapter. Inherited
  boot/CPU stacks remain outside runtime admission. See
  `docs/reference/stack-admission.md`.
- IOMMU backing uses `device::dma_tables::Tables` for domain roots/intermediates,
  SMMU descriptors and shared unit tables/queues/completion cells. Reserve actual
  pages before physical allocation, retain an exclusive owner through zeroing
  and fallible ledger preparation, and mark published before hardware authority.
  Failed creation/retirement retains its registered retiring domain/source fence
  unless detachment, hardware maintenance/drain and complete physical release
  succeed. Partial physical release freezes the owner and whole charge; never
  retry freed tables or refund by reusable IDs. Empty branches stay charged and
  cached until retirement. Failed data-leaf prefixes require confirmed hardware
  completion before pin release; prepare quarantine capacity before publication,
  retain rejected pins, and consume teardown collections outside backend guards
  without allocating snapshots. See `docs/reference/iommu-table-admission.md`.
- Kernel scheduler `Observable` sources must implement fallible owned waiter
  registration; there is no weak-only default. Do not invoke callbacks inline
  while the scheduler holds its thread table. Use `ObserverList`/`WaiterSource`
  and invoke detached notifications after subsystem guards are released.
- Observer lists use `ListRef` and `ObserverList::try_new` with independent
  allocation admission. Capture classification from source admission or the
  first waiter/watch sponsor; never infer it from caller-controlled roles or
  re-resolve an ASID under a source lock. Empty tokens and weak-only backing
  retain the original node/ordinary slot through final allocation release.
  Do not restore bare global-allocator list Arcs or an uncharged constructor.
  Entry admission and list allocation admission remain separate. See
  `docs/reference/observer-list-admission.md`.
- Completion records use `CompletionRef`/`CompletionWeak` and
  `ChargedAllocator::try_arc`. Their original admission follows allocation
  lifetime through final weak release, including the private charge holder's
  deallocation. Keep the preparation owner through failed payload construction;
  do not restore field-owned charges or bare global-allocator record Arcs.
  Allocator clones share one allocation allowance; never expose a weak or bare
  Arc to their private charge holder. See
  `docs/reference/completion-record-budgets.md`.
- Anonymous timer admission follows the event, `queue::OwnedNode` and all
  cancellation references through final backing release. Keep the node's
  lifetime owner outside its Box; queue removal or callback completion alone
  cannot refund a retained cancellation allocation. Shared allocator clones
  retain one original reservation; only the cancellation state consumes their
  allocation allowance. Completion timer producer observers must be allocated
  fallibly before callback registration, record/authority publication and enqueue;
  rejection retains no prepared event or submission slot. No uncharged
  cancellable constructor remains. See
  `docs/reference/scheduler-timer-budgets.md`.
- Non-scheduler completion callbacks use `completion::observe` and retain its
  `CompletionObservation` owner. Dropping that owner cancels only the
  subscription, not the operation or its producer. Keep arbitrary callback work
  short and capture exact operation identity rather than re-resolving reusable
  ASID/capability numbers; asynchronous callers with an existing captured object
  use `observe_registered`. The timer's single internal slot is not a general
  callback-registration API.

- `catten-syscall` mirrors the register ABI and intentionally exposes integers.
- `catten-rt::owned` is the safe application layer and owns kernel resources.
- Protocol crates define wire formats and opcodes; they do not own live
  capabilities.
- Service/client helpers wrap protocol lifetimes on top of `catten-rt::owned`;
  they do not duplicate syscall cleanup.
- Drivers may use raw MMIO, DMA, CQ, and device operations only where the typed
  runtime API cannot express the hardware contract. Keep that raw region small
  and expose an owned interface to the rest of the service.
