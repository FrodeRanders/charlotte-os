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
- Scoped deployment uses `launch_scoped_artifact` and retains its
  `DeployedArtifact`. The kernel's signed launch receipt supplies its retirement
  principal; never reconstruct that owner from a caller-supplied name or use
  a zero-principal singleton selector. See `docs/reference/scoped-deployment-launch.md`.
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

## Waiting and readiness

- Prefer registering interest in the exact resource and blocking until its
  readiness transition over repeated `poll`, `yield_lp`, `spin_loop`, or short
  timer sleeps. Yielding keeps a thread runnable; sleeping between polls is
  timer-driven polling, not resource readiness registration.
- Use `PendingCall::wait` or `PendingCall::wait_timeout` for a single accepted
  RPC, and owned completion/endpoint/CQ waits where their producers actually
  signal readiness. Carry one monotonic deadline through the operation;
  retries and unrelated notifications must not restart its budget. Timeout and
  waiter/watchdog admission rejection are distinct outcomes, neither proving
  completion, quiescence, or permission to replay an accepted request.
- Keep the pending owner and every memory borrow through timeout or rejected
  waiter admission. End a borrow only after confirmed completion/cancellation;
  use the existing owned close/Drop policy, including domain abort on rejected
  loan cleanup. Readiness alone does not adopt returned capabilities.
- Kernel waits must register an owned, fallibly admitted subscription against
  captured identity/generation, recheck readiness to close the lost-wakeup
  window, release subsystem/lifecycle guards before parking, and cancel unused
  source/watchdog registrations on wake. Notify after releasing producer guards.
  Preserve the enclosing IRQ policy; never enable interrupts beneath an unknown
  mask merely to permit blocking.
- Kernel and syscall timed completion waits share the checked scheduler's
  absolute-deadline loop. Kernel `WaitAdmissionFailed` and ABI
  `WAIT_ADMISSION_FAILED` retain the pending completion/buffer; neither is a
  timeout or a terminal result. Prepare watchdog/observer storage before the
  wait's local park-publication mask, with the captured generation/sponsor;
  full watchdog admission rejects before observer allocation. Recompute the
  remaining budget after preparation. Preserve any caller-owned mask instead
  of force-enabling IRQs. An unrelated wake must recheck and repark with the
  remaining budget. Do not duplicate a one-wake syscall adapter or turn
  admission rejection into `false`. Exact-generation exit/retirement snapshots
  diagnose waits; absence, a worker-return marker or a wake is not quiescence
  and cannot authorize root/stack/backing release.
  Kernel timer sponsors are address-space-generation accounts shared by that
  root's threads, not private per-thread counters. Pressure fixtures needing
  isolated accounting must prepare a private fixture sponsor before thread
  publication or any event capture, retaining trusted classification and normal
  node admission. Never replace a live sponsor or saturate the shared kernel
  account to claim exact per-thread cleanup; local IRQ masking does not exclude
  another LP's reservations or reclamation.
- IPC close waits retain the exact root and subscribe to the active
  reply/cancellation or endpoint-close claim, including its checked epoch.
  A rollback followed by a new claim on the same token is a different episode.
  Prepare charged source/disposal storage outside IPC; revalidate root,
  capability, token/endpoint and epoch before relinking. Claim completion,
  ordinary rejection and failure close that source through producer-LP deferred
  delivery. Registration enters only its independent list under the scheduler.
  Owned endpoint close borrows its existing server lease for a claimed front;
  retain its transient interest with that owner. Wait admission rejection leaves
  authority and uncertain loans intact. Abandoned claims remain fenced and do
  not generate readiness or permit physical retry. Serialized namespace/fixture
  close returns Pending on an unexpected claim; it must not spin or admit a
  fresh live root beneath lifecycle.
- Scheduled DMA grant preparation may wait on the existing fixed IOMMU unit
  slot while retaining its original `PreparedDmaDomain` root and reservation.
  Admit charged source/disposal storage outside backend guards, using the exact
  captured root classification. Register only against the independent list;
  recheck and claim the unit after wake. An available command engine with a
  detached domain cell is not creation readiness. Restore notifications and
  list disposal use producer-LP deferred delivery after local guards leave.
  Reject masked/nonscheduled waiting and waiter admission failure explicitly;
  serialized/reentrant fixtures use the rejecting grant path. Work runs once
  after admission: never reenter creation with an armed owner or recover an
  abandoned unit claim. Consuming grant cancellation may borrow that same owner
  to wait only before its first destruction claim/descriptor detach. Keep the
  rollback-started fence through admission failure, wake and one destruction;
  rejection/interruption retains the complete grant and cannot be retried.
  Revalidate its recorded domain and published, unfrozen backing under the same
  backend hold as extraction; absence is not completion. Shared admission runs
  its claim callback once and returns a retaining owner before guard exit.
  Source and transient-interest fallback retain their allocations; readiness
  grants no hardware or physical cleanup authority. Serialized fixture adapters
  retain rejecting admission; public DMA operations remain separate contexts.
