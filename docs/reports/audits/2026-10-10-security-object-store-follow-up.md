# Object-store isolation and abort-handoff verification follow-up

This corrects the two SEC-12 defects and the mutex-context probe identified in
the [commit progress verification](2026-10-10-security-progress-verification.md).
The worktree is based on `ec8924fe7d319720d54fb1cb0a68ee785ea6540c`.
The earlier review, reproducer and failed AMD capture remain historical evidence.

## Object-store corrections

`StoredObject` holds one validated locator/header from lookup through the
serialized request. `resolve` reports NotFound only for an absent ID; header
I/O/validation failure returns `ERR_IO_ERROR`. The dispatcher rejects that
failure before any authorization or data operation. Read/info use the same
header that supplied the owner and attributes; they do not retry metadata.
Delete, resize and write also consume the validated object. True owner-zero
legacy headers retain their documented policy; failed reads cannot create one.

Creation now calls the host-tested `admit_create_id`; its automatic path uses
`next_automatic_id`, skips occupied IDs and publishes at most one directory entry
per ID. A free-slot check precedes the automatic search. Existing explicit IDs
still report `ERR_EXISTS` when the directory is full. At most directory-slot-count
plus one candidates are examined;
rejection leaves the original counter unchanged. Automatic IDs are positive
scalar replies (at most `i64::MAX`), excluding high stable and reserved namespaces.
Explicit creation does not advance the automatic counter. Mount rejects duplicate
live IDs instead of silently choosing an owner by directory order. Deletion
also removes the object's pending size, preventing stale resize state from
following a later explicit reuse.

No new owning capability is stored as an integer. Request attachments, borrowed
buffers and moved replies retain the existing `catten_rt::owned` wrappers.
The validated persisted-object snapshot contains disk metadata, not a kernel
capability or independent cleanup/retry authority.

## Mutex-context probe correction

The original scheduled handoff probe asserted that three global mutexes could
be acquired immediately. Legitimate ownership on another LP could fail that
assertion even when the caller held no enclosing guard. This is distinct from
the invariant the probe intended to establish.

`MutexCore` now records the owning LP atomically on successful lock/try-lock
acquisition and clears it before unlock/IRQ restoration. A short local mask pins
the observation and preserves the entry IRQ policy. No allocation, logging,
payload access, blocking acquisition or new custody registry occurs in the
observation. Owning mutex guards already mask IRQs and cannot migrate. The marker
does not authorize access to the mutex payload or change lock ordering.

The scheduled handoff still rejects a lifecycle, publication or table mutex
held on its own LP. It accepts unrelated remote contention rather than disabling
the context check. A deterministic guest fixture tests both successful lock and
try-lock ownership, failed recursive try-lock, unlock clearing, and actual
remote lifecycle contention. While the remote hold is live, global `try_lock`
must fail and the local-ownership observation must be false. Entry IRQ state is
asserted before/after observation and remote completion. The two-thread handshake
has a separately chosen five-second fixture deadline; no historical deadline
was increased.

The failed AMD review run did not record the holder, so its precise cause is
still not proven. The new fixture reproduces the old assertion's false-positive
condition and qualifies its replacement. This does not close historical stack
lease timeouts, other guard classes, full caller inventories or SEC-18.

## Targeted regression evidence

Four host tests execute the shared production admission/resolution helpers:

- metadata failure returns I/O error without a retry; missing IDs do not read
  a header; validated foreign, owner and legacy headers retain their policies;
- mixed explicit/automatic creation preserves foreign IDs across restored
  counters and deletion, without substituting their owners;
- full-directory admission preserves explicit existence/invalid-ID errors and
  rejects automatic creation without probes or counter advancement;
- candidate search obeys its probe bound, rejects exhausted/reserved/negative
  reply ranges and leaves counters unchanged on rejection.

The storage guest now stages kernel-owned object 1 at the fresh automatic
counter. Its ordinary client must receive a different automatic ID and still
be denied read/info/delete on object 1. It deletes its new object explicitly.
The same client submits a well-formed empty scope table and must be denied;
its existing read-only grant must survive. The kernel also submits a malformed
table and requires rejection before that grant is exercised.

After those checks, the kernel installs a valid empty table while the client
and its original connection remain live. An authenticated kernel-only fixture
request asks that same principal to verify that both read and info now return
`ERR_DENIED` with no payload. Store mount logs successful validated table restore;
a later disk-reuse execution checks persisted revocation rather than rebuilding
the NVMe image. This remount evidence is separate from sudden-power-loss and
failed-device-write qualification.

Selected success diagnostics:

```text
[domain abort] lifecycle ownership observation: local hold detected, remote contention accepted and caller IRQ state preserved
[objstore-client] automatic-ID collision preserved foreign ownership; unauthorized scope replacement denied and original read grant preserved
[objstore-client] live scope revocation denied read and info
[objstore] persisted scope table restored (0 entries)
```

## Validation

- Workspace host harness: **353 Rust tests passed, zero failed, three ignored**,
  plus signing/boot-result checks and 68 syscall-emitter call sites.
- Both architecture service bundles rebuilt and signed from the corrected code.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Strict kernel Clippy (`-D warnings`): AArch64 default and x86 headless ACPI
  configurations passed.
- Strict service Clippy (`--release -Z build-std=core,alloc -D warnings`): passed.

Final-artifact executions (each runner exited zero):

