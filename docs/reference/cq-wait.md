# Checked completion-queue wait

Runtime ABI 3 has one CQ wait: syscall 88 (`CqWaitChecked`) and SDK
`cq_wait_checked(timeout: Option<u64>, cq)`. Unchecked 12/42, `cq_wait`,
`cq_wait_timeout`, `wait_on_cq` and `wait_on_cq_timeout` are removed. Their
ignored `min_complete` parameter and boolean timeout/admission fallback are gone.
Rebuild/re-sign all variants; ABI 1/2 requirements reject on ABI 3 nodes.

| Register | AArch64 | x86_64 | Contract |
| --- | --- | --- | --- |
| Input timeout | x1 | rdi | Milliseconds for a finite wait; zero for event-only |
| Input queue | x2 | rsi | Exact u32 queue ID |
| Input timed flag | x3 | rdx | 1: finite timeout, 0: event-only |
| Output count | x0 | rax | Pending ring entries; zero can still mean a peer/endpoint wake |
| Output status | x1 | rdi | 0 readiness, 1 deadline, 2 invalid queue/request, 3 admission rejection |

There is no timeout sentinel. Some(u64::MAX) remains a finite request; None
encodes timeout zero/flag zero. Invalid flag, a nonzero event-only timeout or
wide queue rejects before registration. The safe runtime `catten_rt::cq::wait`
returns `WaitOutcome::{Ready, Deadline}` or `WaitError::{AdmissionRejected,
InvalidQueueOrRequest, InvalidStatus}`. An event-only deadline status is invalid.

The kernel captures its exact CQ list identity, flushes backlog, and registers
an owning source subscription. Finite waiting uses the existing checked scheduler
absolute deadline/watchdog loop; unrelated wakes recheck and repark with the
remaining budget. It cancels unused interests and revalidates queue identity
before consuming a work generation. No ring occupancy threshold, one-wake timer
adapter or uncharged legacy watchdog remains. Producer and outer IRQ/context
qualification remains as published; reusing this helper is not a universal
allocator/IRQ proof.

Readiness is a hint, not result observation, attachment adoption or physical
quiescence. Preserve every pending operation and borrowed buffer on timeout or
rejection. `catten_services::reactor::wait` gives ordinary reference reactors a
shared explicit policy: log and abort the domain on wait rejection. It cannot
spin, invent a protocol clock tick or replay an accepted request. Grant control
and security fixtures retain their specialized bounded admission policy.
Its timer-backed `sleep_ms` pause also aborts on submission/wait rejection or
non-successful timer completion; failed pause admission cannot create a hot
retry loop. The buffer-free timer retains its existing owned cancellation/close
policy, and the caller's pending transaction remains live through domain abort.
Controller idle waits are event-only: both bound endpoints and kernel lifecycle
publication signal their default CQ.

Periodic Raft/discovery/retransmission watchdogs and bounded hardware polling
remain explicit timed policies. Raft, DNS, discovery and reliable-message
watchdogs retain their original deadline across ordinary readiness; only an
actual clock event/watchdog expiry starts their next period. This removal does not create missing hardware
or reply notifications. Telemetry archive and other already-recorded polling
boundaries still require their own source contracts.

The sibling Sitas backend uses syscall 88 on both architectures. Its reactor and
sleep keep one monotonic budget; peer/completion wakes cannot reset it or finish
sleep early. Rejection aborts the domain because its portable executor retries
backend errors and its parker/sleep traits have no error channel. Private shard
reactors can wait event-only. Public shared parkers retain a 5 ms backstop: a
competing CQ reader can consume its shared work-generation cursor. This remains
timer recovery, not a per-waiter readiness guarantee. That notification boundary
must be qualified before removing the backstop.

CI pins every Sitas checkout to published commit
`d27f7e7b7a6e3657c9b39e53da02cf0cd8f91a78`, the checked-wait update. The prior
`94eca685` pin still emitted retired syscalls 12/42 and is incompatible with
runtime ABI 3, even if compilation and ELF signature validation succeed.
`scripts/tests/test-ci-runtime-inputs.py` checks the pinned backend against the
SDK wait number and rejects that earlier source. Developers' sibling checkout
is `../../gautelis/sitas`; changing CI's pin does not modify that working tree.

CI supplies its warning policy through `CARGO_BUILD_RUSTFLAGS`. A global
`RUSTFLAGS` replaces Cargo's target-specific flag set and would remove the
kernel's required frame pointers. Both kernel targets retain denied warnings
and frame pointers explicitly; userspace keeps its existing code generation.
This preserves panic diagnostics without widening the SITAS verifier deadline.
