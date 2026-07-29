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
| `ScalarCall` | `ipc::scalar_call` | Direct for pending-call creation, internal reply-token creation and enqueue. No server-visible reply capability exists yet. |
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

## Completion queues

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `OpenCq` | completion address-space/CQ attachment | Abstract: allocation and shared mapping are omitted. |
| `SubmitNoBuffer` / `SubmitWithBuffer` | `completion::submit`, `Completion::new` | Direct: a successful submission begins in `InFlight`; capacity failure is represented by the disabled action. |
| `Complete` / `Fail` | `completion::complete`, `Completion::complete`, `post_to_cq` | Direct for idempotent terminal transition, CQ publication, generation increment and notification. Result variants are reduced to status/value. |
| `CancelOp` | `completion::cancel`, `Completion::cancel` | Direct: `InFlight` becomes `CancelPending`; no CQ entry is posted until completion. |
| `DrainOne` / `DrainAll` | userspace ring drain plus kernel `flush_backlog` | Abstract: shared-memory head/tail mechanics and batching details are collapsed. Marks CQ delivery consumed, not the capability result observed. |
| `ObserveResult` | `completion::poll`, `Completion::take` | Direct: `Completed` becomes `Observed`, independently of CQ draining. |
| `CqWait` | `completion::wait_on_cq` and timeout variant | Abstract: scheduler blocking and the post-registration lost-wake recheck are represented by one atomic waiter transition. |
| `CqWake` | `completion::wake` | Direct for generation increment and observer notification. |
| `SubmitTimer` | `completion::submit_timer` and timer observer registration | Abstract: deadline values and scheduler timer queues are omitted. |
| `TimerFire` | `CompletionTimerObserver::notify`, `completion::complete` | Direct for first-winner completion and cancellation-result forcing. Fairness of timer delivery is not checked. |
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

## Deliberately omitted behavior

The models currently omit vector IPC, connection attachments, multiple memory
attachments, memory contents, CPU page mappings, completion observers,
detached operations, timeout races, address-space validation failures and
cross-registry rollback steps.

Those omissions matter when interpreting a result: TLC checks the modeled
protocol projection only. A future refinement effort should split concrete
multi-registry operations into pre-linearization, committed and rollback
actions and prove that their projection implements these abstract transitions.

## Scheduler lifecycle

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Spawn` | `Thread::new`, `MASTER_THREAD_TABLE.add_element` | Direct for reusable TID allocation and fresh monotonic generation; context construction is omitted. |
| `Admit` | `submit_new_thread`, `submit_to_lp` | Abstract: initial run-queue insertion and `ThreadState::Ready` are one transition. Deferred re-admission uses generation-checked `submit_woken_thread`. |
| `Dispatch` / `Preempt` | `RoundRobin::next` | Direct for the Ready/Running handoff and one current thread per LP. |
| `Block` | `block_thread_with_constraint` | Direct for waker generation capture and `Blocked`; concrete observer registration shares the transition's linearization point. |
| `Wake` | `Waker::notify`, `submit_woken_thread`, `add_thread` | Direct for generation validation and re-admission. A stale generation disables the model action and is rejected by Rust. |
| `Migrate` | `try_rebalance` | Direct for migration-safe, unpinned Ready threads; load-window policy is omitted. |
| `Abort` | `abort_thread`, `take_element`, `stage_dead_thread` | Abstract atomic transition from master-table membership to the LP-local dead list. |
| `Reap` | `reap_dead_threads` | Direct for post-context-switch destruction; stack-pointer deferral is omitted. |

## Service lifecycle

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Load` | `loader::load_domain` | Abstract: ELF parsing, mappings and bootstrap frames are omitted. |
| `Start` | `start_domain`, `spawn_thread` | Direct for the initial `(tid, generation)` domain handle. |
| `Publish` | name-service `register` map insertion | Direct linearization point: the new `(connection, generation, access key)` is inserted before the superseded connection is retired or deferred lookups are released. Deferred lookups retain the caller key and are authorized against the published policy before receiving a connection. IPC details are covered by `CharlotteIPC`. |
| `RequestStop` / `Exit` | service shutdown or `abort_thread` | Abstract: cooperative and forced shutdown share the same lifecycle projection. |
| `Reap` | `wait_domain_exit`, scheduler master/dead-table observations | Direct for the condition required before teardown. |
| `Teardown` | `teardown_domain`, `close_user_address_space` | Direct for the reaping precondition and resource/address-space release. |

