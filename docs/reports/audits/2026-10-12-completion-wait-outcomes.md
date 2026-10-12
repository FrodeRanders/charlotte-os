# Checked timed completion outcomes

This continues SEC-18, C16/C18 and G2/G7 after the
[DMA-map failure ownership correction](2026-10-12-dma-map-failure-ownership.md).
Kernel completion waits now distinguish rejected admission from deadline expiry.
The userspace syscall shares their absolute-deadline loop and no longer treats
one unrelated notification as timeout. Pending operation/buffer ownership stays
with the existing completion. SEC-18 remains partial; finding counts are unchanged.

## Historical trigger and causal limit

The previous first Intel boot used kernel SHA-256
`11ac50b240c5bb62d81ce6f6ba2d94f79890bbd24f5b53d2a964b18540ce2351`.
Following an adjacent thread-abort diagnostic at 23.429878 s, its scheduled
memory-close parent panicked at `memory/object/close/tests.rs:231:5`:

```text
assertion failed: crate::completion::wait_timeout(SCHEDULED_CLIENT, watched, 5_000).unwrap()
```

Its final waiting observation was 17 passed / 1 pending, passed bitmap
`0x123abff`, pending `0x1000` (cq-wait), without an authoritative terminal result.
The runner exited 1 after deployment partial-body transport retries and later
node/cluster controls timed out. An identical-artifact repeat completed 18/0/0
at 18.185136 s and passed host controls. The prior report preserves both runs,
matching-symbol backtrace and failure diagnostics.

Inspection found a concrete diagnostic ambiguity: the kernel adapter called
`block_until`, which converted scheduler admission error to `false`, the same
value as deadline expiry. The captured assertion cannot distinguish those
outcomes. Inspection also found an independent syscall defect: its duplicated
one-wake implementation polled once after any wake and returned timeout when
that poll was still pending, even if the requested deadline had not expired.
Neither source finding proves what caused the historical Intel failure. That
fixture called the kernel adapter, not the duplicated syscall adapter. Shared
causality with its later ingress failures remains unresolved.

## Correction and ownership

`completion::wait_timeout` retains its captured `CompletionRef` and invokes
`try_block_until`. `Ok(true)` means terminal, `Ok(false)` means deadline expiry,
and `CapError::WaitAdmissionFailed` means source/waiter/watchdog rejection.
Unknown namespace/capability remains a separate lookup error. Admission rejection
and timeout neither consume the result nor cancel/close the operation or its
buffer. Terminal readiness can win an admission race through the existing
condition recheck; that returns success rather than a synthetic error.

The syscall now uses this same adapter. Its existing register ABI is unchanged:
READY plus result, PENDING_OR_TIMEOUT on expiry, WAIT_ADMISSION_FAILED on rejected
admission, or INVALID_CAPABILITY on lookup failure. It polls/consumes a result
only after terminal readiness. The redundant syscall watchdog/waker path is
removed; no compatibility-only branch remains.

The shared scheduler computes one deadline before its wait loop. An unrelated
wake cancels that iteration's unused watchdog, checks the captured completion
again and reparks using the remaining budget. Registration retains its owned
generation-bound token; the post-registration condition recheck closes the
lost-wake window. An exhausted budget returns timeout without attempting fresh
waiter/watchdog admission. Ordinary rejection leaves the caller runnable and
cleans only unused subscription/watchdog storage.

Fallible watchdog and timeout-observer preparation now precedes the helper's
local park-publication mask. Preparation keeps the exact generation and owned
scheduler sponsor, admits the watchdog before allocating its observer, and
recomputes the remaining budget after allocator/scheduling delay. Successful preparation that exhausts the remaining budget returns
readiness/timeout and disposes unused storage without
publishing Blocked. Only registration, lost-wake recheck and watchdog enqueue
retain the helper mask. Full watchdog rejection does not allocate its observer.
Any enclosing caller IRQ policy remains unchanged; this is not permission to
park under an arbitrary caller mask or qualify every allocator/outer context.

Existing `Completion`/`ReadOperation` owners retain their cancel, terminal-wait
and close policy. Runtime WAIT_ADMISSION_FAILED handling already preserved the
owner and required no change. No new source, producer callback, root lease/count,
stack/backing charge, physical retry, cleanup receipt, registry or custody is
introduced. Caller IRQ state is preserved, without forcibly enabling interrupts.
This does not qualify every outer syscall/kernel IRQ or destructor context.

## Exact-generation failure observations

The scheduled memory-close fixture keeps its original roots, transfer and exit
watch through the same five-second deadline. Its existing progress atomic now
records 0 before entry, 1 while the child executes close, and 2 after close
returns. On rejected/expired exit wait it records the exact worker TID/generation,
matching live state, staged-retirement snapshot and completion lifecycle state.
All registry guards leave before logging or panic; inspection never polls or
consumes the watched result.