- DMA unmap distinguishes backend admission rejection from completion failure.
  Only `OPERATION_IN_FLIGHT` from before actual domain/engine claim permits
  another unmap submission. Keep typed admission/completion errors through the
  kernel adapter and device status ABI; a later busy-looking error is terminal.
  `DmaTransfer` and `SharedDmaMemory` record the attempt before kernel entry,
  retain memory on ambiguous failure, and do not resubmit from `finish` or Drop.
  Confirmed DMA completion survives later CPU-unmap failure. Best-effort local
  teardown does not discharge an uncertain kernel DMA pin or its original
  backing charge. Add readiness only with a real producer contract; do not
  replace this distinction with an unmap polling/retry loop.
- DMA map/map-exclusive return per-call status, IOVA and diagnostic stage/cause.
  Preserve backend rejection through the exact public root/capability owner;
  never replace it with a zero address or global last-error state in diagnosis.
  Backend diagnostics include pre-claim admission and started maintenance, so
  neither a busy cause nor an error proves rollback or authorizes map replay.
  A failed shared-DMA preparation must retain its existing CPU mapping behind
  `FailedSharedDmaMemory`, without slices or pointers. Explicit CPU unmap and
  best-effort local Drop do not discharge uncertain kernel pins/backing charges.
  Public readiness still needs an owned operation and qualified caller IRQ
  context; never add polling or enable IRQs beneath an unknown mask.
  Map/map-exclusive syscall admission now borrows the same `DmaOperation`
  through the existing fixed unit source. Retain its original pin and captured
  root classification before source preparation; claim domain/engine/pin
  together under the backend hold, then run mapping once. Busy masked or
  nonscheduled callers reject; serialized kernel fixtures keep rejecting
  admission. Ordinary unused preparation releases pin/interest outside guards
  before completing the root. Abandonment retains all fields and the public
  claim; readiness gives no cleanup or replay authority. See
  `docs/reference/dma-map-readiness.md`.
- Services processing several sources must wait on all relevant interests.
  Use `PendingCall::bind_cq` with endpoint interests on one CQ and
  `cq_wait_checked` for the earliest remaining operation deadline (or an
  event-only idle wait). Its owned kernel watchdog requires no extra completion
  capability. Keep rejected wait admission distinct from timeout; a reply wake
  does not observe its returned resources.
  Kernel lifecycle request publication wakes the default CQ after exact-root
  page access and local guard exit. Recheck lifecycle on those wakes.
  Do not block on one reply if that service must receive/process other work to
  produce it. Do not replace reply polling with a CQ wait unless reply readiness
  is wired to that CQ. Add the missing notification contract rather than an
  unrelated wake source or a new per-instance registry.
- Bounded hardware status polling, short low-level lock acquisition, deliberate
  fault fixtures, and admission/protocol backoff without a notification surface
  need a concrete reason and bound or terminal policy. These are not a blanket
  exemption for routine resource waits. Record missing readiness notifications
  as audit gaps; do not claim a polling path is event-driven. Cleanup waits must
  retain their complete existing owners and update the applicable row in
  `docs/reference/cleanup-recovery.md`.

See `docs/reports/audits/2026-10-12-wait-readiness-sweep.md` for the weekly sweep,
the corrected RPC polling paths, and the remaining notification gaps.

## Audit evidence

- Keep published audit reports self-contained. Record concise results, artifact
  hashes and relevant failure diagnostics inline, or link version-controlled
  evidence. Do not cite temporary logs or ignored build artifacts as evidence.
- Prioritize unresolved and intermittent failures; omit routine boot/build
  output. Preserve failed runs separately from passing repeats and distinguish
  observations from causal conclusions. If original captures are unavailable,
  retain the historical result with an explicit verification limitation.

## Architectural boundaries

- Cleanup/recovery changes must update their owner-family row and evidence in
  `docs/reference/cleanup-recovery.md`. Preserve the containing transaction's
  exact roots, pins, scratch, authority and original charges. Retained backing
  or an error code is not a retry owner. Classify unstarted rollback, Pending,
  owner-preserving retry and terminal retention separately. Qualify the outer
  IRQ/guard context and implicit field destructors before adding custody for a
  category; share custody/policy machinery without erasing typed completion
  proofs. New object instances do not justify a new registry or coverage row.
- Kernel admission policy is published once through
  `service::admission::PreparedBootTrust`. Cluster/deployment manifests borrow
  its `BootTrust` view; never configure their policy independently of the kernel
  gate or overwrite an installed boot policy. Signed preparation consumes a
  zeroizing recipient-key owner and checks its public binding. It does not
  authenticate the platform, install rollback-resistant state or provide
  custody; production remains disabled until those boundaries are implemented.
  `boot_trust_test` is an x86 QEMU fixture with public test keys/state, never
  production enrollment. Boot consumes exact installed policy state; the shared
  verifier's direct-successor acceptance is for installation and must not permit
  publication before state commit. Keep substituted boot inputs rejected before
  kernel entry or trust publication, with explicit execution evidence.
