# Current-only Raft persistence and isolated watchdog fixture

Third batch of the [simplification sweep](../../reference/api-simplification.md),
based on `55509c68`. The user confirmed that no old stores exist in production;
this batch removes old layout and migration code outright. It also records the
requirement for explicit, interruption-tested migration when deployed stores
exist in the future.

## Current persistence boundary

`disk_raft::open` jointly validates a compact two-object pair before exposing
either store: tagged `CRFTSTA1` term/vote/join-admission state and the current
`CRFTLOG1` atomic snapshot/log record. State and log occupy consecutive IDs in
the existing `0x8xxx` partition. The old four-slot layout, independent store
constructors, term/vote-only decoder, split-snapshot migration and raw-entry
fallback are removed. There are no retired-slot probes or automatic converters.
Only two absent records authorize initialization; exact successful creation of
both is required. Existing empty, incomplete, corrupt or unsupported records
reject without overwrite or reset. Reopen of a valid pair writes nothing.

State includes mandatory join fields, with an explicit absent-admission
encoding. Log decode consumes the entire declared stream and rejects invalid
UTF-8, partial entries, overflow and the retired zero sentinel. Encoding rejects
oversized IDs instead of truncating them. Snapshot data and retained suffix
still share one object replacement; term/vote/admission has its own record.
This is not cross-object atomicity. Every mutation flushes before returning;
encoding/persistence failure remains fail-stop.

DNS opens the pair through one shared owning runtime `Connection`. The generic
Raft service now accepts only memory mode 0 (or omission) and required disk mode
2; retired mode 1 and malformed policy values reject. Disk-open failure cannot
silently create a volatile voter. `Client`/`PreparedWrite` use `OwnedMemory`,
`MappedMemory` and `PendingCall::wait`, including exact returned-owner cleanup.
The scalar owning connection, fixed scratch address and manual cleanup ladders
are removed. Write buffers prepare before remote submission; accepted calls
are not reconstructed or replayed. Durable object IDs are names, not transient
resources deleted by Drop. C08 evidence is updated; no new kernel cleanup
receipt, custody registry or physical completion proof is added.

The [wire/storage reference](../../reference/raft-persistence.md), architecture,
Markdown/LaTeX/PDF manual, contributor instructions and sweep ledger reflect
the contract. Old development disks require intentional regeneration and are
not silently erased. Future deployed-store changes must preserve term, vote,
admission fences, snapshot membership and exact suffix through an explicit
version-directed migration with a durable commit point. Unknown tags must
remain distinct from absence; no speculative compatibility reader is retained.

## Host regression evidence

Five new tests use the real disk-store implementations with scripted object I/O:

- Fresh initialization creates/writes exactly two objects, flushes and reopens
  without mutation. Dropping store owners leaves the durable records intact.
- Failure at each read/create/write/flush step stops without replay; a creation
  conflict retains foreign bytes and produces no subsequent write.
- Incomplete, empty, retired, unknown-tag and malformed pairs reject without
  repair, overwrite or partial publication to Raft.
- Term, vote, join anchor/fence, snapshot and compatible suffix survive reopen;
  matching snapshot replacement preserves the suffix, truncation persists and
  an explicit consensus reset clears only log/snapshot state.
- Strict codecs reject every truncated prefix, bad UTF-8, excessive fields,
  aggregate/index overflow and a bad second entry or partial trailing entry
  without returning the valid prefix.

These are storage-adapter fault probes, not device power-loss injection or
qualification of every runtime mapping/cancellation failure. The existing
owned-runtime host suite remains enabled. Native process restart and disk
reuse supply separate execution evidence below.
Local reconstruction-allocation failure still shares `MalformedRecord`, and
runtime admission/buffer failure can share `Io`; those labels alone do not
prove corrupt storage or authorize reformatting. Allocation-error diagnostic
separation remains outside these format-removal probes.

## Initial failed pair and fixture correction

Both initial guests used rebuilt signed services, fresh storage, distinct MACs,
`--dns-test`, socket port 19472 and a 180-second limit. The x86 kernel was
`b564de409ce83209cf819a886c10f37f60ccbb854c91abd84db78549d1d9ff4a`;
ARM was `eab9089464e96a4cdffecef1b18681e22ce2b74b417764843f51ce1920f6b09d`.

At about 17.88 seconds x86 panicked at `self_test/cq_wait.rs:268:9`:

```text
assertion `left == right` failed
  left: 5
 right: 2
```

Matching symbols resolve `0xffffffff80237e4f` to
`catten::self_test::cq_wait::cq_driver+0x82ff`; the surrounding path is
`__rustc::rust_begin_unwind+0x4e`, assertion/panic helpers and
`kernel_thread_trampoline+0x8`. The x86 runner exited 1 with no authoritative
terminal result; its last observed aggregate was 11 passed / 7 pending,
bitmaps `0x22bfc` / `0xd9003`.

ARM's isolated NVMe Raft fixture recovered term 1 and advanced to term 2 on
process restart. Its cross-node discovery nevertheless observed no peer,
then panicked at `self_test/el0_disco.rs:116:26` around 122.57 seconds:
`self-test deadline expired while waiting for EL0 disco cross-node peer`.
The final aggregate was 18 passed / 1 failed / 1 pending, bitmaps `0x3ffff`,
`0x40000`, `0x80000`; the runner exited 1. These failed runs remain separate.
They do not prove which timer changed the aggregate or why network progress
stopped after x86's panic.

