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
- Run `cargo fmt --all -- --check` after Rust edits. All Rust packages belong
  to the root workspace even when they use separate build targets.
- For ownership changes, test success, submission failure, mapping failure,
  cancellation/drop, and returned-capability cleanup where practical.

## Architectural boundaries

- `catten-syscall` mirrors the register ABI and intentionally exposes integers.
- `catten-rt::owned` is the safe application layer and owns kernel resources.
- Protocol crates define wire formats and opcodes; they do not own live
  capabilities.
- Service/client helpers wrap protocol lifetimes on top of `catten-rt::owned`;
  they do not duplicate syscall cleanup.
- Drivers may use raw MMIO, DMA, CQ, and device operations only where the typed
  runtime API cannot express the hardware contract. Keep that raw region small
  and expose an owned interface to the rest of the service.