- Boot hardware composition prepares all storage/entropy/network driver grants
  before starting any sibling driver thread. Keep every boot hardware-driver
  image in the signed bootstrap set; preparation cannot wait on the object
  store served by its own unstarted storage driver. Use `PreparedDriver`
  owner to retain the loaded root and exact operation lease through preparation
  and thread publication; its fallback retains all fields without cleanup.
  Dynamic creation still rejects busy unit/engine admission. Preserve DMA grant
  errors as per-return stage/cause and confirmed-versus-retained cancellation
  values; diagnostics are not custody or authority to retry uncertain creation.
- Deployment placement must use signed executable variants and kernel-reported
  execution profiles. Filter architecture, exact runtime ABI and required CPU
  features before capacity/affinity ranking, including explicit pins. Unknown
  profiles reject. Commit each replica's selected variant index; agents may not
  reselect from later reports. Recheck the actual ELF and signed target at kernel
  launch. CPU feature availability is the intersection across schedulable LPs;
  do not advertise extensions without the context-save contract. Deployment
  state retains generation, the signed descriptor and exact replica pins;
  derive requested identity/placement and variant keys/digests from that
  descriptor. Admission must not accept independent name/object/node/digest
  overrides. Keep query, snapshot, relay and durable log tags coherent when
  changing the grammar; reject retired log-only stores before replay too. See
  `docs/reference/execution-targets.md` and `docs/reference/catalog-snapshots.md`.