These are copied observations, not recovery authority. A missing live/staged
entry can mean a detached retirement batch; a return marker or notification
proves neither exit nor stack/root/physical quiescence. The diagnostic path does
not close uncertain roots, decrement counts, retry cleanup or extend deadlines.
Historical lost phase information cannot be reconstructed retroactively.

## Regression evidence

The scheduled CQ fixture exercises both kernel and actual dispatcher adapters:

- Saturate the target completion's independent list with 64 owned subscriptions.
  Each adapter rejects admission distinctly; the exact caller remains Running,
  its pending buffer/result stays live. Rejected registration does not mutate
  the full list; unused owned watchdog preparation is disposed on rejection.
  An ambient timer-sponsor snapshot is not used as an equality proof across
  allocator/scheduling delay or older-node reclamation.
- Saturate only the current thread's captured scheduler-timer sponsor, checking
  its domain limit is reached. Each adapter returns admission rejection with no
  source registration or result consumption. Release those exact charges after
  the probe; no other thread's account is filled.
- Under that saturated sponsor, zero budget returns actual timeout without new
  timer admission. Existing 64 deadline-expiry probes still leave no completion
  or CQ waiter registrations.
- For each adapter, a same-LP pinned child observes the parent Blocked on the
  exact completion, sends a generation-bound unrelated wake and checks cancellation
  of the original subscription under a short IRQ-preserving mask. It then
  observes the parent registered/Blocked again before publishing result 42.
  Kernel polling returns the original buffer; the dispatcher returns READY/42.
  Both child exit watches complete before fixture cleanup, without extra
  abandoned roots, stacks or registrations.

Bounded one-millisecond sampling is confined to those private observation
fixtures. Production uses owned registration, with no readiness polling or new
retry loop. Existing EL0 ping-pong tests continue to execute real timed-wait
entry and result returns. Host tests retain their existing WAIT_ADMISSION_FAILED
owner-preservation and Drop/cancel/terminal-wait assertions. The new kernel
assertions run in guests; they are not counted as host unit tests.

## Initial accounting-probe failure retained separately

The first ARM guest used kernel SHA-256
`99994cf826ecbbd1328cf6b3111771c1e51fdfb2edf9a4e73a569e7b01d23336`.
At about 8.95 s it panicked at `self_test/cq_wait.rs:226:5` after both source
admission rejections returned the expected distinct errors:

```text
assertion `left == right` failed
  left: 4
 right: 5
```

This was the new fixture's timer-sponsor equality probe. Its baseline was
sampled without excluding local timer reclamation; the original preceding
64 timeout probes leave cancelled nodes whose charges follow actual backing
lifetime. A decreased aggregate snapshot is compatible with reclamation of an
older node and cannot establish a leak or refund by the new rejected wait.
The capture does not identify which older node was released. Its last reported
observation was 12 passed / 9 pending (`0x67fd` / `0x21239802`); no terminal
result was produced. Host HTTP/deployment probes timed out, and the runner
exited 1: `guest HTTP keyholes did not return both expected JSON views`.

A subsequent ARM run passed 21/0/0, but its paired Intel artifact
`5c02ea7431094907dd06dfa536c295efaa90470d311c3a8d843cef6d801b9fb5`
failed at `self_test/cq_wait.rs:273:9` around 19.52 s: after the new masked
watchdog-account probe, its observed count was **4**, baseline **3**. This did
not identify a retained event or prove a production leak. That fixture still
allocated/freed its vector inside a `LocalInterruptMask`; the kernel allocator
uses a blocking mutex, so the mask alone did not establish non-scheduling.
The shared helper also allocated observer/watchdog metadata beneath its own
setup mask. These source contexts require qualification independent of an
aggregate counter observation. The last waiting observation at 200.712320 s
was 12 passed / 6 pending (`0x22bfd` / `0x1219002`), without a terminal result.
The runner exited 1: `guest HTTP keyhole was not validated from the host`.

The final fixture removes ambient source-rejection equality snapshots. It
preallocates watchdog-charge storage outside its known rejecting mask, saturates
only counter owners, verifies admission rejection before observer allocation,
clears those exact counter owners, compares there and frees the vector after
mask exit. The helper separately moves fallible preparation before its own
publication mask and recomputes the original remaining budget afterward. No
production timeout extension, polling or forced IRQ enable is added.

The initial Intel artifact
`80f8ac64382d2a972791c79351a3b6c61f489f610a990092d676c22347c5ec44`
passed 18/0/0 at 24.502746 s with host controls, without reproducing the initial
snapshot race. The intermediate ARM artifact
`e7235ace4c3badf41ff8f270055080becf4a52f826571c74b84132568e817404`
passed 21/0/0 at 58.367552 s and host controls. These passes do not supersede
failed probes. Final validation rebuilds both kernels after the preparation and
fixture corrections; all samples remain separate below.

## Validation and artifacts

