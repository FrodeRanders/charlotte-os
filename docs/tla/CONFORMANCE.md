# TLA+ to Rust Conformance Map

This document records which CharlotteOS implementation behavior each TLA+
action represents. “Direct” means the action follows the current Rust state
transition while omitting data irrelevant to the checked invariant.
“Abstract” means several concrete operations are deliberately collapsed.

This is a reviewable correspondence map, not a refinement proof.

## IPC

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `MemoryCreate` | `memory::object::create` and capability insertion | Abstract: frames, sizes, rights and mappings are omitted. |
| `EndpointCreate` | `ipc::create_endpoint` | Direct for owner, capacity, open state and endpoint capability. Interface/version and CQ notification binding are omitted. |
| `ConnectionMint` | `ipc::mint_connection` and `mintable_endpoint` | Direct for endpoint identity and attenuated rights. |
| `ScalarSend` | `ipc::scalar_send`, `enqueue_scalar` | Direct for authorization, closure, queue capacity and enqueue. |
| `ScalarCall` | `ipc::scalar_call` | Direct for pending-call creation, internal reply-token creation and enqueue. The token identity remains kernel-internal while queued; `receive` installs the server-visible one-shot capability. |
| `ScalarCallMove` | `ipc::scalar_call_with_memory_move`, `memory::object::move_to` | Abstract: one attachment only; concrete rollback and mapping checks are omitted. |
| `ScalarCallCopy` | `ipc::scalar_call_with_memory_copy`, `memory::object::copy_to` | Abstract: contents and allocation failures are omitted. |
| `ScalarCallBorrowRead` | `ipc::scalar_call_with_memory_borrow_read`, `memory::object::lend_read` | Direct for distinct borrower, shared readers and reply-bound revocation. Rights and mapping checks are omitted. |
| `ScalarCallBorrowWrite` | `ipc::scalar_call_with_memory_borrow_write`, `memory::object::lend_write` | Direct for distinct borrower, exclusivity and reply-bound revocation. Rights and mapping checks are omitted. |
| `Receive` | `ipc::receive`, `install_reply_cap` | Direct for authorization, FIFO dequeue and installation of receiver-visible reply authority. Vector attachments are omitted. |
| `Reply` | `ipc::reply`, `complete_reply` | Direct: validates and consumes the token, revokes any borrow, records the result and wakes observers. |
| `ReplyReturnMemory` | `ipc::reply_with_memory_move`, `complete_reply`, `memory::object::move_to` | Abstract: connection return and concrete rollback ordering are omitted. |
| `CancelPendingCall` | `ipc::close_cap` for `PendingCall`, `cancel_queued_call` | Abstract: closing the cap removes the pending record in Rust; the model retains a completed record for invariant inspection. |
| `EndpointClose` | `ipc::close_cap` for `Endpoint`, queued-message cleanup | Abstract bulk transition over queued calls, capabilities and borrows. |
| `DomainTeardown` | `ipc::close_address_space`, repeated `close_cap`, `memory::object::close_address_space` | Abstract bulk transition. Concrete teardown is distributed across registries and calls. |
| `ObserveResult` | `ipc::poll_reply` | Direct for the unobserved-to-observed result transition. |

### IPC concrete-to-abstract state

| Model state | Concrete source |
|---|---|
| `capTable[asid][cap]` | Unified `capability` namespace plus the matching entry in `IpcRegistry::caps` or the memory-object capability table. |
| `endpoints` | `IpcRegistry::endpoints`; observers, interface/version and `notify_cq` are hidden. |
| `replyTokens` | `IpcRegistry::reply_tokens`; `MemoryBorrow` is reduced to a memory-object ID. |
| `pendingCalls` | `IpcRegistry::pending_calls`; `None` maps to `NoResult`, `Some` maps to a result record. |
| `memObjects` | Memory-object owner and `LendState`; mappings, rights, physical frames and DMA pins are hidden. |

Call creation queues only an internal token identity. `receive` atomically
turns that identity into a capability in the endpoint owner's table. The
model's `delivered` bit records this linearization point; a live token has a
receiver-visible capability exactly after delivery. Cancellation and teardown
invalidate either queued identities or already delivered capabilities.