- Internal APIs and wire formats have no backward-compatibility requirement.
  Remove compatibility-only branches when a coherent replacement is ready;
  do not preserve an unsafe allocation path for older callers.
  Protocol simplification covers magic/version tags, parsers, writers, SDKs,
  fixtures and both Markdown/LaTeX living documentation together. Prefer one
  current grammar and reject retired formats even when correctly signed; do
  not fill absent fields with compatibility defaults. Remove old paths and
  compile their callers before a separate optional magic/version rename. Never
  reuse a retired magic while its old grammar remains accepted. Current format
  numbers may stay unchanged; resetting everything to version 1 is not required.
  Record batches and remaining candidates in `docs/reference/api-simplification.md`.
  Tooling uses file-only signing-key arguments and repeatable
  `--cluster-service [NAME=]VIP:PORT` assignments. Kernel builds consume only
  `CATTEN_CLUSTER_SERVICES`; retired singleton variables reject explicitly.
  See `docs/reference/tooling-inputs.md`.
  Preserve historical reports; hardware support and external protocol negotiation
  are not internal backward-compatibility obligations.
  Persistent records must carry explicit format tags and distinguish absent
  storage from unsupported, corrupt or incomplete state. Once stores are
  deployed, format/layout changes must include an explicit version-directed,
  interruption-tested migration preserving election and committed state;
  never silently reset or reformat an existing store. Current pre-production
  simplification does not require retaining older decoders or migration code.
  Object-store v4 requires explicit nonzero owners, fixed headers and current
  directory records. Only two successfully read zero-filled superblock slots
  permit first-boot formatting; unsupported, unrecoverable or incompatible
  metadata rejects before mount writes. Host staging uses the kernel principal.
  See `docs/reference/object-store-format.md`.
  Catalog registration uses one record/encoder with an explicit deployment
  generation, including zero for system services. Missing suffixes reject;
  catalog-v18 and Raft pair-v3 reject earlier stores before replay. S3 launches
  require the complete immutable CHS3PF1 profile in read-only launch memory;
  per-field manifest aliases and defaults supply no fallback. See
  `docs/reference/catalog-snapshots.md` and `docs/reference/s3-client.md`.
  Kafka uses the fixed CHKAFP7 profile and one explicit authentication choice.
  Copy borrowed credential fields directly into the existing zeroizing launch
  buffer; reject unknown choices, disabled-field material and retired section
  formats. Broker protocol negotiation remains a separate external boundary.
  See `docs/reference/kafka-client.md`.
  DNS catalog observations use complete bounded CDNSCAT1 records. Reject
  oversized sets, inconsistent counts/lengths and retired untagged pages;
  never shorten identities or decode a clamped prefix. HTTP preserves replica
  rows and reports unavailable catalogs as null. Consume the raw request once
  into an owned reply transaction before scalar fallback guards.
  See `docs/reference/dns-catalog.md`.
  Correlated DNS queries use 0x24/0x25 and exact u32 identity lengths. Prepare
  bounded request bytes before publishing pending state, carrying its original
  deadline; return preparation rejection without a pending query. Preserve
  expected peer/leader, session, query ID and generation checks. Rejected
  reply preparation sends no prefix or synthetic authority and permits no
  replay. See `docs/reference/dns-query-relay.md`.
  DNS invocation uses 0x26/0x27 with full bounded identities and exact frames.
  Prepare request bytes before in-flight publication with the original
  deadline, and retain a prepared reply buffer in the existing local operation
  before admission. Ordinary rejection returns that destination owner; result
  encoding consumes the same buffer. DNS v4 client invocation requires the
  exact CDNSCL01 moved-memory frame and full 1..=256-byte name; retired packed
  calls reject. Local invocation retains `LocalInvocation` with owned pending
  calls/connections, binds both RPC phases to the reactor CQ and keeps typed
  reply authority in its destination. Capture one kernel-monotonic deadline
  before decoding/routing, wait for the earliest remaining operation deadline,
  and treat binding rejection after accepted invocation as uncertain, never
  replayable. NS interface 2 generation observation/retirement requires the
  exact CNSGEN01 memory record, complete name and explicit zero/positive
  generation. Retired packed forms reject. Preserve exact registrar/manager
  authorization from the kernel identity, and keep checked observation rejection
  distinct from absence before catalog mutation. Queue local cleanup through
  the owned `Unregistration`, reserve storage before submission, bind its reply
  to the CQ and retain its original deadline. Failed admission has no recovery
  owner: follow the fail-closed policy with the containing transaction live;
  rejected binding retains accepted work through abort, never timer polling or
  replay. NS connection close is best-effort logical teardown, never physical
  proof. A lookup of a third-party publication grants no retirement authority;
  NS bearer-key opcodes 5/6 are retired; do not restore key storage, exported
  entry points or a missing/malformed-key-to-public fallback. Unsupported
  requests adopt their complete incoming transaction once before rejection.
  Public bootstrap naming remains separate from application policy: immediate
  and deferred public lookup cannot grant a policy-published service, and a
  public registration cannot displace it. Security fixtures use CNSGEN01 for
  metadata, retaining explicit rejection controls for earlier shapes.
  NS registrations retain an owned connection and persistent endpoint-death
  completion together. Admit the watch before policy/registry mutation and
  cancel it before source disposal. NS waits event-only on its request endpoint
  and those default-CQ completions, rechecking lifecycle. A CQ entry is a hint:
  confirm the current owner's exact endpoint-close result before unpublishing
  its captured full name/generation. Unexpected watch/wait outcomes retain the
  registry through terminal abort, never replay or fabricated death. Trusted
  NS construction prepares its bounded 64-publication plus replacement watch
  budget before thread publication; never resize a live sponsor or derive
  platform classification from a manifest/role/name. Share
  the existing publication cleanup claim across Raft commit and the scan; never
  accept two retirements for one captured episode. Raw watch/receive/transport and other packed APIs remain gaps. See
  `docs/reference/name-service-generations.md`. Keep dedup ACK
  fencing and uncertain timeout semantics. Carry the caller's query deadline
  into invocation; reject an expired routing result before admission instead
  of restarting its budget. See `docs/reference/dns-invocation-relay.md`.
  DNS registration/retraction use only `0x28/0x29/0x2a`, complete bounded u32
  identities and explicit deployment generations. Bind registration replies to
  the captured leader, session and nonreused request ID, never name alone.
  Prepare requests, pending storage and terminal reply bytes before publication
  or Raft mutation. Keep registration connections/reply authority in the existing
  owning phases; full-name publication must not fall back to a packed prefix.
  Retraction still matches the exact owner and nonzero generation; preparation
  rejection retains the publication and grants no unconditional removal or replay.
  Raw watch/reactor contexts remain separate qualification work. See
  `docs/reference/dns-registration-relay.md`.
  Application snapshot restore must return rejection without changing live
  state. Propagate that rejection before exposing a Raft node or publishing
  snapshot/log progress, membership or join completion; unsupported state is
  never an empty-catalog fallback. See `docs/reference/catalog-snapshots.md`.
