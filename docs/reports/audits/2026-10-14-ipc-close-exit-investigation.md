# IPC close-worker exit readiness investigation

Date: 2026-10-14. Baseline: `63f83943` (current DNS registration/retraction).
This is a bounded native investigation and diagnostic improvement, not a proven
fix for the historical timeout. C18 and the IPC readiness finding remain open.

## Retained failure and attribution limit

The [registration batch](2026-10-14-protocol-simplification-dns-registration-relay.md)
recorded an x86 panic at **20.265305 s** in the scheduled endpoint-close fixture:

```text
assertion failed: crate::completion::wait_timeout(CLIENT, watched, 5_000).unwrap()
```

The fixture had registered a closer on a real cancellation claim, observed it
blocked, then completed that claim's real memory-loan revocation. The failed
wait required the worker's exact-generation exit. No contemporaneous worker
stage/live/staged snapshot survived. A later `FATAL: failed to persist Raft log
state` at **30.528249 s** and an ARM membership timeout at **133.951261 s** were
separate observations. They do not identify the earlier wait's cause.

The failed x86 kernel hash was
`6a4e6850df70c43a46c88eeb74500f84074d46b1b4e61a2c629a3a8f1743f617`.
The old failure remains unresolved; passing repeats cannot retrospectively
prove whether the close remained blocked, returned without exit notification,
or encountered scheduler/reaper delay.

## Source review and changed probe

Production already registers against the exact claim/epoch, rechecks its closed
list, parks through the checked scheduler and defers notification/disposal until
producer guards leave. Endpoint front waiting borrows its existing close owner's
server lease. The worker exit watch is generation-bound. Source review did not
establish a defect warranting a speculative production scheduler change.

The scheduled probe now runs **eight rounds** of both scalar-call close and
owned endpoint close on up to **four LPs**, starting with the producer's LP.
Four-LP guests exercise 64 distinct close episodes each; smaller guests exercise
16 episodes per selected LP. Larger machines select a bounded four-LP subset.
Each episode creates fresh original roots, confirms source registration and the
worker's blocked state, checks busy-root rejection, finishes the original claim
once, requires the original exact exit watch, and explicitly closes those roots.
The real endpoint case retains its queued memory loan through revocation.
A repeated episode is a new owned fixture, never replay of uncertain cleanup.

The probe retains an Interest clone for the exact admitted claim source until
exit observation finishes. That keeps the original list allocation/charge alive
for diagnosis even after registry detachment and deferred source disposal; it
creates no replacement source, owner registry or authority. Capture happens
under IPC before claim completion; the registry guard leaves before cleanup or
waiting. The final interest is released outside local guards. This does not
prove that every source/metadata/Drop context is qualified.

Timeout diagnostics include captured TID/generation, textual live ThreadStateKind,
affinity/pinned LPs, exact staged-retirement snapshot, worker-return marker,
worker stage, source closed state and registered count. Stage values are:
0 before worker entry, 1 entering close, 2 entering endpoint-front wait,
3 that wait returned, 4 close returned. Stage/return markers and absent snapshots
are observations only; none substitutes for exact exit or physical quiescence.
Closed-source readiness likewise grants no permission to retry loan cleanup.

The existing five-second registration and exit budgets remain. Kernel waiter
admission rejection stays an error, not a fabricated timeout/completion. No
watchdog saturation, sponsor replacement, forced IRQ enabling, added production
polling, physical cleanup or retry policy is introduced. The existing one-ms
registration sampling is a bounded fixture-only observation, not a claimed
production readiness path. Success logging now follows both close families.

## Native execution

Both linked pairs used fresh v4 stores, four LPs per guest, `--deploy-test`, and
a 300-second guest budget. A selector-backed coordinator started x86 after ARM
boot publication and confirmed the actual listener before x86 boot. Both pairs
finished with ARM/x86 runner exits **0 / 0**.

| Run | IPC close episodes | ARM | x86 |
| --- | --- | --- | --- |
| Initial same-LP/next-LP pass, TCP 19148 (QEMU listener PID 55987) | 32 per guest; producer/worker LPs 1→1/2 on ARM, 3→3/0 on x86 | IPC controls completed 9.328382 s; final **22/0/0** at 89.135614 s, bitmap `0xcfffff` | IPC controls 15.896071 s; final **20/0/0** at 41.220384 s, bitmap `0xcfbbff` |
| Final bounded four-LP pass, TCP 19149 (PID 60180) | 64 per guest, all four worker LPs | IPC controls completed 9.552644 s; final **22/0/0** at 94.960968 s, bitmap `0xcfffff` | IPC controls 20.819753 s; final **20/0/0** at 46.285613 s, bitmap `0xcfbbff` |

**192 total close/exit episodes passed; the historical timeout did not recur.**
Catalog replication, follower registration, remote invocation, endpoint retraction
and all three signed architecture-pinned deployment generations passed in each
pair. These results qualify the exercised boot contexts, not every scheduling
interleaving or a universal bounded retirement guarantee.

## Other checks and artifact identity

Host suite: **457 passed, 0 failed, 4 ignored**, plus the existing signing,
boot-input, secure-boot, syscall, tooling and object-store controls. Strict kernel
Clippy passed on both architectures with the native discovery/DNS/deployment/
cluster-control feature set. Workspace rustfmt and diff checks passed. Native
runners verified assembly section permissions. There is no userspace/protocol/
ABI change, so the current signed service bundles were reused.

| Artifact | Bytes | SHA-256 |
| --- | --- | --- |
| Initial ARM kernel | 62,447,984 | `bd7f884e7a678b9ed56517f056baca4696029919979a73e2fd671517fe97ddbf` |
| Initial x86 kernel | 59,362,256 | `0ad791cb8f0461e0f8cd953d88114383d8e95e188d99782bac205541cc20d5c2` |
| Final ARM kernel | 62,449,088 | `3c6938910d410fec8e478419dc88210ac58b8080143ebed108ef7acedd1074df` |
| Final x86 kernel | 59,360,920 | `a1c4481d117ae694f2085db21673647ff957513aef7e8f1dd690bf4cde6cb4c5` |
| Unchanged signed ARM DNS | 904,792 | `263797bd2e66bbd51db10d1b86131af1373ccf6911276c99ba5ac1d8a537e0fe` |
| Unchanged signed x86 DNS | 931,928 | `80c283996b2bb67f1b3b7052c34c7684c350a2b948ff06b1e1cb8e6ad5cd215f` |

Final kernel hashes matched the runners' captured payload hashes. Read-only v4
inspection verified each final guest's DNS object `0xfffe2f18f4769a5c` and matched
its complete bytes to the unchanged signed ELF. Evidence above is inline rather
than depending on temporary logs or ignored build artifacts.

## Remaining work

Keep the IPC timeout and subsequent persistence errors open. A future recurrence
now has source/phase/exact-generation diagnostics to guide the next investigation;
a passing repeat supplies no historical causal conclusion. No HTTP, S3, Kafka,
new admission-pressure suite or production trust qualification ran in this batch.
Wider reaper/physical-release, outer IRQ/guard, DMA and shared-CQ boundaries remain
separate. Internal API simplification can proceed independently, while this
regression remains enabled in the ordinary scheduled CQ fixture.