## Endpoint observers

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `ArmReadiness` | `EndpointObservable::register_observer` | Direct: installs a waiter in `Endpoint::readiness_observers`. |
| `ArmCloseWatch` | `watch_connection_closed` | Direct: installs a completion observer in `Endpoint::close_observers`, with an atomic already-closed check. |
| `Send` | `enqueue_message` | Direct for the empty-to-readable event: drains readiness observers only. |
| `Receive` | `receive` | Abstract: payload and authorization are omitted. |
| `Close` | endpoint branch of `close_cap` | Direct: marks the endpoint closed and drains both observer classes. |
| `ObserveClose` | `completion::poll` | Abstract: consumes the completed close-watch result. |
| `UnsafeMessageWake` | former shared `Endpoint::observers` queue | Negative model only: message arrival drains a close watcher and violates `CloseSignalImpliesClosed`. |

The kernel IPC self-test arms a close watch, sends and receives an ordinary
message, requires the watch to remain pending, then closes the endpoint and
requires the same watch to complete. This is the authoritative concrete trace
corresponding to the repaired model and its retained negative counterexample.

## Completion queues

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `OpenCq` | completion address-space/CQ attachment | Abstract: allocation and shared mapping are omitted. |
| `SubmitNoBuffer` / `SubmitWithBuffer` | `completion::submit`, `Completion::new`; `catten_rt::owned::ReadOperation::submit` | A successful submission begins in `InFlight`. Buffered submission also transfers the mutable borrow to the in-flight operation (`kernelOwnsBuffer`). |
| `Complete` / `Fail` | `completion::complete`, `Completion::complete`, `post_to_cq`; `ReadOperation::wait` | Direct for the terminal transition, CQ publication, generation increment and notification. Only this terminal boundary releases the kernel's buffer loan. |
| `CancelOp` | `completion::cancel`, `Completion::cancel`; `ReadOperation::drop` | Direct: `InFlight` becomes `CancelPending`; no CQ entry is posted and the buffer remains loaned until the later terminal completion. The safe wrapper cancels, waits, and closes before its Rust borrow can end. |
| `DrainOne` / `DrainAll` | userspace ring drain plus kernel `flush_backlog` | Abstract: shared-memory head/tail mechanics and batching details are collapsed. Marks CQ delivery consumed, not the capability result observed. |
| `ObserveResult` | `completion::poll`, `Completion::take` | Direct: `Completed` becomes `Observed`, independently of CQ draining. |
| `CqWait` | `completion::wait_on_cq` and timeout variant | Abstract: scheduler blocking and the post-registration lost-wake recheck are represented by one atomic waiter transition. |
| `CqWake` | `completion::wake` | Direct for generation increment and observer notification. |
| `SubmitTimer` | `completion::submit_timer` and timer observer registration | Abstract: deadline values and scheduler timer queues are omitted. |
| `TimerFire` | `CompletionTimerObserver::notify`, `completion::complete` | Direct for first-winner completion and cancellation-result forcing. Fairness of timer delivery is not checked. |
| `Complete` for an exit observation | `observe_thread_exit_with_generation`, `CompletionExitObserver::notify` | The generic completion transition also projects the new EL0 thread-join and supervisor domain-exit observers. A missing/recycled TID completes immediately only after the expected generation is checked. |
| `Reclaim` | `completion::close` | Direct for terminal-only capability removal; a queued CQ entry may outlive the capability. |

### Completion concrete-to-abstract state

| Model state | Concrete source |
|---|---|
| `InFlight`, `CancelPending`, `Completed`, `Observed` | `completion::OpState`. |
| `Reclaimed` | Capability absent after `completion::close`. |
| `cqDrained` | Whether userspace has consumed the operation's CQ entry; this is deliberately separate from `OpState::Observed`. |
| `cqRings.entries` | Entries visible in `CompletionQueueRing`. |
| `cqRings.backlog` | `CqState::backlog`. |
| `cqRings.gen` | `CqState::work_generation`. |
| `waiters` | Queue observer registration and queue-wide `last_seen_generation`, reduced to one reactor per CQ. |
| `kernelOwnsBuffer` | Whether an in-flight or cancel-pending operation can still access a submitted mutable buffer. |

