# Deferred delivery before park publication

Date: 2026-10-14. Baseline: `c55a4b9`. Scope: C16/C18 readiness delivery;
no protocol, runtime ABI, custody or physical-release change.

## Defect and attribution limit

The preceding [IPC exit investigation](2026-10-14-ipc-close-exit-investigation.md)
reproduced exact-generation exit timeouts on ARM and x86. Workers returned near
the five-second exit deadline. A closed source, returned marker, absent table
entry or staged node does not prove exit/quiescence or authorize backing release.

Source review found a distinct ordering gap: IPC claim completion queues its
admitted callbacks on the producer LP after subsystem guards leave. A timed or
event-only wait could publish its producer as Blocked, restore IRQs and be
preempted before `yield_lp` drained that queue. A dependent close worker could
therefore need delivery from the producer that just parked waiting for its exit.
Other scheduler/idle boundaries can drain the queue, but the park boundary must
not depend on reaching them or on the exit watchdog firing.
This explains a possible delayed-wake interleaving; no captured trace identifies
it as the cause of either historical timeout or subsequent Raft persistence error.

## Correction and ownership boundary

Guard-free timed/event-only park setup now takes an IRQ-state-preserving local
publication mask and drains the current LP's existing transport before waiter
registration or Blocked publication. No scheduler, thread-table or subsystem
guard is held across callbacks. Timed setup recomputes the remaining original
absolute deadline after delivery. Sleep flushes after its prepared event and
before publication under its existing mask. Untimed completion waiting uses the
same setup and now masks publication/recheck until yield. Caller-owned masks
remain masked; setup leaves before any yield.

Transport/source allocations and charges remain with their original owners.
No fresh registry, producer, interest or authority is introduced. Delivery remains
readiness/metadata work; it does not release stacks, relax loan fences, complete
uncertain teardown, replay accepted IPC or authorize a physical retry. Blocking
lock acquisition beneath other guards and wider allocation/Drop contexts remain
separate qualification work, rather than gaining blanket dispatch permission.

## Deterministic negative control

The ordinary scheduled CQ fixture stages an admitted precursor callback, then
requires delivery before timed/event-only waiter registration. The callback
reenters the thread table and requires the producer still Running and locally
masked. Waiter rejection remains `WaitRegistrationFailed`; the producer stays
runnable, source sponsorship returns to zero and nested IRQ state is preserved.
Callback/list storage is admitted before the fixture mask, without saturating a
live sponsor or supplying a replacement wake. Both original and nested masks
are checked; no IRQs are force-enabled.

A bounded ARM four-LP negative run temporarily omitted only the new setup drain.
It failed before wait admission with:

```text
panicked at crates/catten/src/cpu/scheduler/deferred.rs:148:13:
assertion `left == right` failed: park stranded producer delivery
  left: 0
 right: 1
```

The runner exited 1 without an authoritative suite result within its 35-second
budget. Negative kernel SHA-256:
`4aa4ee91447b57b93dc5d2f66d2cdf48623c53faf2c16ef5730480ca450ba041`.
The omitted drain was restored before corrected builds. This control detects
the ordering defect; it does not reproduce the historical IPC timeout itself.

## Corrected native execution

The first corrected mixed pair used fresh v4 stores, four LPs per guest and
`--deploy-test`, with a 300-second guest budget. The coordinator registered
stdout interest, started x86 after ARM boot publication and verified the actual
TCP 19158 listener (QEMU PID 19297). Both runners exited **0 / 0**.

| Control | ARM | x86 |
| --- | --- | --- |
| Timed/event-only precursor ordering, nested masks and rejected admission | passed 7.881633 s | passed 14.086852 s |
| Original exact-generation IPC close/exit watches | all 64 episodes passed 9.524105 s | all 64 episodes passed 15.513870 s |
| Authoritative suite result | **22/0/0** at 91.695385 s; bitmap `0xcfffff` | **20/0/0** at 43.061661 s; bitmap `0xcfbbff` |

Both guests passed catalog replication, full-name lookup/invocation/retraction
and all three signed, architecture-pinned rollout generations, including
cooperative and forced retirement. The historical IPC exit timeout and later
Raft persistence failure did not recur in this pair. That observation does not
identify their historical cause or prove a universal bounded exit guarantee.

The six reused NS/DNS/Echo ELFs were independently verified before launch:
NS Bootstrap, DNS/Echo Service, with no blanket manager grant. They are unchanged
from the [NS generation batch](2026-10-14-protocol-simplification-name-generations.md),
which retains their complete SHA-256 identities. No signing/build operation was
left pending when corrected guests started. Native runners also verified native
assembly section permissions (1 ARM and 249 x86 entries).

| Executed payload | SHA-256 |
| --- | --- |
| First corrected ARM kernel | `28cec3fac53983ca83631b44501656fd7957eb8f57e7aeb8e286c7c91b2d156f` |
| First corrected x86 kernel | `0677bfcc7cbfb52afdda29d99aea5c1a085df7e4a6e0dcd4f02edf000d666932` |

Strict kernel Clippy passed on both architectures with the exercised
discovery/DNS/deployment/cluster-control feature set. Workspace rustfmt and
diff checks passed. No host-testable library or userspace source changed.

Markdown policy/reference/audit evidence and the LaTeX scheduling chapter were
updated together. The manual rebuilt through two successful LaTeX passes;
changed pages 73/74 were rendered and visually inspected. The 214-page PDF
SHA-256 is `965192993a7d4cbb0d3a6177feb1186f88bd6006fd439b30348ca1ff037bcf46`.

## Unchanged-payload fresh-store repeat

A second pair used TCP 19159 (actual QEMU listener PID 23178), new fresh v4
stores and the same four-LP/300-second guest setup. Both executed kernel hashes
matched the first corrected pair byte-for-byte. Both runners again exited **0 / 0**.

| Control | ARM | x86 |
| --- | --- | --- |
| Precursor ordering, masks and rejection | passed 7.697338 s | passed 13.789001 s |
| Exact-generation IPC close/exit watches | all 64 episodes passed 9.292188 s | all 64 episodes passed 15.215039 s |
| Authoritative suite result | **22/0/0** at 91.360578 s; bitmap `0xcfffff` | **20/0/0** at 42.589213 s; bitmap `0xcfbbff` |

Both guests again completed full-name DNS controls and all three rollout
generations. Across the corrected pairs, **256 real close/exit episodes passed**;
no historical exit-watch timeout or later Raft persistence failure recurred.
The deliberately failing negative run remains separate evidence. This repeat
qualifies the exercised contexts, not all interleavings or historical causality.

## Remaining boundaries

Historical timeout/persistence attribution remains open, with failed captures
preserved separately from passing repeats. Exact readiness still grants no
physical cleanup or retry authority. Wider reaper/constructor/syscall masks,
metadata/Drop allocation, blocking-lock nesting, DMA and shared-CQ qualification
remain separate. Third-party NS endpoint-death notification still requires its
own producer/authority contract. No HTTP/S3/Kafka or production trust
qualification is claimed by this batch.