The userspace Raft reactor applies the same single-wait discipline: a bounded
CQ wait is released by endpoint/transport readiness or supplies the next
election-clock tick on timeout. It does not combine an indefinite CQ wait with
a separately delivered detached-timer completion.

## DMA and SMMUv3 lifecycle

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `CreateMemory` | `memory::object::allocate` | Abstract: frame count, physical addresses and ordinary CPU mappings are omitted. |
| `CreateDomain` | `device::grant_dma_domain`, `smmu::create_domain` | Direct for unique requester-stream ownership and domain authority. Page-table allocation is omitted. |
| `BeginMap` | `memory::object::pin_for_dma` | Direct: rights are checked and the complete object is pinned before entering the SMMU registry. |
| `CommitMap` | `Domain::map`, `invalidate_asid`, successful return from `smmu::map` | Abstract: per-page PTE installation and IOVA arithmetic are collapsed. Publication occurs only after invalidation succeeds. |
| `FailMap` | partial-PTE cleanup and `memory::object::unpin_dma` | Direct rollback for failures before complete PTE installation, including unknown-domain lookup. |
| `QuarantineMap` | failed `invalidate_asid` after `Domain::map` | Direct safety policy: the unpublished internal mapping retains its pin because hardware translation state is uncertain. A later acknowledged domain destroy reclaims it. |
| `RevokeMap` / `ReleasePin` | `Domain::clear_mapping`, `invalidate_asid`, then `unpin_dma` | Direct ordering: translation removal is acknowledged before the object becomes reclaimable. |
| `BeginDestroy` | `smmu::destroy_domain` before the aborting STE is acknowledged | Abstract in-progress management state under the SMMU lock. |
| `AcknowledgeDestroy` | successful `write_ste(sid, None)` | Direct linearization point at which the requester stream is forced to abort and mappings may be consumed. |
| `QuarantineDestroy` | `destroy_domain` error return | Direct safety policy: the domain and pins remain retained when hardware acknowledgement is uncertain. |
| `ExitDriver` | `device::close_address_space`, `memory::object::close_address_space` | Abstract bulk teardown; pinned owned memory becomes `destroy_when_unpinned`. |
| `ReclaimMemory` | final `unpin_dma` for a destroy-pending object | Direct for last-pin removal, capability cleanup and frame reclamation. |

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
addresses, protobuf encoding, join-request admission, transport retries, and
temporal availability are outside this abstraction.

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

## Unified capability namespace

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Allocate` | `capability::allocate` | Direct for fresh per-AS serial allocation and authoritative kind insertion. |
| `Remove` | typed-registry removal followed by `capability::remove` | Direct for owner-and-kind checked removal. Concrete callers assert that the unified entry exists, including optimized builds, so payload and authority tables cannot silently diverge. |
| `DelegateCopy` | subsystem delegation followed by `allocate` in the target AS | Abstract: payload-table insertion is omitted; the target handle is fresh. |
| `BeginMove` / `CommitMove` | subsystem move transaction and target capability allocation | Abstract split around payload transfer so intermediate revocation is checkable. |
| `RollbackMove` | `memory::object::rollback_move_to`, `capability::restore` | Direct for reverse-order transaction rollback and crate-private restoration of the exact pre-transaction handle. The live-upgrade supervisor uses the target handles returned by `move_to`, rolls partial multi-object handoff back in reverse order, and aborts replacement launch on endpoint-delegation failure. |
| `CloseAddressSpace` | `capability::close_address_space` | Direct for dropping the complete authority namespace after payload teardown. |