`CharlotteCQ_buffer_unsafe.cfg` makes cancellation release the buffer
immediately. TLC must violate `NonTerminalBufferRemainsLoaned`, retaining the
use-after-cancel scenario as an executable negative regression.

## Deliberately omitted behavior

The models currently omit vector IPC, connection attachments, multiple memory
attachments, memory contents, concrete page-table mappings, completion observers,
detached operations, timeout races, address-space validation failures and
cross-registry rollback steps.

Those omissions matter when interpreting a result: TLC checks the modeled
protocol projection only. A future refinement effort should split concrete
multi-registry operations into pre-linearization, committed and rollback
actions and prove that their projection implements these abstract transitions.

## Scheduler lifecycle

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Spawn` | `Thread::new`, `system_scheduler::publish_thread` | Direct for reusable TID allocation and fresh monotonic generation. Publication shares the domain-abort gate, so a new sibling is either captured by the abort snapshot or rejected. |
| `Admit` | `submit_new_thread`, `submit_to_lp` | Abstract: initial run-queue insertion and `ThreadState::Ready` are one transition. Deferred re-admission uses generation-checked `submit_woken_thread`. |
| `Dispatch` / `Preempt` / `SwitchOff` | `RoundRobin::next`, ISA `cond_yield_lp` / `switch_ctx` | `onCpu` is deliberately separate from `ThreadState`: a blocking or wake-raced outgoing thread retains physical execution until the context switch. |
| `Block` | `block_thread_with_constraint` | Direct for waker generation capture and `Blocked`; concrete observer registration shares the transition's linearization point. |
| `Wake` | `Waker::notify`, `submit_woken_thread`, `add_thread` | Direct for generation validation and re-admission. A stale generation disables the model action and is rejected by Rust. |
| `Migrate` | `try_rebalance` | Direct for migration-safe, unpinned Ready threads; load-window policy is omitted. |
| `RequestRemoteAbort` | `abort_thread`, `abort_requested`, `abort_owner_lp`, scheduler IPI | Direct for cross-LP termination: the caller records the physical owner but leaves the executing context in the master table. |
| `RetireRemoteAbort` | `RoundRobin::next`, `retire_requested_threads` | Owner-LP transition after switching away. Run-queue selection and `add_thread` reject the requested generation, including block/wake races. |
| `AbortNotRunning` / `SelfAbort` | `abort_thread`, `take_element`, `stage_dead_thread` | Non-running contexts can be removed immediately; self-exit is staged while still on its stack and switches away before reaping. |
| `BeginDomainAbort` | `domain_abort`, `abort_address_space`, `abort_as_threads` | The concrete refinement holds the publication gate, records the current address-space generation, snapshots every owned thread, and retains the gate through the sweep. This is the linearization boundary for the model's bulk transition. |
| `Reap` | `reap_dead_threads` | Direct for post-context-switch destruction; the concrete stack-pointer check may defer a context again. |
| `DestroyAddressSpace` | `domain_exited`, `teardown_domain`, `close_user_address_space` | Requires every master-table and deferred-dead thread owned by the ASID to be gone. `OnCpuHasLiveAddressSpace` checks the resulting safety boundary. |

The model makes master-table removal and insertion into the deferred-dead
state one atomic retirement action. Rust uses separate locks for those tables,
so `RETIREMENTS_IN_FLIGHT` marks the concrete intermediate interval;
`domain_exited` rejects that interval. This supplies the linearization bridge
needed for the model's atomic `RetireRemoteAbort`/`AbortNotRunning` projection.

`CharlotteScheduler_unsafe.cfg` enables the former immediate remote-removal
transition and is required to produce a `ReapOnlyOffCpu` counterexample. This
negative model corresponds to the stale-AS SVC panic fixed by owner-LP
retirement; it is not part of the repaired `Spec`.

`CharlotteScheduler_domain_abort_unsafe.cfg` publishes a sibling after the
abort snapshot and must violate `AbortingThreadsDoomed`. The Rust publication
gate prevents this trace and also prevents a captured numeric TID from being
reused by another domain before the sweep reaches it.

## Reusable address spaces and interrupt routes

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| Address-space `Allocate` / `CaptureHandle` | `register_user_address_space`, `AddressSpaceHandle` | Direct for recyclable numeric ASID plus monotonic software generation. The same handle identity now keys service scratch-window cursors. |
| Address-space `CloseExact` | `close_user_address_space_handle`, generation checks in map/unmap/teardown | Direct for rejecting a stale handle after ASID reuse. Scratch allocation and mapping teardown are serialized across this boundary. |
| Hardware-ASID `Allocate` / `Retire` / `Invalidate` | AArch64 hardware-ASID allocator and TLB invalidation | Abstract: page-table contents are omitted; tag reuse is allowed only after invalidation removes stale translations. |
| Interrupt-route `Bind` / `QueueWake` / `Unbind` / `DrainSafe` | device interrupt binding, route generation, deferred wake drain | Direct for generation-fenced delivery. GIC register programming and MPIDR routing are below the model boundary. |

The August `memory_map_any` repair did not change memory ownership in
`CharlotteIPC`; it changed address-space placement. Its safety-relevant part
is the generation-keyed scratch cursor and lifecycle serialization represented
by `CharlotteAddressSpace`, not a new IPC transfer mode.

## Service lifecycle

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `StageTrusted` / `StageUntrusted` / `RejectUntrustedLoad` | signed service bundle/object-store staging; `verify_image_signature`, `try_load_domain` | Direct for the trust gate: unsigned, tampered, or artifact-mismatched bytes cannot reach address-space allocation/mapping. Cryptography and ELF parsing are abstracted to the trust bit. |
| `Load` | `loader::try_load_domain` | Abstract: ELF segments, mappings, bootstrap frames and finite hardware-ASID allocation are omitted. |
| `Start` | `start_domain`, `spawn_thread` | Direct for the initial `(tid, generation)` domain handle. |
| `Prepare` | `NameCatalog::apply_command(CMD_REGISTER)` | Direct: increments the retained generation, records the owner, and stores an inactive entry. A replacement is intentionally unresolvable until activation. Exhaustion returns generation zero without changing the entry. |
| `PublishLocal` | node-local `ns::register` from DNS registration flow | Direct for installing the re-delegable local connection before distributed visibility. The local name service independently allocates and returns its checked generation. |
| `Activate` / `RejectStaleActivate` | `CMD_ACTIVATE` | Direct: only the exact prepared generation with a nonempty owner becomes active. |
| `Lookup` | `NameCatalog::lookup`, quorum-contact query path | Direct for filtering inactive entries/tombstones. Linearizable read-barrier mechanics remain in the Raft layer. |
| `FencedUnregister` / `RejectStaleUnregister` | `CMD_UNREGISTER_GENERATION`; local `OP_UNREGISTER_GENERATION` | Direct owner-and-generation fence. A delayed cleanup request cannot unpublish a replacement. The unsafe model removes this check and retains the corresponding counterexample. |
| `CleanupLocal` | `pending_local_unregistrations`, `LocalPublication::local_cleanup_submitted` | Abstract asynchronous cleanup, separately fenced by the node-local generation. |
| `RequestStop` / `Exit` | `ipc_connection_watch_closed`; service shutdown or `abort_thread` | Endpoint closure completes a retained kernel watch; the owner proposes the tombstone directly or sends a source-authenticated, retried request to the known leader. The catalog may remain briefly active while this asynchronous transition is in flight. |
| `DomainAbort` | Rust panic handler, fatal EL0 exception handling, `DOMAIN_ABORT` | Abstract spontaneous failure: scheduler retirement of the complete address space is collapsed into `Exited`; catalog cleanup, reaping, and teardown retain their ordinary fences. |
| `Reap` | `wait_domain_exit`, scheduler master/dead-table observations | Direct for the condition required before teardown. |
| `Teardown` | `teardown_domain`, `close_user_address_space` | Direct for the reaping precondition and resource/address-space release. |

Concrete teardown treats a domain as exited only after every master-table and
deferred-dead thread with that ASID is gone. Looking only at the initial TID
allowed a secondary EL0 thread to enter SVC after its address space had been
removed; the strengthened check implements the model's domain-wide `Reap`
precondition.

The userspace Raft reactor applies the same single-wait discipline: a bounded
CQ wait is released by endpoint/transport readiness or supplies the next
election-clock tick on timeout. It does not combine an indefinite CQ wait with
a separately delivered detached-timer completion.

## DMA and SMMUv3 lifecycle

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `CreateMemory` | `memory::object::allocate` | Abstract: frame count and physical addresses are omitted. |
| `CreateDomain` | `device::grant_dma_domain`, `smmu::create_domain` | Direct for unique requester-stream ownership and domain authority. Page-table allocation is omitted. |
| `CpuMap` / `CpuUnmap` | `memory::object::map`, `unmap` | Abstract boolean projection of CPU page mappings. Exclusive DMA blocks mapping; concrete virtual addresses and permissions are omitted. |
| `BeginLoan` / `EndLoan` | `lend_read`, `lend_write`, `return_lend` | Abstract projection of IPC memory loans. A new loan is rejected while any DMA pin exists. |
| `BeginMap(..., "Coherent")` | unsafe `dma_map`, `memory::object::pin_for_dma` | The raw coherent-sharing path may coexist with CPU mappings or a previously established loan; synchronization remains the unsafe caller's obligation. It cannot coexist with an exclusive pin. |
| `BeginMap(..., "Exclusive")` | safe `dma_map_exclusive`, `OwnedMemory::begin_dma` | Direct ownership transfer: requires no CPU mappings, loans, or other DMA pins. The mode remains exclusive until acknowledged unmap and pin release. |
| `CommitMap` | `Domain::map`, `invalidate_asid`, successful return from `smmu::map` | Abstract: per-page PTE installation and IOVA arithmetic are collapsed. Publication occurs only after invalidation succeeds. |
| `FailMap` | partial-PTE cleanup and `memory::object::unpin_dma` | Direct rollback for failures before complete PTE installation, including unknown-domain lookup. |
| `QuarantineMap` | failed `invalidate_asid` after `Domain::map` | Direct safety policy: the unpublished internal mapping retains its pin because hardware translation state is uncertain. A later acknowledged domain destroy reclaims it. |
| `RevokeMap` / `ReleasePin` | `Domain::clear_mapping`, `invalidate_asid`, then `unpin_dma` | Direct ordering: translation removal is acknowledged before the object becomes reclaimable. |
| `BeginDestroy` | `smmu::destroy_domain` before the aborting STE is acknowledged | Abstract in-progress management state under the SMMU lock. |
| `AcknowledgeDestroy` | successful `write_ste(sid, None)` | Direct linearization point at which the requester stream is forced to abort and mappings may be consumed. |
| `QuarantineDestroy` | `destroy_domain` error return | Direct safety policy: the domain and pins remain retained when hardware acknowledgement is uncertain. |
| `ExitDriver` | `device::close_address_space`, `memory::object::close_address_space` | Abstract bulk teardown; pinned owned memory becomes `destroy_when_unpinned`. |
| `ReclaimMemory` | final `unpin_dma` for a destroy-pending object | Direct for last-pin removal, capability cleanup and frame reclamation. |

`ExclusiveDmaHasNoCpuAuthority` checks that an exclusive pin cannot overlap a
CPU mapping, IPC loan, or second DMA pin. `CharlotteDMA_unsafe.cfg` removes the
exclusive precondition and must produce a counterexample.

## Raft election and durable voting

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `StartElection` | `RaftNode::start_election` | Abstract atomic transition. Rust persists the incremented term and self-vote before sending vote requests. |
| `GrantVote` | `handle_vote_request`, followed by `handle_vote_response` | Abstract delivery pair. Voter persistence precedes its response; candidate vote sets deduplicate peer IDs. Log freshness is omitted. |
| `BecomeLeader` | `has_election_majority`, `become_leader` | Direct for a fixed voter set and distinct-voter majority. Leader no-op append is deferred to the log layer. |
| `ObserveHigherTerm` | `step_down` from request or response handling | Direct for durable term advancement, vote clearing and candidate-vote reset. |
| `Crash` / `Restart` | service-domain exit and reconstruction through `RaftNode::new` | Abstract: volatile role and collected votes disappear; term and vote reload from `PersistentStateStore`. |

This first Raft layer checks election safety only. It represents loss by
withholding an action and duplication through idempotent same-candidate voting.
It does not model the transport queue, snapshot installation, storage write
failures, fairness, or eventual election. Joint membership is checked by the
separate membership layer below.

## Raft log replication and commit

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Elect` | `handle_vote_response`, `become_leader` | Assumes the election model's one-leader-per-term result and Raft's up-to-date-log voting rule. Each replacement leader has a strictly newer term. |
| `AppendLeader` | `append_client_entry`, `LogStore::append` | Direct for a durable leader append. Client response and state-machine application are omitted. |
| `ReplicateOne` | `handle_append_entries`, `truncate_suffix`, `append` | One-entry projection. A matching entry retains the existing suffix; a conflict truncates from that index before appending. The store flushes each mutation before returning. |
| `CommitLeader` | `advance_commit_index` | Direct: a configured-voter majority must contain the index, and Raft advances by counting only an entry from the leader's current term. |
| `PropagateCommit` | follower `handle_append_entries` update of `commit_index` | Direct for `min(leader_commit, last_new_index)` after prefix validation. |
| `Crash` / `Restart` | service-domain exit and `RaftNode::new` with its `LogStore` | Durable logs survive. The model conservatively retains commit knowledge; concrete restart currently reconstructs it from snapshot/application progress and subsequent leader messages. |

