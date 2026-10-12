# Weekly wait/readiness sweep — 2026-10-12

The sweep found new timer-driven reply polling and new cleanup waits that stay runnable while yielding. The reply polling is corrected in this change. Cleanup, reaper and shootdown notification gaps remain open; this is not a clean bill for all waits or closure of SEC-18.

## Scope and source evidence

Reviewed `6d458d7de093e4cb19b2efe0e2772de92365289a` through `c5e2ecbcd6e8c9a459f8dda5586e9c5eecdcc5b5`: 96 commits since October 5 and 255 changed Rust files. A broad added-line expression (`spin_loop|yield_lp|yield_now|sleep_ms|\.poll\(|spin_reply|poll_reply|while|loop`) produces 312 candidates across Rust and host scripts, including comments, imports and productive loops. The [checked-in classified inventory](evidence/2026-10-12-wait-readiness-candidates.json) preserves the original file/line/code at the reviewed head. Its SHA-256 is `f808adc051ea78064dbd444a8eb8ddf521aaec0e6efdf7703cdb4cf53b23bfa6`.

The current Rust scan of explicit spin/yield/short-sleep sites under `crates/` produced 226 lexical matches before remediation. That count includes declarations, comments, fixture code and yields after successful blocking. Added loops were inspected separately for hidden callbacks and `sleep_millis`, notably the new per-LP reaper. Host HTTP stream reads block and enforce framing/size/deadline bounds; the secure-boot runner deliberately samples its serial file/process every 200 ms under a timeout and reaps its VM. Neither is a production guest readiness loop.

This is a source sweep, not exhaustive reachability analysis or a CPU-utilization measurement. Grep counts are search coverage, not counts of bugs. Existing drivers, boot fixtures and admission-pressure fallbacks are not certified by a passing boot.

## Corrections

- `clusterctl_client::observe` and `publication::register` introduced 5/10 ms reply polling this week. The kernel already had `PendingCallObservable` and a timed registration/watchdog helper, but EL0 lacked a timed reply syscall. New syscall **86, `IpcReplyWaitTimeout`**, exposes that machinery through `PendingCall::wait_timeout(&mut self, timeout_ms)`. It parks on the exact call, closes the registration race with a readiness recheck, and cancels competing subscriptions/watchdogs after wake. Readiness is observed through the existing result-publication path; returned owners are adopted once.
- `scheduler::try_block_until` now distinguishes failed source/watchdog admission from expiry. `ipc::wait_reply_timeout` returns `ResourceLimit` on admission rejection; syscall 86 reports `RESOURCE_LIMIT`. Status 1 means timeout. Both leave the pending capability and any Rust memory borrow owned. Existing `block_until` retains its explicitly fail-soft boolean contract for controllers that need only a condition outcome. No polling fallback was added to the new timed reply API.
- The older `grant_client` acquire/publish reply loops use the same owned wait. Deployment and grant calls retain their original total deadline; the new remaining-budget conversion rounds up fractional milliseconds without restarting admission. Publication retains its separate five-second observation budget. Application timeout/error drops its pending owner through existing cancellation; failed close still aborts the domain rather than ending an unsafe borrow. No accepted request is replayed and no remote rollback is inferred.
- `node_identity::Store::call_move` still retried obsolete opaque `CreationFailed` despite the checked move ABI. It now retries only exact `QUEUE_FULL`, retaining the returned request owner and its existing ten-second admission budget. Its accepted RPCs already use event-driven untimed waits; bounding them is a separate policy question.
- The AArch64 syscall-86 emitter passes the timeout in x2 and returns all four result registers. Reply-wait emitters do not assert `nomem`: borrowed buffers can change while parked. The emitter inventory checks 88 wrapper sites across nine arities.

`AGENTS.md` now requires readiness registration where available, one operation deadline, exact ownership through timeout/rejection, lost-wakeup protection, post-guard notification, preservation of IRQ policy, and honest documentation of polling exceptions. It warns against blocking a service on one reply when that service must process other sources to produce it, or using a CQ that does not receive reply notifications.

## Remaining findings and justified distinctions