Both signed bundles were rebuilt; their ELF contents remain unchanged from the
preceding correction. Final host suite: **393 passed, 0 failed, 4 ignored
documentation examples**. Strict `-D warnings` Clippy passed for services and
both final-source kernel profiles. Formatting, diff and changed Markdown links
passed. Fresh-storage runners used the existing 180-second deadline and bounded
host ingress probes; no timeout was extended for this correction.

| Profile / revision | Result or last observation | Bitmap | Completion time | Runner |
| --- | --- | --- | --- | --- |
| Initial ARM, unsynchronized accounting probe | No terminal result; 12 passed / 9 pending | passed `0x67fd`, pending `0x21239802` | last observation 8.744508 s | 1 |
| Initial Intel, same source | 18 / 0 / 0 | `0x123bbff` | 24.502746 s | 0 |
| Intermediate ARM, first masked probe | 21 / 0 / 0 | `0x2123ffff` | 58.367552 s | 0 |
| Intermediate Intel, allocator-sensitive masked probe | No terminal result; 12 passed / 6 pending | passed `0x22bfd`, pending `0x1219002` | last observation 200.712320 s | 1 |
| Final ARM SMMUv3, security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | 57.009097 s | 0 |
| Final Intel VT-d, HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 19.078118 s | 0 |
| Final AMD-Vi, HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 22.956864 s | 0 |
| Final ARM shutdown ingress | 19 / 0 / 0 | `0x103ffff` | 11.056878 s | 0 |

All four final profiles emitted the assertion-backed timed-wait diagnostic and
passed the scheduled memory-close wake and later cancellation/IPC-readiness
checks. Their runners passed ingress controls, with existing bounded transport
retries where used. Shutdown requested PSCI power-off at 14.402212 s. These
passing observations do not supersede either failed probe or resolve historical
Intel failure causality. Synchronous assertions supplement deferred counts.

Final guest commands used `CATTEN_SKIP_EMBED_BUILD=1` after signed bundle builds:

```sh
scripts/run-aarch64.sh --security-test --http-test --dhcp-test --fresh-storage --timeout 180 --instance completion-wait-arm-qualified
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance completion-wait-intel-qualified
scripts/run-x86_64.sh --http-test --dhcp-test --iommu amd --fresh-storage --timeout 180 --instance completion-wait-amd
scripts/run-aarch64.sh --shutdown-ingress-test --fresh-storage --timeout 180 --instance completion-wait-shutdown
```

Final HTTP/deployment ports were 19360/18660, 19361/18661, 19356/18656 and
19357/18657. Initial runs used 19354/18654 and 19355/18655; intermediate runs
used 19358/18658 and 19359/18659 with the same profile arguments and instances
`completion-wait-arm-final` / `completion-wait-intel-final`. The initial instance
names were `completion-wait-arm` / `completion-wait-intel`. AMD alone used
`CATTEN_SKIP_KERNEL_BUILD=1`, after its source and current ELF matched the final
Intel per-boot hash. Immutable boot hashes precede later profile rebuilds:

| Artifact | SHA-256 |
| --- | --- |
| Initial failed ARM kernel | `99994cf826ecbbd1328cf6b3111771c1e51fdfb2edf9a4e73a569e7b01d23336` |
| Initial passing Intel kernel | `80f8ac64382d2a972791c79351a3b6c61f489f610a990092d676c22347c5ec44` |
| Intermediate passing ARM kernel | `e7235ace4c3badf41ff8f270055080becf4a52f826571c74b84132568e817404` |
| Intermediate failed Intel kernel | `5c02ea7431094907dd06dfa536c295efaa90470d311c3a8d843cef6d801b9fb5` |
| Final combined ARM kernel | `85b08a2101ece741794e0038c4103589ee5f8804e6f15ba5e386d4d24f850290` |
| Final Intel / AMD kernel | `55cba2a08487103756c6cf394409250c4b80de16a892a8b4a06c3d27912a39bf` |
| Final ARM shutdown kernel | `113b14050bb114f508dfc2c8325b5df0bb11cb560640cafdf5cbb2c2088a6c23` |
| ARM bundled service ELFs | `db645c0e1b527a6521fdfcd79dd7e498637b1a650024f8fbbeb82cb3c8b30f7a` |
| x86 bundled service ELFs | `0a4a369f13576fbddd304e76b0facc268ea31da31e9d50cd6707ced075bd9bb2` |

Bundle hashes use sorted newline-terminated `basename:ELF-sha256` lines hashed
with SHA-256. Relevant failure diagnostics, limits, results and artifact hashes
are inline; temporary logs or ignored artifacts are not report dependencies.

## Remaining limits

Historical Intel scheduled-close/ingress causality, earlier Intel DMA grant
failure and the lost AMD DMA-map cause remain unresolved. Fresh passes do not
establish reliability or retroactively explain a failed sample. Public DMA
readiness/caller IRQ qualification, arbitrary kernel-operation abandonment,
broader metadata/allocator/callback contexts and physical-platform evidence
remain separate SEC-18 acceptance work. No physical cleanup or root-release
permission follows from these wait outcomes or snapshots.