The second layer checks log matching, committed-entry agreement, and leader
completeness under bounded conflict repair and restart. It relies on the first
layer for election safety rather than reimplementing durable one-vote-per-term
inside the log model. Snapshots and joint membership are checked by separate
layers. Failed storage operations, state-machine application, transport
framing, and temporal liveness are outside this abstraction.

## Raft membership and joint consensus

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Elect` | `has_election_majority`, `become_leader` | Membership projection of election: a candidate must be a current voter and must obtain both current and next voter majorities while joint. Durable voting remains in the election model. |
| `SubmitJoint` | `submit_joint_configuration`, `submit_command` | Direct for appending the encoded `JOINT` command while the old configuration is authoritative. |
| `Replicate` | append-response handling and `match_index` | Abstract monotonic replication progress for active voters and learners. Log contents and conflict repair remain in the log model. |
| `CommitJoint` | `advance_commit_index`, then `apply_configuration_command(Joint)` | The `JOINT` entry commits under its preceding configuration; applying it activates the old/new union and records the finalization fence. |
| `SubmitFinalize` | `maybe_auto_finalize_joint_configuration` | Direct: every proposed voter and learner must reach the committed joint-entry fence before `FINALIZE` is submitted. |
| `CommitFinalize` | `advance_commit_index`, then `apply_configuration_command(Finalize)` | Requires both voter majorities while joint, installs the next configuration, and decommissions nodes absent from the resulting voter/learner set. |
| `Crash` / `Restart` | service-domain exit and `RaftNode::new` | Abstract volatile availability. Durable configuration recovery is checked in the snapshot layer. |

The model represents peer identity and role as voter/learner sets. Peer
addresses, protobuf encoding, transport retries, and temporal availability are
outside this abstraction. Pre-membership join admission is checked separately.

## Raft pre-membership join admission

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Elect` | ordinary single-node election before admission | Abstract; durable election safety remains in `CharlotteRaft`. |
| `BeginJoining` | `RaftNode::begin_joining` | Direct: persists the selected anchor and pre-admission snapshot index before stepping down, disabling the election deadline, and clearing candidate votes. |
| `SubmitJoin` / `CommitJoin` | `submit_join`; application of `ConfigurationCommand::Join` | The JOIN is idempotently appended and commits under the existing configuration. Only then does `pending_joiners` expose its fence. |
| `ReplicateToJoiner` | `accepts_joining_leader`; `handle_append_entries` / `handle_install_snapshot` | Direct source fence: a non-member accepts log/snapshot authority only from `joining_from`. |
| `SubmitJoint` | `maybe_promote_pending_joiners` | Direct fence: every promoted joiner has `match_index >= JOIN index`; caught-up joiners may share one joint transition. |
| `CommitJoint` | `apply_configuration_command(Joint)` | The admitted node atomically snapshots current/next membership, then clears the durable and volatile admission fences. |
| `Crash` / `Restart` | service-domain exit and `RaftNode::new` | Volatile join posture disappears; `PersistentStateStore::join_admission` restores the selected anchor and suppresses elections until a newer durable membership snapshot contains this node. |
| `UnsafeReplicateToJoiner` | pre-fix arbitrary non-voter leader acceptance | Negative model only; violates `JoiningAcceptsOnlySelectedAnchor`. |
| `UnsafeRestartForgetsAdmission` | pre-fix reconstruction with `joining = false` | Negative model only; violates `RestartPreservesAdmission` and captures the singleton-election regression. |