| Source/context | Observation | Required next boundary |
| --- | --- | --- |
| `ipc::close_cap_with_cleanup`, `PreparedEndpointClose::finish_with` | Newly introduced close paths release IPC and yield/recheck completing tokens, minting-source claims or claimed queue fronts. Final reply notifications do not cover ordinary claim rollback/release. | Owned registration on exact claim transitions, including rejection and rollback; notify after unlock. Preserve complete cancellation/endpoint owners, roots and uncertain loan pins. Do not wait for a reply that need never be published. C08/C09 remain open. |
| `memory::object::close_cap_with_wait` | New yield/recheck waits for source escrow, transfer fences, DMA/copy pins and loan cleanup. | Exact pin/fence release notifications under the captured object/generation; preserve current registry/lifecycle order and backing owner. CPU readiness cannot substitute for DMA completion. C05/C18 remain open. |
| `ClosingAddressSpace::wait`, `teardown_domain`, `node_shutdown_worker` | New/modified owners are polled via short sleeps or runnable yields. Namespace cleanup returns Pending correctly but its outer controller has no complete lease/claim/pin drain subscription. Terminal shutdown-failure loops are separate retention paths. | Compose exact thread-exit, cooperative acknowledgement, cleanup-lease and subsystem readiness without allocating teardown snapshots or losing the closing fence. C02/C10/C11/C12 remain open. |
| `threads::start_reapers` | New same-LP workers wake every millisecond to scan retirement work. | Notify pinned workers when work becomes eligible, with registration/recheck protection. Preserve same-LP off-CPU proof and physical stack release outside IRQ tails. C16 remains open. |
| x86 `try_send_sync_shootdown_with` | This week's replacement yields while coordinator admission is busy and spins while waiting for exact epoch acknowledgements. Both are bounded, but neither registers readiness interest. | Qualify coordinator availability and epoch-completion subscriptions in the real caller/IRQ contexts. Keep stale/failed IPI rejection and physical-retirement ownership; no interrupt enabling beneath an unknown mask. C04 remains open. |
| VT-d `Commands::wait_idle`, QEMU NVMe reset | Added bounded register-status polling: one million iterations and 100 ms respectively. No implemented completion-interrupt subscription covers these status transitions. | Hardware polling is an explicit exception, not evidence of event-driven operation. Preserve complete hardware ownership and caller IRQ policy; timeout cannot release reachable backing. |
| Listener submission, endpoint publication admission, socket close admission, identity content publication | Queue-full retries are spaced and bounded, but queue capacity and object-content publication lack subscription protocols. | Add protocol notification if removing backoff; a connection-close watch or reply wait is not a queue-capacity/content event. Retry only unaccepted operations. |
| `grantctl` and security probe multi-source reactors; existing service/driver polling | Preexisting reactors must process their own endpoint while outstanding calls complete. Pending replies do not currently signal their CQ. Existing boot/status, network/socket and driver polling remains outside this week's corrections. | A composite ready set or actual reply-to-CQ wiring is needed before replacing their polls with one blocking wait. Review each producer contract; do not deadlock the sole producer. |
| Existing untimed IPC/completion waits and timer sleeps under admission pressure | Ordinary paths register interest. Failed waiter/timer admission can deliberately leave callers runnable while retrying, preserving a live borrow or void wait contract. | Keep rejection observable and ownership safe. These pressure fallbacks are not evidence that normal operation spins, nor a reason to add poll loops to fallible timed APIs. |

The inventory classifies 172 fixture matches, 55 non-executable matches, 36 productive matches, 13 backoff matches, nine cleanup/admission gap matches, six corrected reply matches, six terminal-retention matches, four timer-poll gap matches, three hardware-poll matches, three nonblocking progress matches, two host fixture polls, and one each acknowledgement gap, single readiness check and blocking host read. Counts are added-line classifications; the table also records enclosing wait contexts and preexisting boundaries not represented by an added yield line.

## Subsequent scoped correction

The [reply CQ follow-up](2026-10-12-reply-cq-readiness.md) implements owning
pending-reply CQ interests and exact queue-identity validation, then converts
`grantctl` and the security probe's self-serving/idle reactor. Checked CQ waits
use an owned kernel watchdog for the remaining deadline, with explicit wait
admission rejection and no extra completion capability. Exact-root lifecycle
publication signals default-CQ work. The table above records the original
reviewed-head findings; cleanup/reaper/shootdown and the other protocol/driver
notification gaps remain open.