- New kernel capability publication uses `capability::Reservation`: reserve
  before payload/ownership mutation and publish under the subsystem's
  serialization. Retain the captured namespace identity, not just its ASID.
  `reserve_captured` requires that registry's captured generation and guard;
  do not acquire lifecycle under a subsystem guard. A source move or loan needs an
  owning payload transaction as well as capability escrow; use `PreparedTransfer`
  and `commit_transfers` rather than reconstructing scalar cleanup. There is
  no unbounded allocator or retirement bypass for any of the six kinds.
  Unified namespace storage uses shared `retirement_list::AdmittedMap` nodes.
  `PreparingNamespace` owns its node and original budget account before user
  ASID publication. User registration prepares outside lifecycle/table guards
  and cancels ordinary unused storage explicitly after they leave. Drop retains
  both allocations.
  Namespace teardown detaches its complete node before releasing entries/account
  outside `CAPABILITIES`. Individual authority records use the same admitted
  nodes: `PreparingRecord` owns fallible storage and provisional charge before
  serial mutation; unused preparation finishes explicitly after capability
  unlock. `RetiredRecord` owns a detached node and its original charge until
  explicit release; abandonment retains both without locks or allocation.
  Batch moves retain this owner in the exact `SourceEscrow` through payload
  completion; `PreparedTransfer` disposes it after leaving the memory registry.
  Containing lifecycle/subsystem guards and active reservation/escrow Drop
  contexts remain separate qualification work.
  The scalar restoration API is removed. See
  `docs/reference/capability-admission.md` for the current enforcement scope.
  Mailbox publication uses `PreparingMailbox` to retain the exact root,
  namespace/endpoint nodes, budget and unified authority preparation before
  lifecycle/mailbox guards. Revalidate captured generation, retirement and
  closing state before allocation-free publication. Existing receiver reuse
  needs no fresh storage. Explicit close carries payload and authority in
  `RetiredMailbox` until post-guard release, completing the root last. Both
  fallbacks retain every field without cleanup; neither is a retry owner.
  Final close detaches mailbox payload/queue backing and the unified authority
  namespace into the existing `ClosingAddressSpace` before root extraction,
  then transfers all of them into `RetiredAddressSpace`. Failed final invalidation
  retains original charges and metadata in that complete receipt. Release them
  explicitly after confirmed invalidation and local guard exit, before root
  backing/slot completion; abandonment retains every field. Root recovery may
  retain only this complete owner, with no separate metadata ticket or replay.
  Word-queue operations retain their captured root before registry access.
  Prepare admitted namespace nodes and queue backing outside lifecycle/queue
  guards; revalidate generation, sponsorship and closing before relinking.
  Established queues use shared registry borrows without fresh storage. Carry
  losing preparation through post-guard disposal, root last. Final teardown
  retains the complete queue node in the same root receipt; Drop retains all
  fields. Legacy and capability word callers share that exact mailbox family
  account; prepare any missing admitted account namespace outside guards and
  relink after captured validation. Reserve queue sets/requested ring bytes
  before fallible backing allocation, with captured platform classification.
  Retain the charge through actual ring deallocation, failed invalidation and
  terminal abandonment. Fixed word rings use short IRQ-state-preserving LP
  mutex holds, released before IPI notification. Namespace/account metadata,
  principal/aggregate heap admission, completion teardown and other final
  bookkeeping remain separate contexts. Scalar mailbox teardown is a
  serialized boot-fixture adapter, never a production close path.
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
  hardware creation and retain a `PreparedDmaDomain` until publication. All
  MMIO/IRQ/DMA grants share `GrantAdmission`: capture the exact root and prepare
  device namespace/payload nodes and unified `PreparedReservation` storage
  fallibly before local publication guards or hardware. Prepare against the
  captured root, then revalidate generation, closing and original memory-budget
  admission under lifecycle before charging/minting. Keep their admitted storage
  and reservation inside that owner;
  publication only relinks prepared retirement-list nodes under `DEVICES`.
  Dispose unused storage/reservation metadata explicitly after guards leave.
  Detach successful close/namespace nodes into `RetiredEntry` owners and release
  outside lifecycle/device guards; abandonment retains the nodes without field
  deallocation. Device close also detaches unified authority into `RetiredRecord`:
  non-DMA `PreparedClose` retains it with exact root/descriptor/payload through
  invalidation and scratch completion; DMA carries it in its existing operation
  owner after confirmed backend cleanup. Dispose both metadata owners before
  completing the root, outside local lifecycle/device guards. Failure or
  abandonment retains the original charge and complete claim; never finish an
  uncertain close or retry a started one. Broader IOMMU ledger/heap metadata and
  other unified authority callers remain separate boundaries. Release
  lifecycle after exact root/reservation admission; revalidate the captured root
  generation and closing state under lifecycle before capability publication. DMA
  publication preparation owns the exact root operation and capability
  reservation together with the backend rollback obligation. Borrow capability
  publication; dispose reservation metadata and explicitly roll back only after
  local lifecycle/device guards leave. Failed rollback/abandonment retains the
  entire preparation without hardware, registry, allocator or logging work in
  Drop. Never retry this publication owner after rollback starts. Backend
  creation borrows its `DmaCreation` and records the admitted domain before any
  reachable descriptor publication; errors retain that obligation. Reject an
  armed creation owner before initialization/reset/allocation. Published-domain
  rejection only marks retiring and publishes abort under exclusive ownership; use
  the enclosing preparation's post-guard destruction for maintenance/physical
  rollback. Creation owns the complete installed unit in its existing claimed
  slot through reset, construction and initial configuration, outside backend
  serialization. Failed hardware rollback must quarantine reachable backing,
  never recycle it.
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
  Joint heap/image preparation Drop only retains captured backing and charges;
  no allocator/pool/table guard or logger may be entered there. Ordinary
  tracking/allocation rejection refunds explicitly, and confirmed mapper rejection
  explicitly rolls back. Use consuming `cancel_unpublished` for ordinary unused
  preparation and handle its physical error. Even abandonment before allocation
  retains its original reservation; neither root teardown nor successor reuse
  refunds it. This does not qualify stack destructors or move ordinary physical
  rollback outside the borrowed table guard. Raw `PreparingUserFrame` Drop retains backing;
  explicit consuming release needs unpublished backing or confirmed quiescence,
  never address-based re-adoption.
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
  Table preparation Drop only retains its exact account/frame; shared charge
  fallback records quarantine atomically without acquiring its pool. Ordinary
  unused preparation uses consuming `cancel_unpublished` and handles rejection;
  allocation failure explicitly refunds unused admission. Arm hardware-tag
  rejection explicitly cancels its prepared lazy root. Reservation-only
  abandonment retains admission through root destruction. No table/frame
  fallback may free, invoke callbacks, take allocator/table/pool guards or log.
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
  Explicit memory-close waits lease the transfer's captured source root and
  retain an owned interest in that exact fence. Prepare its charged source and
  disposal node outside memory/lifecycle/IPC guards, then revalidate before
  linking. Completion restores/consumes escrow, releases its pin and clears the
  fence before closing the interest. Queue admitted notifications and source
  metadata for producer-LP post-guard dispatch; another LP must not consume them
  while the producer holds IPC. Wait admission failure returns ResourceLimit
  without consuming authority or proving completion. Ordinary loan/DMA/copy/
  mapping pins still return LendingActive; their drain notification remains
  separate. No physical owner, retry authority or root completion enters the
  deferred waiter transport. Close/source fallback retains all fields.
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
  supported QEMU NVMe reset owns the exact endpoint's logical config/BAR/ECAM
  claim through new-domain creation and capability publication. Short config
  holds and device serialization leave before reset polling; exact grant and
  complete-unit ownership also release lifecycle/backend guards. Preserve any
  enclosing IRQ policy. Keep bus mastering disabled until explicit
  post-publication activation of the exact busy DMA capability. Reject ordinary
  config/MSI lookup and overlapping MMIO grant/map/close while claimed. Retain
  reset in the containing `DmaCreation` before writes; uncertainty or abandonment
  retains its original grant/root/reservation. Only unstarted or confirmed-reset
  cancellation may clear the claim, after confirmed backend cleanup. Never write
  hardware, clear claims or reacquire config from Drop. Reject old MMIO authority before reset; uncertain
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
  The detached namespace node owns its original ordered capability list; never
  copy it into a teardown snapshot or free its backing below registry guards.
  Require the exact completion receipt before progressing to IPC loan cleanup.
  Loan revocation rejects DMA pins under the memory registry; CPU invalidation
  never substitutes for DMA completion. Backing pins do not lease an ASID; see
  `docs/reference/memory-object-retirement.md`.
  Cooperative supervisor drain/status access uses `bootstrap::with_service_pages`.
  Admit the exact root outside deployment/coordinator guards and validate both
  captured runtime frames against its fixed image mappings before access. Borrow
  the existing abort-sweep lease for force publication; never reacquire through a
  staged close or write a physical frame from a copied service handle. Cache
  rejected access without advancing acknowledgement, authority or device gating.
  Complete ordinary access explicitly; abandonment retains root/image backing.
  Final user-root close detaches into `RetiredAddressSpace`/`RetiredEntry`,
  leasing the software slot through post-guard invalidation and destruction.
  Never take/drop a published user root under the address-space table guard or
  return its slot before quiescence. Invalidate ARM using the owned hardware
  tag, not a detached numeric-ASID lookup. Abandonment quarantines the complete
  root/accounts/slot; only explicit release completes its lease. This does not
  replace thread quiescence or solve earlier mapping/IPC/device locking. See
  `docs/reference/address-space-retirement.md`.
  Failed final-root invalidation may transfer only its complete detached owner
  into `retirement::recovery`'s fixed registry. Tickets identify records, never
  authority. Claim outside registry serialization; retry only invalidation before
  physical release. Masked production retry rejects before claiming. Complete
  physical teardown once through `release_value_with`; rejected/partially released
  backing and abandoned attempts are terminal, never restored or retried. Drop
  marks its stable admitted cell atomically and quarantines without locks or
  physical cleanup. Exhaustion returns the original owner; no unbounded spill.
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
  lookup. Admit the exact `AbortExecutor` before root acquisition. Pending
  mutual/remote requests must defer retirement while this owner is live;
  scheduling/wake/reaping share `abort_ready`, and migration/placement retain its
  captured LP. Ordinary rejection completes root then executor. Drop retains
  both fences/root/stack; never clear abandoned ownership. Finish peers before
  the exact local self-request, root completion and executor release share a
  short IRQ-state-preserving mask; no scan, yield, IPI or physical cleanup may
  enter that handoff. Terminal self-abort must not return into an unchecked
  continuation while retirement remains deferred. This is scoped to abort
  sweeps, not arbitrary retained kernel operations.
  User thread publication takes lifecycle before its gate and rejects aborting
  or closing roots through
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
  Provisional initial/growth stack Drop and implicit `StackSlot` destruction
  retain backing, slot, exact root lease and original reservation without
  allocator/table/pool guards, callbacks or logging, even before allocation.
  Ordinary unused slot/page preparation requires consuming `cancel_unpublished`;
  constructor allocation or confirmed mapping rejection cancels explicitly.
  Growth abandonment marks its borrowed parent uncertain; only explicit
  successful rollback may clear the fence it just armed. Ordinary growth
  allocation rejection leaves existing backing usable. Published `Stacks` and
  thread/context retirement still have separate cleanup/context requirements.
  Raw stack allocation is confined to that admitted memory adapter. Inherited
  boot/CPU stacks remain outside runtime admission. See
  `docs/reference/stack-admission.md`.
  Published stack-pair release is explicit and one-shot; field destruction only
  retains backing/admission. Both architectures use pinned IRQ-enabled reapers,
  never physical cleanup in IRQ tails. Failed pairs stay in their existing
  retirement nodes and subsequent scans must not retry. Context storage is
  admitted before generation/backing; ordinary publication/submission rejection
  releases never-admitted stacks after local serialization leaves. Preserve
  outer masks. `Thread` fallback retains its complete `ManuallyDrop` payload,
  including context, waiter state, exit subscriptions, sponsors, retirement node
  and exact stack/root admission; it only records an atomic diagnostic. Never
  invoke callbacks, cancel waits, refund usage or destroy metadata from that
  fallback. Explicit consuming metadata disposal requires confirmed release of
  the exact stack pair and runs after local guards, before retirement-node
  destruction. Outer constructor/syscall masks and growth guards still require
  qualification; this is terminal retention, never a physical retry owner.
  Per-LP reapers park on an owned retirement-head waiter, with a durable
  post-registration readiness recheck. Staging detaches admitted waiters into
  the shared producer-LP deferred transport without callbacks or allocation.
  Only that producer LP dispatches after guards leave, preserving IRQ state;
  idle/yield and post-switch boundaries deliver the wake. Started/failed pairs
  are not productive readiness and must not
  cause timer polling or physical retry. The 10 ms reaper backoff is confined
  to recorded waiter-admission rejection, never ordinary idle waiting.
  A guard-free wait must flush its LP's pending deferred delivery before
  publishing Blocked, under the local park-publication mask. Restoring IRQs
  can switch the producer away before its subsequent yield; that yield must
  not be the only path to a dependent worker's wake. Preserve the caller's
  mask and recompute a timed wait's remaining absolute budget after delivery.
  This flush is metadata/readiness delivery, never physical cleanup authority.