Dynamic admission is enabled only when the standalone Raft service opened its
disk-backed log and persistent-state stores. Memory mode remains useful for
local tests, but it refuses auto-join because a process restart cannot preserve
the admission fence.

## Raft snapshot installation and recovery

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `BeginReceive` / `ReceiveChunk` | `handle_install_snapshot`, `PendingSnapshot` | Direct for ordered chunk accumulation. A mismatched offset is rejected without changing the durable image. |
| `DiscardStale` | early completed response from `handle_install_snapshot` | Direct: an index at or below `commit_index` is acknowledged but cannot replace newer state or move progress backwards. |
| `PersistSnapshot` | `LogStore::install_snapshot`, `DiskLogStore::persist_log_state` | The durable boundary, bytes, current/next membership, and compatible suffix are one serialized object-store replacement. The object store publishes it copy-on-write after data and metadata reach stable storage. |
| `ActivateSnapshot` | commit/last-applied update, membership reconstruction, and `StateMachine::restore` | Abstract split after durable publication so a crash between persistence and activation is explored. Membership activation also recomputes local decommissioning. |
| `Crash` | service-domain exit | Pending chunks and volatile state-machine contents disappear; the atomically published log-state object survives. |
| `Restart` | `DiskLogStore::new`, then `RaftNode::new` | Direct: construction restores snapshot bytes and current/next membership before exposing its index as committed and applied. |

