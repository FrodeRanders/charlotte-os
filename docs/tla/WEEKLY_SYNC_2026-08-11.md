# TLA+ / implementation sync — 2026-08-11

## Review window and method

This pass compared the TLA+ baseline at `0664d0c` (2026-08-02) with the
implementation changes from 2026-08-04 through `b183598` (2026-08-10). That
slightly wider-than-seven-day window includes the complete signed-deployment,
distributed catalog, dynamic Raft admission, wait-path, scratch-mapping, and
reliable-message changes rather than cutting related work at midnight.

The review followed each changed implementation path already named by
`CONFORMANCE.md`, classified whether its abstract state or only its mechanics
changed, and then ran TLC with required-action coverage. Negative models are
retained when a repaired behavior has a small, useful counterexample.

## Impact by model

| Model area | Relevant weekly change | Result |
|---|---|---|
| Service lifecycle | Mandatory Ed25519 loader gate; persistent signed artifacts; replicated prepare/activate; inactive tombstones; generation-fenced automatic unregister; asynchronous local cleanup | Material model drift. `CharlotteServiceLifecycle` now includes trust staging/rejection and the actual two-phase catalog state machine. The old atomic replacement claim was removed. |
| Raft membership | MAC-level dynamic join, committed JOIN fence, catch-up before promotion, selected-anchor restriction, batched joiner promotion | Material new state machine. Added `CharlotteRaftJoin`, then extended it with durable admission crash/restart after the review exposed a volatile-state gap; joint consensus remains in `CharlotteRaftMembership`. |
| Reliable messaging | v2 restart sessions, retry-session abandonment, bounded retired sessions, fragmentation and bounded queues, detached retransmit clock | Material new safety state. Added `CharlotteReliableMessage`; fragmentation/queue/timer mechanics remain documented omissions. |
| Completion queues / scheduler | EL0 thread-exit observation, generation-fenced supervisor wait, event-driven live-upgrade waits, detached timer watchdogs | Existing abstraction remains valid. Thread exit is another source of the generic completion transition; the scheduler reaping boundary is unchanged. Conformance mapping was updated. |
| Endpoint IPC | Waiters now re-check their own call after unrelated observable wakes; endpoint close watches remain separate from readiness | Existing endpoint-observer and CQ models already require the re-check/lost-wake discipline. No TLA transition change. |
| Address spaces | `memory_map_any`, kernel scratch windows, ASID-generation-keyed cursors, mapping/teardown serialization | Placement mechanics changed, ownership did not. `CharlotteAddressSpace` remains the relevant generation-identity model; conformance now maps the scratch cursor explicitly. |
| Hardware ASID / interrupt routes | AArch64 interrupt delivery and LP routing hardening | No abstract tag-reuse or route-generation change. Existing models remain applicable. GIC/MPIDR encoding is below their boundary. |
| Raft election/log/snapshot | Direct Ethernet framing, exact payload lengths, snapshot-response correlation, heartbeat cadence, persistent DNS stores | Core safety transitions remain aligned. Transport correlation and timing are not silently attributed to the Raft safety models. |
| Remote calls | Causal application barrier and transport plumbing changes | Identity, generation fencing, uncertainty and safe dedup eviction are unchanged. `CharlotteRemoteCall` remains aligned. |
| IPC memory / capability / DMA | Scratch-address migration and error-path handle cleanup | No abstract transfer, namespace, or DMA pin-lifecycle change found. Existing models remain aligned. |

## Findings reflected into code

1. **Retry/restart session collision.** The implementation initialized a
   service instance with `session = generation`, then incremented that value
   when an uncertain send was abandoned. Generation N's first retry therefore
   had the same identity as generation N+1's initial session. The retained
   unsafe TLC configuration reaches this collision through `AbandonSession`
   followed by `RestartService`. The wire helper now packs service generation
   and retry epoch into disjoint 32-bit fields.

2. **Receive-session regression after the retirement window.** A bounded list
   of retired sessions prevents only bounded-delay replay. An older SYN can be
   accepted after it falls out of that list, resetting receive sequence state
   backwards. The second negative configuration demonstrates the regression.
   Receivers now accept only a well-formed session strictly newer than the
   current packed identity; delayed older sessions are rejected without a
   retirement window.

3. **Generation exhaustion violated the model's freshness guard.** The
   distributed catalog used `saturating_add`, which reuses the maximum
   generation forever, while the local catalog used unchecked signed
   addition. Either behavior defeats generation-fenced stale cleanup at the
   numeric boundary. Both paths now fail closed: distributed prepare returns
   the existing protocol failure generation zero without changing state, and
   local registration returns an error and closes the unpublishable incoming
   connection.

4. **Publication was modeled too atomically.** The old lifecycle spec claimed
   a replacement remained continuously resolvable across one publication
   linearization point. Current DNS intentionally commits an inactive prepare,
   publishes locally, then commits activation; a replacement is temporarily
   absent from lookup. The model and conformance documentation now state that
   behavior instead of overstating availability.

5. **Dynamic admission could forget its authority fence on restart.** The
   selected anchor and joining posture were process-local. CharlotteOS now
   persists the anchor plus the pre-admission snapshot index before accepting
   the anchor's log, restores election suppression on restart, and snapshots
   admitted membership before clearing the fence. Auto-join is disabled when
   the Raft service is using volatile stores. The join model now checks safe
   crash/restart recovery and an explicit forgetful-restart counterexample.

## Explicit remaining gaps

- Relmsg fragmentation bounds, retransmission liveness, and transport queue
  scheduling are tested implementation concerns but are not proved by the new
  session-identity model.
- The models remain bounded safety specifications. They do not prove Rust
  refinement, network liveness, cryptographic correctness, or fault tolerance
  under arbitrary partition/reordering.

## Validation

- `docs/tla/check.sh` completed all 17 fast configurations with required-action
  coverage. The updated lifecycle model explored 74,232 distinct states; the
  focused join model 520; and the reliable-message session model 21.
- All 10 negative configurations produced their named invariant violation and
  exercised the required unsafe action. These include stale catalog
  unregister, arbitrary join-anchor replication, forgotten restart admission,
  flat session collision, and receive-session regression.
- `catten-graft` passed 24 host tests, including restart-fenced join admission,
  selected-anchor rejection, snapshot correlation, and membership recovery.
- `charlotte-protocol-msg` passed 6 host tests, including disjoint ordered
  retry/restart identities and fail-closed session exhaustion.
- The changed `ns`, `relmsg`, and `raft` services built and passed Clippy with warnings
  denied for the AArch64 target. Workspace formatting, shell syntax, and diff
  whitespace checks passed.