| Execution | Passed | Failed / pending | Passed bitmap | Targeted evidence |
| --- | ---: | --- | --- | --- |
| AArch64 security, fresh NVMe image | 19 | 0 / 0 | `0x2003ffff` | Collision isolation, scope rejection/live revocation, forced remote contention |
| AArch64 security, same instance with disk reuse | 19 | 0 / 0 | `0x2003ffff` | Validated persisted table restored with **zero entries before fixture grant installation**, followed by the same negative checks |
| x86 AMD-Vi, fresh image | 15 | 0 / 0 | `0x1bbff` | Collision isolation, scope rejection/live revocation, forced remote contention |
| x86 Intel VT-d, fresh image | 15 | 0 / 0 | `0x1bbff` | Same x86 kernel and bundles as AMD, same negative checks |

Commands used for the final executions:

```sh
scripts/run-host-tests.sh
scripts/build-catten-services.sh --embed
scripts/build-catten-services-x86_64.sh

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18974 CATTEN_DEPLOY_HOST_PORT=18274 \
  scripts/run-aarch64.sh --security-test --instance security-followup-complete-20261010 \
  --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18974 CATTEN_DEPLOY_HOST_PORT=18274 \
  scripts/run-aarch64.sh --security-test --instance security-followup-complete-20261010 \
  --reuse-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance security-followup-amd-complete-20261010 --fresh-storage --timeout 150
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_SKIP_KERNEL_BUILD=1 \
  scripts/run-x86_64.sh --no-network --iommu intel \
  --instance security-followup-intel-complete-20261010 --fresh-storage --timeout 150
```

Artifact SHA-256 values (results above are recorded inline; ignored artifacts
and temporary serial files are not required as published evidence):

| Artifact | SHA-256 |
| --- | --- |
| AArch64 fresh security kernel | `e41c5cb6baacee7ff81ea6130cbece2166dd62f831b0251adc67baae11142be5` |
| AArch64 disk-reuse security kernel | `b6563eab8647280c416b1d3b542a2090a59b65585c25603fd1b75b3f9034d8f0` |
| x86 AMD/Intel kernel | `3fa5f86b43e44fa556107ab98b2506b9ddb810a4f2e20ddc7d2e914b6dc9ed81` |
| AArch64 signed `objstore.elf` | `67c9e819f91c9d124795a42151859dad415aa67112b720f0cb791d01da867006` |
| x86 signed `objstore.elf` | `3ae072b52a276daef725367d55a7be6662a03829a9820c81012a083cbe067688` |
| AArch64 signed `objstore_client.elf` | `99fb56496edd41426439c8b18b4bf0776697ebbff23247a4feb3f254fb286634` |
| x86 signed `objstore_client.elf` | `2e1f7dd79752ed8daaf58e7993d5df7cd5e252df904008fa53cc181c4158a5aa` |
| Unchanged `Cargo.lock` | `6237dbf75d0e29bb7fe1c0dba4301d12d5a298d626fdc89bd05e5d766c50aa76` |

The AArch64 runner generates new development security keys/deployments for
each invocation, so its two kernel hashes differ. Source and signed service
bundles were unchanged between those executions, and disk reuse did not
reseed the image.

Corrected-source SHA-256 values:

| File | SHA-256 |
| --- | --- |
| `crates/catten-services/src/bin/objstore.rs` | `a55a2ce7afdc40795378cefb681dfa2ffcda0d17f8aa526eb05d08fe73abe21c` |
| `crates/catten-services/src/bin/objstore_client.rs` | `275a8d8e72557571a49ac8a588f5206164e85d14d5a0cbb0cd987f06079d546a` |
| `crates/catten-services/src/lib.rs` | `7c459aba4cf814621ffe1eebcf5e955a2853d2e94bf2f3a97fb7c580b119098b` |
| `crates/catten/src/cpu/multiprocessor/spin/mutex.rs` | `605bd7e64889a559319c156e4e4e8af890482a92186d0b26cc4ebde2508c64d6` |
| `crates/catten/src/cpu/scheduler/system_scheduler/domain_abort/tests.rs` | `9e9984fb1e93c872fc799f6ce103230c30616a12b5f0adb9aa610e92bed26c96` |
| `crates/catten/src/self_test/el0_nvme.rs` | `4d6560ea0c544c599ab4897df5974a3632c15b59744ed71b4676c79b2510a564` |

## Disposition and limits

The two demonstrated SEC-12 isolation defects have concrete corrections and
regression coverage. Production bootstrap/custody, node/operator authentication,
security time, aggregate resource/progress admission and broad recovery remain
separate SEC-04/07/08/09/10/18 work. This pass does not claim hostile network
pressure, real NVMe I/O failure, power-loss atomicity, cross-LP recovery under
outstanding device I/O, or physical-platform qualification. The metadata-failure
probe runs the shared production resolver with a failing callback on the host;
it does not inject a failing physical device into a guest.

The review's proposed direct negative guest probes for legacy resolution of a
policy-published name, over-ceiling relmsg frames, oversized upgrades and
TCP/IP receive-object retention remain additional coverage work. Existing host
reassembly admission and missing-transmit-reservation tests passed again, but
are not equivalent to those guest probes. In particular, SEC-32's retained
cross-domain receive charging remains partial under SEC-07; this pass changes
neither that charging model nor its disposition.

The Markdown and LaTeX storage contracts now describe the same isolation rules.
The cleanup/recovery owner-family C16 row records the narrower mutex-context
evidence without expanding its overall completion claim.