The model abstracts snapshot contents to one value, membership to peer-ID sets,
and chunks to a bounded count. It omits peer roles and addresses within those
sets, checksums below the object-store interface, network framing, storage
exhaustion, and state-machine-specific validation of snapshot bytes.

## Remote-call identity and uncertainty

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Start` | `dns::OP_CALL`, `InFlightCall` | Captures caller DNS session, monotonic call ID, expected peer and replicated target generation before dispatch. |
| `ReplaceTarget` / `RejectStale` | `NameCatalog` generation transition; inbound `rcall` validation | The target executes only when the request generation equals the active catalog generation; otherwise it returns `ERR_STALE_GENERATION`. |
| `Execute` / `DuplicateRequest` | inbound `TAG_REQUEST`; `CompletedCall` cache | First delivery invokes the local endpoint and caches its result; the same caller/session/call identity reuses the cached result. |
| `QueueReply` / `DeliverReply` | `TAG_REPLY`; relmsg acknowledgement counter | Abstract split between application result creation, reliable-message delivery, and client reply completion. |
| `Timeout` | `REMOTE_CALL_TIMEOUT_MS`, in-flight expiry | Direct: once dispatch may have executed, expiry returns `ERR_UNCERTAIN`, not a retry-safe transport error. |
| `SettleTransport` / `Evict` | per-peer relmsg reply-ACK ordinal and bounded result window | Direct for transport settlement: Rust evicts only an entry whose reply ordinal has been acknowledged by that peer. If every entry remains unsettled, it returns `ERR_BUSY` before execution instead of evicting deduplication evidence. Explicit uncertain-session retirement remains a modeled extension. |

The model does not assert global exactly-once behavior. It checks at-most-once
execution while an identity remains tracked and makes the condition for safe
eviction explicit. Durable deduplication across DNS process restart and general
remote object-capability transfer are outside the implementation and model.

## Reliable-message sessions

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `AbandonSession` | retry-lease exhaustion; `Peer::abandon_transmit_session` | Direct: the next send resets its sequence under a fresh retry epoch. Exhaustion disables sends instead of reusing an identity. |
| `RestartService` | new relmsg name-service generation; `initial_wire_session` | Direct for a new service-generation namespace. The packed high/low fields make restart identities disjoint from every retry of an earlier generation. |
| `AcceptCurrentSession` | `Peer::accept_session`, `wire_session_is_newer` | Direct: a SYN may reset receive sequencing only when its well-formed packed identity is strictly newer. |
| `AcceptDelayedSession` | former bounded retired-session acceptance | Negative model only. Once an old identity fell out of the window, a delayed SYN could regress receive state. |

Payload fragmentation, buffer ceilings, retransmission timing, ACK/data frame
encoding and Ethernet loss are outside this session-identity projection. ACK
completion is nevertheless source-session-fenced in Rust by comparing the
echoed session and pending sequence before consuming the pending call.

## Unified capability namespace

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Allocate` | `capability::allocate` | Direct for fresh per-AS serial allocation and authoritative kind insertion. |
| `Remove` | typed-registry removal followed by `capability::remove` | Direct for owner-and-kind checked removal. Concrete callers assert that the unified entry exists, including optimized builds, so payload and authority tables cannot silently diverge. |
| `DelegateCopy` | subsystem delegation followed by `allocate` in the target AS | Abstract: payload-table insertion is omitted; the target handle is fresh. |
| `BeginMove` / `CommitMove` | subsystem move transaction and target capability allocation | Abstract split around payload transfer so intermediate revocation is checkable. |
| `RollbackMove` | `memory::object::rollback_move_to`, `capability::restore` | Direct for reverse-order transaction rollback and crate-private restoration of the exact pre-transaction handle. The live-upgrade supervisor uses the target handles returned by `move_to`, rolls partial multi-object handoff back in reverse order, and aborts replacement launch on endpoint-delegation failure. |
| `CloseAddressSpace` | `capability::close_address_space` | Direct for dropping the complete authority namespace after payload teardown. |

