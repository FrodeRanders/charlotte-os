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
attachments, memory contents, page mappings, DMA pins, completion observers,
detached operations, timeout races, address-space validation failures and
cross-registry rollback steps.

Those omissions matter when interpreting a result: TLC checks the modeled
protocol projection only. A future refinement effort should split concrete
multi-registry operations into pre-linearization, committed and rollback
actions and prove that their projection implements these abstract transitions.
