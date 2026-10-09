# IPC backing release outside serialization

Date: 2026-10-06. Follow-up to SEC-18 after returned-memory source qualification
at `303ef861`. This is a point-in-time remediation record. Current invariants
live in [memory-object retirement](../../reference/memory-object-retirement.md)
and repository contributor instructions.

## Problem and resulting behavior

Closing undelivered move/copy attachments or unobserved returned memory called
`try_close_cap` while retaining the IPC write guard. That adapter dropped the
memory registry before physical free, but the outer IPC guard still masked
interrupts and serialized unrelated communication across the entire frame list.
A large object or endpoint queue could multiply that hold. Existing ownership
visibility prevented applications from mapping or pinning these owning caps;
it did not release the outer guard during physical deallocation.

Published attachment cleanup now removes authority and payload under IPC into
`RetiredMemory`, then consumes that owner after IPC unlock. Its original frame
vector and sponsor charge remain live until explicit physical release. No
numeric-ASID lookup occurs during release, so namespace closure/reuse cannot
refund a successor's accounting. Dropping an unfinished retirement quarantines
its backing, charge and frame metadata without allocator/registry work.

Owning retirement rejects active mappings, loans, DMA/copy/retirement pins and
transfer escrow before mutation. Normal IPC delivery visibility makes such
states impossible for undelivered owning attachments. Confirmed loan revocation
may already have removed a loan cap; it is not retired as a second backing
owner. Unexpected live-use rejection is an invariant failure, not ignored
successful cleanup.

## Admission and cleanup storage

Vectors can contain **255** memory entries. `MemoryAttachments` reserves space
for retirement owners fallibly before attachment authority publishes. Submission
failure still restores prepared source moves and drops staged copies, capability
reservations and IPC metadata. The existing prepared-call transaction owns that
rollback; no manual scalar restoration path was added.

Each message carries its admitted retirement storage through dequeue or cleanup.
Bulk endpoint close moves the existing admitted queue outside IPC after retiring
all attachments; its queue charge survives until that backing storage actually
frees. Queued cancellation carries one message owner and a single unobserved
result uses one inline retirement owner. Cleanup does not allocate a snapshot,
grow vectors or use a large stack array sized by the vector limit.

Prepared cancellation confirms every loan first, then detaches owning
attachments. Backing releases outside IPC before exact caller/server root leases
complete. Explicit endpoint close retains its claim/server lease through both
asynchronous release and each queued call's completion. Whole-domain cleanup
retains its borrowed closing owner. Delivered/observed memory keeps application
ownership and survives pending-call/endpoint cleanup.

`ChargedFrames` additionally releases at most **16 frames per physical allocator
hold**, consuming each identity before deallocation. Its allocator guard drops
between batches. This bounds the number of frees in one hold, not elapsed
latency or the duration of inherited outer guards. Staged-copy rollback still
retains IPC; changing its full ownership transaction remains separate work.

Physical deallocation failure retains the original whole charge, including
partially freed backing, and is never retried. IPC logs the rejected release
and may complete logical close: authority was detached and no mapping/pin
survived. This policy differs from uncertain mapped-loan invalidation, where
terminal success is forbidden and the call/root/backing must remain fenced.

## Regression coverage

Kernel ABI boot fixtures now exercise:

- Retirement-storage rejection before publication with source move restoration,
  staged-copy rollback, attached-connection admission refunds, unchanged backing
  and capability counts, and an empty endpoint queue.
- Two full vectors containing **510 owning attachments**, released through
  existing queue storage after unlocking IPC.
- Queued-call and unobserved-result cleanup of 35-page backing, including
  authority detachment before free and charge retention until completion.
- Queued read/write loan cancellation with owning move/copy extras; each loan
  finishes before retirement and both root leases remain live during release.
- Owned endpoint cleanup spanning a loan-bearing call and asynchronous owning
  send, with its server root leased through all backing releases.
- Rejection of mapped and copy-pinned retirement without consuming authority.
- Explicit detached release after original-root closure and same-ASID generation
  reuse, with the original charge refunded and successor accounting untouched.
- Drop quarantine after generation reuse, retaining its page and original charge.

Release callbacks assert that IPC, lifecycle, address-space table, memory-object
registry and physical allocator guards are available before physical free.
35-page releases traverse multiple bounded allocator batches. Existing receive
rollback, observed ownership, staged close, cancellation/reply failure and
abandonment fixtures remain enabled.

The Drop probe deliberately retains **one additional 4 KiB page and one object
charge** for the test guest's lifetime. The object-retirement fixture total is
now twelve data pages and ten charges, separate from other kernel/IPC root
quarantine probes. No fixture re-adopts or retries quarantined backing. These
are software fault/interleaving tests, not actual DMA timeout or failed IPI
injection.

## Validation

| Check | Result |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed: runtime/services/protocol, lifecycle/ownership, signing and boot-result suites. |
| x86/AArch64 kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check` and `git diff --check`: passed. |
| Four-LP x86 guest | **15 passed, 0 failed, 0 pending**; new backing and existing ownership fixtures executed. |
| AArch64 security guest | **19 passed, 0 failed, 0 pending**; probes reported `0xffff`, publication generations reached 1 and 2, concurrent cancellation retired after 4,752 requests. TCP/IP time/cycle progress held while pressure clients retired after 1,404 and 1,508 requests. |

Commands for the final guest runs:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance ipc-backing-20261006 --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18500 \
CATTEN_DEPLOY_HOST_PORT=17800 scripts/run-aarch64.sh --security-test \
  --instance ipc-backing-20261006 --fresh-storage --timeout 180
```

Both runners rebuilt their kernels and checked native assembly permissions.
Validated staged service bundles were reused; this batch changes kernel code
only. ARM used approved isolated localhost forwarding. No unrelated VM or
storage was stopped/reset.

## Remaining scope

SEC-18 remains partial. Unpublished staged-copy rollback and general
allocation/metadata destruction under IPC, recoverable shootdown failure, full
CPU/DMA quiescence, physical-device reset and recovery for abandoned roots remain
open. Root shutdown still depends on caller-established thread quiescence and
recipient progress. This change provides neither a latency measurement nor a
recovery API for quarantine.

SEC-07 translation/metadata admission, peer/management authentication, production
provisioning and security-time work remain open as previously documented.