## Authorization policy and capability issuance

The target-independent state machine is implemented and host-tested in
`charlotte-authorization`, but production authorization is not yet wired into
`ns`. The current service still maps names to re-delegable connections and
optionally compares a service-selected bearer key. Its IPC envelope lacks the
authenticated address-space generation required to use the engine's stable
identity boundary safely, and it has no authorization audit records. The
design contract and staged implementation plan are in
[`../architecture/authorization-policy.md`](../architecture/authorization-policy.md).

| TLA+ action | Intended CharlotteOS implementation | Correspondence |
|---|---|---|
| `PublishService` / `ReplaceService` / `UnpublishService` | `PolicyStore::publish_service` / `unpublish_service`, then generation-fenced `ns`/DNS publication | Direct in the engine for authenticated role, rights ceiling, checked generation, and stale-unpublish rejection. Runtime role provisioning and catalog integration remain unwired. |
| `SetPolicy` | `PolicyStore::set_policy` | Direct for administrator role, exact subject/service rule, optimistic version fence, explicit deny, and exhaustion failure. The administration IPC endpoint and durable/audited storage remain target work. |
| `IssueTicket` | `PolicyStore::issue_ticket` | Direct for exact generation-aware identity, requested rights, current rule and binding, attenuation, bounded outstanding decisions, and default deny. Kernel-authenticated `DomainIdentity` delivery is still missing from the runtime adapter. |
| `CancelTicket` | `PolicyStore::cancel_ticket` | Direct for subject-bound removal. Expiry is not implemented. A co-located adapter can keep decisions internal by using `authorize_now`. |
| `Redeem` | `PolicyStore::redeem_ticket`, followed by attenuated `ipc_reply_connection` | Direct in the engine for single use and subject, policy-version, service-generation, and rights revalidation. Connection delegation exists in `ns`, but the safe adapter joining the two is intentionally not wired. |
| `CloseCapability` | ordinary `ipc_close` or endpoint teardown | Direct for capability lifetime. Policy changes intentionally do not retract an already issued direct connection. |

The five negative actions define concrete regression obligations for a future
implementation: ordinary callers cannot mutate rules; a decision cannot be
redeemed for another principal; policy and service replacement fence stale
decisions; and minting cannot amplify rights.
