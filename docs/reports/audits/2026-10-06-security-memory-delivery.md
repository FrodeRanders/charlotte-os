# IPC owned-memory delivery — 2026-10-06

Baseline: `1bd9a919`, following
[namespace memory retirement](2026-10-06-security-memory-retirement.md).
This closes newly identified SEC-29. SEC-18 remains partial.

## SEC-29: owning memory was usable before IPC delivery

**Severity: medium ownership/availability. Corrected.** IPC published moved or
copied memory in its destination namespace before receiving the queued message.
Returned memory likewise became usable before the caller observed the result.
Ordinary memory lookup required live authority but had no delivery gate. A
recipient guessing a valid local capability ID could therefore read, map,
transfer, loan or DMA-pin undelivered owning memory.

Queued cancellation, endpoint close and unobserved-result cleanup used
`try_close_cap` and ignored its error. Mapping or pinning could make that close
reject while IPC still discarded its message/result receipt. The memory authority
and original sponsorship charge then remained live beyond the promised
undelivered cleanup. Early onward movement could also remove authority from the
namespace that IPC expected to reclaim. Backing fences prevented premature frame
reuse; this finding does not establish use-after-free or access to an unrelated
domain's memory. The recipient was an intended IPC destination, but had not yet
received the ownership transfer.

This gap is confirmed by source inspection and regression fixtures using real
queued/result identities. The fixtures exercise the kernel memory boundary;
they do not demonstrate an external attacker or a separate EL0 exploit payload.

## Delivery ownership

Owning IPC memory now publishes with `MemoryCap::delivery_pending`. All ordinary
memory lookups reject that state with `UnknownCapability`, including explicit
application close, byte access, mapping, transfer/loan preparation and DMA
pinning. Unified authority remains live and charged while its queued message or
unobserved result owns it. The private cleanup lookup allows IPC to consume it.
An application cannot create the mappings/pins/loans that previously made this
cleanup reject and strand the ownership receipt.

Scalar/vector calls and asynchronous sends, plus both immediate and split-phase
returned-memory replies, use `commit_undelivered_transfers[_with_authority]`.
Source escrow, backing pins and jointly reserved IPC authority retain their
existing transaction. Trusted non-IPC transfers still publish immediately using
the ordinary commit API. No additional teardown allocation, lifecycle acquisition
under IPC, root lease or provisional frame owner is introduced.

Receive publishes owning attachments under the IPC write hold after speculative
reply admission and result-page writing succeed. Failure leaves the message
queued and its memory hidden. The first successful reply poll publishes returned
memory under the same IPC hold that records observation; concurrent pending-call
close cannot reclaim it afterward. Repeat polling does not republish authority.
Readiness waiting alone does not observe or transfer the result.

Delivered moves/copies belong to their recipient and survive call/endpoint
cancellation, including when mapped. Queued and unobserved owning memory stays
exclusively reclaimable by IPC. Loans retain their separate owned revocation
contract and existing delivery semantics; this gate is for moves/copies, not a
claim that every IPC capability family has deferred visibility.

The prior report accurately recorded serialized attachment cleanup, but that
adapter closes **unmapped** memory and rejects mapped memory; it did not itself
perform owning-attachment TLB invalidation under IPC. This batch addresses the
premature application use that could strand cleanup, rather than introducing an
unlocked mapping teardown for those attachments. Physical allocator release can
still occur under IPC, with its existing latency and quarantine behavior.

## Validation

New boot fixtures capture actual queued/unobserved scalar IDs. Before delivery
they verify rejected information/byte access, mapping, unmap, explicit close,
move/copy/loan preparation and DMA pinning, while checking unified authority
remains present. Scalar moves/copies cover calls and sends, cancellation and
successful delivery. Failed receive result-page writing retains the queue and
hidden authority. Delivered memory remains usable and mapped after cancellation;
undelivered cleanup removes unified authority and refunds its original backing
charge. Whole-server close also reclaims a queued move and completes its caller.

Vector call/send fixtures cover both owning modes and publication of all
attachments through successful result-page receive. Returned-memory fixtures
cover readiness without observation, first poll, unobserved cancellation and
observed ownership. Existing split-phase returned-memory fixtures now require
hidden results until polling, and continue to test loan cleanup, preparation/
publication rollback, staged root close and source escrow. These new tests add
no deliberately quarantined roots or pages. Existing fault quarantines remain.

| Check | Result |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed, including slot/scratch ownership, runtime/services/protocol, signing and boot-result suites. |
| x86/AArch64 kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check` and `git diff --check`: passed. |
| Four-LP x86 guest | **15 passed, 0 failed, 0 pending**, including new delivery fixtures and existing EL0 memory IPC tests. |
| AArch64 security guest | **19 passed, 0 failed, 0 pending**. Both probes reported `0xffff`, publication generations reached 1 and 2; cancellation traffic retired after 4,768 requests. TCP/IP clock/cycle progress held while pressure clients retired after 1,492 and 1,440 requests. |

Successful guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance memory-delivery-20261006 --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18497 \
CATTEN_DEPLOY_HOST_PORT=17797 scripts/run-aarch64.sh --security-test \
  --instance memory-delivery-20261006 --fresh-storage --timeout 180
```

Runners rebuilt both kernels and checked native assembly permissions. Signed
service bundles were reused from the preceding validated build; this batch
changes kernel code only. The first ARM launch could not bind host-forwarding
ports within the sandbox; the approved isolated retry passed. No unrelated VM or
storage was stopped/reset.

Logs:

- `/private/tmp/charlotte-delivery-host.log`
- `/private/tmp/charlotte-delivery-clippy-x86.log`
- `/private/tmp/charlotte-delivery-clippy-arm.log`
- `/private/tmp/charlotte-delivery-guest-x86.log`
- `/tmp/charlotte-x86-memory-delivery-20261006-serial.log`
- `/private/tmp/charlotte-delivery-guest-arm-approved.log`
- `/tmp/charlotte-memory-delivery-20261006-serial.log`

## Remaining scope

SEC-18 remains partial: allocator release latency under IPC, recoverable
shootdown failure, full CPU/DMA quiescence, physical-device reset and recovery
for abandoned roots remain open. Final root shutdown still depends on
caller-established thread quiescence and recipient progress. Deferred visibility
of non-memory IPC authority is outside this batch.

SEC-07 translation/metadata admission, peer/management authentication, production
provisioning and security-time work remain open as previously documented.