## Validation and reproducible evidence

Both complete signed service bundles were built before guest execution using `scripts/build-catten-services.sh --embed` and `scripts/build-catten-services-x86_64.sh`. All four guest runs exit 0:

| Configuration | Passed / failed / pending | Passed bitmap | Guest completion |
| --- | --- | --- | --- |
| ARM security/HTTP/DHCP, fresh disk | 21 / 0 / 0 | `0x2123ffff` | 57.574896 s |
| Intel HTTP/DHCP, fresh disk | 18 / 0 / 0 | `0x123bbff` | 21.411154 s |
| AMD HTTP/DHCP, same x86 kernel, fresh disk | 18 / 0 / 0 | `0x123bbff` | 19.090547 s |
| Separate ARM shutdown ingress, fresh disk | 19 / 0 / 0 | `0x103ffff` | 10.147842 s |

ARM's second signed probe reports publication full-queue deadline, silent-call cancellation and recovery of all eight slots at 46.875073 s; scalar/move deadlines, rejected/cancelled/returned page charges and healthy status pass at 57.011767 s. The probe exits with retired policy fenced and publication generation 2 at 57.569123 s. These execute syscall 86 from EL0, including accepted unanswered calls and normal returned-memory responses, rather than merely exercising the kernel helper. They do not inject a physical loan cleanup failure.

HTTP/deployment/node/cluster recovery controls pass on ARM, Intel and AMD. Trickled deployment bodies reject and recover in 30.14/30.21/30.16 host seconds respectively. Expected reset/timeout retry diagnostics remain visible and healthy controls succeed. Shutdown acknowledges ObjectStore at 14.510846 s and requests PSCI power-off at 14.523484 s; this validates normal drain, not uncertain hardware cleanup.

Host suite: **381 passed, zero failed, three ignored**. New tests retain a borrowed-memory call through timeout and wait rejection, then observe and clean returned connection/memory owners exactly once; dropping a timed-out owner cancels the exact call. Deadline tests cover fractional conversion, expiry, invalid clock and extreme values. Kernel scheduled fixtures perform 64 real reply timeouts without retained subscriptions and force waiter-source saturation: a ten-second timed wait rejects immediately with `ResourceLimit`, leaves the call pending and existing registrations intact, and preserves IRQ state. These fixtures run in all guest profiles above.

The first host attempt failed to compile the two new tests because they used the nonexistent `call_borrow` method. Correcting them to `call_borrow_read` yielded the passing full repeat; this was a test-source error, not a guest failure. No failed guest run occurred in this sweep. Strict service and ARM/x86 kernel Clippy, workspace formatting, syscall emitter coverage and diff whitespace checks pass.

Kernel SHA-256: ARM combined `a3397a700cec0074afff090dfbce8e23f2b0f86403490cad8aef726b7e7541e0`; Intel/AMD `972c11ec4845bae280e31fbd58cbdeaf0feb2d648f4f2c13345295166c1c5350`; ARM shutdown `91fc34c5fc07ddc0149fb4f0d2181c6cbe691da69136c92b78333dc7fc75b23b`. Sorted `filename:ELF-digest` bundle hashes: ARM `1a905affb17abcbc9fa1f77a44c46eac11b708965c06fb372804dd8fc9d8d48d`; x86 `06b464edfa1c0d54f1ee4eea257d200faa2789c9c87eab2d15ed95c4fd6f55c7`.

Reproduce host checks with `scripts/run-host-tests.sh`. ARM combined uses `--security-test --http-test --dhcp-test --fresh-storage --timeout 180`; x86 uses `--http-test --dhcp-test --iommu intel` or `amd --fresh-storage --timeout 180`; shutdown uses separate `--shutdown-ingress-test --fresh-storage --timeout 180`. Give concurrent guests distinct instances and forwarded ports. Reuse a kernel only for identical profile/bundles. Static candidates can be regenerated with `git diff --unified=0 BASE HEAD -- '*.rs' '*.c' '*.h' '*.S' '*.s' '*.py'` and the expression above, preserving added-line numbers.

Normal passes establish operation and ownership behavior in these fixtures. They do not prove every wait is registered, measure production CPU savings, or resolve the historical Intel object-store read rejection or AMD NIC startup failure.