- IOMMU backing uses `device::dma_tables::Tables` for domain roots/intermediates,
  SMMU descriptors and shared unit tables/queues/completion cells. Reserve actual
  pages before physical allocation, retain an exclusive owner through zeroing
  and fallible ledger preparation, and mark published before hardware authority.
  Failed creation/retirement retains its admitted domain slot/source fence
  unless detachment, hardware maintenance/drain and complete physical release
  succeed. Partial physical release freezes the owner and whole charge; never
  retry freed tables or refund by reusable IDs. Empty branches stay charged and
  cached until retirement. Failed data-leaf prefixes require confirmed hardware
  completion before pin release; prepare owning mapping storage before publication,
  retain rejected pins, and consume teardown collections outside backend guards
  without allocating snapshots. Table/region fallback retains original backing,
  charges and ledger storage without entering physical/heap allocators, pools,
  guards or logging, including reservation-only abandonment. Cancel known-private
  fixture construction explicitly with `cancel_unpublished` /
  `prepare_unpublished`. Production domain constructor errors return complete
  typed private payloads into `DmaCreation`, never physical cancellation or
  metadata destruction under backend guards. Keep the exact grant root and
  authority reservation through post-guard `PrivateDomain` cancellation, including
  VT-d context-table rejection. Freeze before physical release; failure/abandonment
  retains the complete private grant and never permits partial-release retry.
  Explicit domain destruction publishes the rejecting descriptor and moves the
  complete domain plus actual command engine into `Maintenance` before unlocked
  hardware waits. Empty engine admission fences ordinary backend mutation/reset.
  Keep the original domain cell empty and requester nonzero. Hardware rejection
  restores exact engine state and retiring domain together without allocation;
  abandonment retains both and permanently fences the unit. Never reconstruct,
  rewind or auto-restore queue/epoch state. Confirm maintenance before returning
  the engine and proceeding to unlocked physical release in `DetachedDomain`.
  Physical finalization uses the registered-state hold even while another domain
  owns the engine. Competing destroy rejects a claimed cell, never treats it as
  completed. Physical rejection restores the exact frozen owner without allocation;
  abandonment retains every field and the claim. Boot unit initialization
  claims its existing typed `UnitState` slot before unlocked preparation,
  hardware control, waits and private cancellation; ordinary DMA lookup must
  never lazily initialize. Preserve the complete unit behind `DetachedDomain`
  before allocation. Clear only a confirmed unstarted/private rollback claim.
  Started control uncertainty retains the whole unit and claim even before
  backing publication; published failure, physical rejection and abandonment
  also never replay initialization. Install only published backing once. Keep
  the boot caller's outer IRQ policy; unit shutdown/recovery remains absent.
  Public DMA map/unmap/close owns an exact `DmaOperation` root/capability claim
  through completion. Explicit close keeps its original payload cell claimed
  through backend destruction. Ordinary rejection clears only that claim after
  the backend retains/restores its complete owner; never extract/reinsert a DMA
  capability record on failure. Consume authority/payload only after confirmed
  backend success; abandonment retains the root and claim without cleanup.
  Confirmed-success device payload nodes now detach into owning receipts for
  explicit post-guard destruction; unified capability-table metadata remains
  separate. Move the complete domain, actual engine and detached pending pin
  into `MappingMaintenance` before unlocked walking/maintenance. Keep its admitted
  cell empty and requester nonzero; restore exact domain/engine together before
  post-guard unpin. Rejected unmap quarantines its pin in admitted storage, never
  allocates a reinsertion node. Abandonment retains all fields and public claims
  without cleanup. Domain creation moves the complete installed unit into a
  retaining owner and leaves its existing slot `Claimed`. Require an available
  actual command engine and no detached domain cells before extraction; a domain
  in physical finalization must retain access to its original registered state.
  Reset, construction and initial configuration run outside local lifecycle/backend
  guards. Restore the exact complete unit on ordinary success/error before grant
  rollback; abandonment retains every field and permanently fences its slot.
  Backend domain cells, requester fences and VT-d context-table metadata use the
  shared admitted map. Prepare their typed nodes inside the existing `DmaCreation`
  before reset, ID mutation or domain backing; partial preparation stays with the
  complete grant. Publish context metadata before its hardware link, and reuse
  an exact zero requester fence in place after confirmed reset. Keep the empty
  domain cell and nonzero source through physical tables and all data unpins;
  detach only then and dispose its node after backend unlock. Explicitly finish
  unused nodes only after confirmed grant rollback or publication and guard
  release; Drop retains them. Per-domain mappings use shared admitted records:
  `PendingPin` owns pin and unused/detached node through the containing maintenance.
  Prepare before data leaves; failed prefix consumes that node and failed unmap
  relinks its original node into quarantine without allocation. Restore exact
  backend state before confirmed post-guard unpin and metadata release. SMMU
  cache misses prepare metadata before branch allocation/linking; partial walks
  retain unused storage in that same domain for later cache admission. Dispose
  cached/unused nodes only after confirmed private cancellation or domain-table
  release, never by field Drop. Metadata does not prove hardware completion or
  authorize physical retry. Table ledgers, general heap/principal admission and
  wider contexts remain separate.
  Intel cannot overwrite
  a busy invalidation register; AMD and SMMU retain their existing exact completion
  and unconsumed queue contracts.
  See `docs/reference/iommu-table-admission.md`.
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