Source review establishes a concrete fixture error: `memory::budget::timer_sponsor`
returns the address-space generation's shared account; `thread_timer_context`
clones that account. All these kernel threads use `KERNEL_ASID`. The probe's
claim that its sponsor was private to the driver was false, and a local IRQ
mask did not exclude reservation/reclamation on another LP. Saturating that
shared account could also reject other kernel threads' watchdog admission.
This accounts for an invalid isolation premise, not the identity or cause of
each historical counter delta.

The correction gives the raw boot fixture's still-unpublished driver a fresh
existing `SchedulerSponsor` with trusted kernel platform classification,
before publication or any watchdog capture. The driver retains its original
kernel root and normal stack/thread admission. Production `Thread` construction
still captures its root-generation account; no live sponsor is replaced and
no domain/node ceiling, production wait, timeout or physical release changes.
Preallocation and the short known-rejecting mask remain; pressure is confined
to the private fixture sponsor. C16 evidence and contributor instructions are
updated. No accounting API, bypass or new registry is added.

## Validation

Validated 2026-10-13:

- `scripts/run-host-tests.sh`: 402 passed, zero failed, four ignored
  documentation examples. Five persistence tests account for the change from
  397. Signing-policy, boot-result, syscall-emitter probes and signing-tool
  self-tests also passed.
- Both signed service bundles rebuilt; strict AArch64 service Clippy passed.
  Both corrected kernels built and passed strict Clippy for the exact
  DNS/discovery profile. Initial kernel Clippy invocations rejected missing
  compile-time `CATTEN_AARCH64_SERVICE_BUNDLE` / `CATTEN_X86_64_SERVICE_BUNDLE`
  variables (exit 101); corrected invocations supplied the rebuilt paths and
  exited 0. No source change was needed for those invocation errors.
- `cargo fmt --all -- --check`, `git diff --check` and local Markdown link
  resolution passed. The PDF rebuilt in two LaTeX passes, has 208 pages, and
  rendered pages 102 and 111 were visually inspected. Current record formats,
  rollout and future migration text are readable without clipping; unrelated
  existing manual layout diagnostics remain outside this batch's scope.

Corrected paired runs use `--dns-test`, MACs `52:54:00:12:34:61` (x86 listener)
and `52:54:00:12:34:62` (ARM connector), a 180-second limit and the same
qualified instances/disks. The kernels were prebuilt; both guests launched
together using the runner's explicit skip-build options. ARM's authorized
outside-sandbox socket launch used the explicit network override.

| Run | Storage / link | Authoritative result | NVMe single-voter recovery |
| --- | --- | --- | --- |
| ARM corrected fresh | Fresh disk, port 19473 | 20/0/0 at 25.373758 s, bitmap `0xfffff` | First process term 1, replacement term 2 |
| x86 corrected fresh | Fresh disk, port 19473 | 18/0/0 at 30.255443 s, bitmap `0xfbbff` | First process term 1, replacement term 2 |
| ARM corrected reboot | Same disk, `--reuse-storage`, port 19474 | 20/0/0 at 24.324023 s, bitmap `0xfffff` | Recovered term 3, replacement term 4 |
| x86 corrected reboot | Same disk, `--reuse-storage`, port 19474 | 18/0/0 at 28.770888 s, bitmap `0xfbbff` | Recovered term 3, replacement term 4 |

Every corrected runner exited 0. Both architectures completed discovery and
replicated DNS catalog/remote invocation/generation-fenced unregister on both
boots. The 1→2 then 3→4 progression shows persistence survived process teardown
and whole-guest reboot, rather than reinitializing the pair. Cold paired restart
is not single-replica recovery inside a running quorum or power-loss testing.
The initial failed pair remains separate; corrected passes do not identify
individual historical counter changes or prove general transport reliability.

Reproduction flags (fresh pair; repeat with `--reuse-storage` and port 19474):

```text
scripts/run-x86_64.sh --dns-test --net-listen 19473 --mac 52:54:00:12:34:61 --fresh-storage --timeout 180 --instance raft-cleanup-x86-qualified
scripts/run-aarch64.sh --dns-test --net-connect 127.0.0.1:19473 --mac 52:54:00:12:34:62 --fresh-storage --timeout 180 --instance raft-cleanup-arm-qualified
```

SHA-256 identities captured for the actual artifacts:

| Artifact | SHA-256 |
| --- | --- |
| Corrected ARM kernel, both boots | `5d6a08e7e13be3cc3ba841b802224ec387a0a7055f8725991f5e76b10ac5c3f3` |
| Corrected x86 kernel, both boots | `12f9f250fcf48ebac946048b50b3ceb5008498ac077363431c303ae07e2278d9` |
| Signed ARM bundle, 41 ELFs | `c69918624834109b23ee4c80256c3a39fe8f9eb46314f034f62bbadc4db2b2a0` |
| Signed x86 bundle, 37 ELFs | `6d5b9568f6e01a2276e4935709401db142d3e6bb37438f0f22f7d1f8000e1e9e` |
| Rebuilt manual PDF | `100b6328e6922e783c3935306a94f3847ffcd14be644f382c2624e428e6516d3` |

Bundle identity hashes sorted, newline-terminated `basename:ELF-SHA256` lines.
Failed and passing results, relevant symbols and artifact identities are
recorded inline; temporary logs, ignored outputs and symbol sidecars are not
published evidence dependencies. These QEMU images use public development
trust, not production enrollment.

## Remaining scope

Deployment/catalog APIs are next (S04). Production migration tooling becomes a
requirement when an actual deployed format is superseded; no current production
store requires conversion. Device power-loss atomicity, wider kernel guard/Drop
qualification and historical scheduled-close/transport failures remain open.
This simplification and fixture correction close no security finding counts.
