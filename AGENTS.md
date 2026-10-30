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

## Waiting, cleanup and admission

- Retain one complete owner for accepted work, its memory borrows, exact root
  generations, authority and original charges until confirmed completion or
  cancellation. Timeout, waiter admission rejection and readiness are distinct;
  none permits replay or early release.
- Register interest in the resource that can make progress, recheck readiness
  after registration and keep one monotonic deadline across unrelated wakes.
  Services with several sources must bind their pending calls and endpoints to
  the same CQ. A notification is a hint; observe results through their owners.
- Prepare fallible storage and reservations before mutation/publication. Retain
  original sponsorship through transfer, quarantine and generation reuse; refund
  only after actual confirmed release. Platform classification comes from trusted
  captured state, never a manifest, role, name or later ASID lookup.
- Qualify enclosing locks, IRQ state and implicit field destruction before
  cleanup. Release producer guards before notification. Never enable interrupts
  beneath an unknown mask to permit waiting. Preserve terminal fences and charges
  when completion is uncertain; retry requires the complete phase-correct owner.
- Polling needs a concrete missing notification contract and a bound or terminal
  policy. Record that limitation; a short sleep does not establish readiness.

## Required subsystem reading

The detailed requirements in
[Subsystem contributor contracts](docs/reference/contributor-contracts.md)
remain part of these instructions. Read each applicable section before editing;
cross-subsystem changes require all affected sections. This separates detailed
rules from the entry-point instructions without relaxing their scope.

| Work area | Required section |
| --- | --- |
| Cleanup/recovery | [Cleanup change obligations](docs/reference/contributor-contracts.md#cleanup-change-obligations), [mapping retirement and domain cleanup](docs/reference/contributor-contracts.md#mapping-retirement-and-domain-cleanup), and the affected [owner-family row](docs/reference/cleanup-recovery.md#receipt-coverage-map) |
| Blocking, readiness or service reactors | [Waiting and readiness](docs/reference/contributor-contracts.md#waiting-and-readiness) |
| Admission policy, boot drivers, signed launch or placement | [Trust, boot and placement](docs/reference/contributor-contracts.md#trust-boot-and-placement) |
| Protocols, naming, catalog, connector profiles or persistent formats | [Internal protocols and persistent state](docs/reference/contributor-contracts.md#internal-protocols-and-persistent-state) |
| Capability, mailbox, IPC or device-grant publication | [Capability and IPC publication](docs/reference/contributor-contracts.md#capability-and-ipc-publication) |
| Heap, image, translation or mapping lifetime | [CPU backing and page tables](docs/reference/contributor-contracts.md#cpu-backing-and-page-tables) and [mapping retirement and domain cleanup](docs/reference/contributor-contracts.md#mapping-retirement-and-domain-cleanup) |
| Thread creation, abort, stacks, switching or reaping | [Threads, stacks and scheduler handoff](docs/reference/contributor-contracts.md#threads-stacks-and-scheduler-handoff) |
| DMA, MMIO, IRQ, hardware reset or IOMMU | [Devices and IOMMU backing](docs/reference/contributor-contracts.md#devices-and-iommu-backing), plus publication/cleanup/waiting sections as applicable |
| Observer, completion, CQ, watch or timer | [Observers, completions and timers](docs/reference/contributor-contracts.md#observers-completions-and-timers) and [waiting and readiness](docs/reference/contributor-contracts.md#waiting-and-readiness) |
| Syscall registers, SDK, runtime ABI or layer boundaries | [Runtime ABI and layer boundaries](docs/reference/contributor-contracts.md#runtime-abi-and-layer-boundaries) |

## Documentation and audit records

- Capture durable rules, examples and limitations in the relevant guide or
  reference page. Update current contracts and remaining-work tables in place;
  do not append a remediation narrative for each patch. Start with
  [security engineering](docs/guides/security-engineering.md).
- Keep [security status](docs/reference/security-remediation.md) focused on
  current dispositions, acceptance gates and unresolved incidents. Update the
  existing cleanup owner-family row when its contract or qualification changes;
  new object instances do not create a new family.
- Use dated reports for significant investigations, audit checkpoints or failure
  captures that need a historical record. Routine successful commands belong in
  the change's validation summary. A new report is not required for every fix.
- Keep published reports self-contained: concise results, artifact identities
  and relevant diagnostics inline or in version-controlled evidence. Never cite
  temporary logs or ignored build artifacts as durable evidence.
- Preserve historical reports and failed runs separately from passing repeats.
  Keep unresolved/intermittent failures visible; distinguish observation from
  causal attribution. If original captures are unavailable, state the limitation.
  Documentation compaction does not close a finding or explain an incident.