- CQ wait has one checked ABI (88): explicit timeout milliseconds, queue and
  timed flag, with readiness/deadline/admission rejection kept distinct. Syscalls
  12/42 and their SDK/kernel adapters are retired. Use `catten_rt::cq::wait`;
  ordinary service reactors use the shared fail-closed policy, while specialized
  admission recovery must retain original pending owners and deadlines. A wait
  rejection is neither a clock tick nor permission to replay accepted work.
  See `docs/reference/cq-wait.md`. Runtime ABI 3 and all signed variants match
  this register contract, including the sibling Sitas dependency.
- All IPC receive forms use the twelve-register authenticated envelope (71--73).
  Use `Endpoint::receive`/`try_receive` and authorize from the captured sender
  generation/principal/roles, never a later lookup of the recyclable ASID. Zero
  principal or missing roles grant no authority. Register-contract changes bump
  `charlotte_launch::execution_target::RUNTIME_ABI` and rebuild/sign every variant,
  including runner fixtures. Retired nine-register syscalls 22/27/53 reject.
  See `docs/reference/ipc-receive.md`.
- `catten-syscall` mirrors the register ABI and intentionally exposes integers.
- `catten-rt::owned` is the safe application layer and owns kernel resources.
- Protocol crates define wire formats and opcodes; they do not own live
  capabilities.
- Service/client helpers wrap protocol lifetimes on top of `catten-rt::owned`;
  they do not duplicate syscall cleanup.
- Drivers may use raw MMIO, DMA, CQ, and device operations only where the typed
  runtime API cannot express the hardware contract. Keep that raw region small
  and expose an owned interface to the rest of the service.
