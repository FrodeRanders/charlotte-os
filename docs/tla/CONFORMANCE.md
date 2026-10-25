# TLA+ to Rust Conformance Map

This document records which CharlotteOS implementation behavior each TLA+
action represents. “Direct” means the action follows the current Rust state
transition while omitting data irrelevant to the checked invariant.
“Abstract” means several concrete operations are deliberately collapsed.

This is a reviewable correspondence map, not a refinement proof. The
[18 October security synchronization](../reports/audits/2026-10-18-security-model-sync.md)
adds focused root-retirement and checked-wait projections. The subsequent
[IPC publication review](../reports/audits/2026-10-18-ipc-publication-model.md)
adds captured admission, escrow and hidden delivery. Remaining omissions in
other models are still open qualification work. The subsequent
[DMA operation review](../reports/audits/2026-10-18-dma-operation-model.md)
adds fixed-unit admission and kernel/runtime map/unmap failure ownership.

The [NS publication review](../reports/audits/2026-10-18-name-publication-model.md)
adds the serialized local registry, public/policy issuance boundary, prepared
watch ownership and exact death retirement. It also corrects publication
exhaustion at NS's signed-result limit.

The [boot-trust review](../reports/audits/2026-10-18-boot-trust-model.md)
separates current verifier/fixture/handoff behavior from a conditional future
protected-installation contract. Platform, atomic-state and custody inputs remain
unimplemented production boundaries.

## Namespace token selection

`CharlotteNamespaceDrain` projects one poll of an already-fenced namespace with
quiescent callers and two nonreused token identities. Its peer completion assumes
the competing owner's typed cleanup proof; it does not manufacture that proof.

| Model action | Rust boundary | Correspondence |
|---|---|---|
| `Select`, `SelectEmpty` | IPC token scan in `namespace_close::close_with` | Selection captures an observation before preparation; no token means only that this scan drained, with later authority/memory/root cleanup still required. |
| `ClosePeer`, `FinishClaim` | Competing loan-free peer close or original claim owner completion | Actual token removal permits progress; claim completion and physical proof are boundary inputs. |
| `CancelSelected` | PreparedCancellation succeeds and finishes through its existing typed owner | Abstracts successful per-token cleanup; it never duplicates a claimed operation. |
| `RejectPreparation`, `RecheckMissing` | Preparation returns Pending, then exact presence is rechecked under IPC | Missing selection permits rescan with the same closing owner. Nonreused identities and fenced admission make every such rescan observed progress. |
| `ReturnPending` | Every still-present token remains Pending | Includes active/changed-epoch claims and ordinary peer-admission rejection. No busy loop, claim reset or cleanup authority follows this observation. |

`NoStalePending` records that a Pending decision checked actual token presence.
`CompleteOnlyDrained` forbids namespace completion with represented tokens left.
Unsafe controls restore stale Pending or skip a live selection. The finite graph
ends at a poll's return; later wakes, deadlines, reentry, epoch transitions,
allocator/admission pressure and root/loan/backing release are outside it. The
existing root-retirement model carries the enclosing lifetime boundary. Native
controls inject a real close between scan/preparation with local guards released;
production supplies an empty fixture hook. See the
[implementation evidence](../reports/audits/2026-10-22-ipc-namespace-selection.md).

## Controller server relays

| Model state/action | Implementation | Qualification |
|---|---|---|
| `CheckedPreflight`, `proof`, `sent` | `clusterctl_relay::preflight`, `Runtime::poll`, `Machine::arm` | Only an explicit absent result or checked older record permits mutation; identical bytes return a generation and other outcomes stop. Byte/signature/identity validation is a host/native/source boundary, abstracted as checked evidence here. |
| `ProducerReply`, `ObserveTerminal`, `Park`, `Wake` | CQ-bound DNS calls, bounded receive turn and rotating relay observation | Two already admitted operations assume binding and producer delivery. Ready work prevents parking on a silent peer. Legacy upload/join/key execution and bounded synchronous validation remain outside this guarantee. |
| `deadline`, `Expire`, `backing`, `ReleaseReceiver` | One server budget across preflight/mutation; cancel pending before mapping and upstream reply | Independent caller deadline is not renewed. DNS may retain a moved copy after local termination; logical capacity is not physical refund. Actual original/copy byte admission, returned resources and shutdown use source/tests rather than this projection. |
| `AbortBindingOrWait` | fail-closed binding/wait rejection with pool live | No retry owner or physical cleanup authority is introduced. Native fixture exercises healthy moved return beside held DNS, incoming mapping timeout and unexpected connection rejection; actual map/bind faults are not injected. |

Four negative controls violate `MutationRequiresCheckedQuery`, `OriginalBudget`,
`ParkCoversAll` and `AcceptedOnce`. See the
[server evidence](../reports/audits/2026-10-22-security-controller-relay.md).

## Deployment listener setup

| Model state/action | Implementation | Qualification |
|---|---|---|
| `stage`, `status`, `Admit`, `accepted` | `deployment_listener::Machine::advance`, `OpeningSocket`, `OwnedSocket::listen_call`, `accept_call` | Creation/listen/accept retain one owner in the existing position. Binding is assumed within admission before parking; actual binding failure aborts with the accepted owner live. No accepted replay. |
| `socket`, `Observe`, `RejectConfirmed` | Checked creation scalar produces a private socket lease; listen/accept require zero without attachments; confirmed negatives reject | The model abstracts validated replies. Runtime retains uncertain observations; returned attachments follow existing owned disposal. Transfer to input/close ends this projection and proves no physical release. |
| `deadline`, `RejectQueue`, `retryAt`, `Tick` | One five-second setup budget and queue-full-only 50 ms admission backoff | Creation/listen/accept phase changes preserve the deadline. The idle admitted accept has no timer. Concrete clock arithmetic and syscall return latency remain source/test boundaries. |
| `Park`, `Wake`, `ProducerReply`, `Abort` | CQ drain, exact owner recheck, earliest remaining deadline, terminal domain abort with all fields live | Fixed two-position projection assumes producer notifications and atomic turns. Three negatives replay a phase, restart its budget or park while another owner is ready. No temporal fairness claim. |

Shutdown retains accepted creation/listen until checked observation, then hands
known sockets to the existing close pool; idle accepts cancel before their
sockets move. This host-tested adapter behavior, frame/mapping/returned-owner
cleanup, dynamic admission across all five phases, physical TCP state and
wall-clock pressure are outside this model. The native IPC adapter holds creation
beside another listener's complete setup/accept/close; native HTTP runs use the
real TCP/IP service. See the
[listener evidence](../reports/audits/2026-10-22-security-deployment-listener.md).

## Deployment logical close

| Model state/action | Implementation | Qualification |
|---|---|---|
| `owners`, `state`, `Admit`, `accepted` | `OwnedSocket::into_closing`, `ClosingSocket`, `deployment_close::Machine::start/arm` | Complete pending/socket owner enters a fixed client position before admission. Only queue-full rejection permits another attempt. Model starts with two already owned sockets; setup and cross-phase admission are omitted. Binding is assumed before parking. |
| `RejectQueue`, `retryAt`, `Tick` | 50 ms admission backoff inside the original five-second close deadline | Queue admission has no notification surface. Accepted pending calls use their real CQ source; backoff is not described as readiness waiting. Model time is finite abstract ticks. |
| `ProducerAck`, `ProducerUnavailable`, `ObserveAck`, `confirmed` | `ClosingSocket::poll`, checked Started/Unavailable scalar without attachments | Either typed logical receipt disarms socket fallback and releases its position. Returned-capability rejection/cleanup uses the runtime; actual decoding is source-reviewed/native success, not a model refinement. No former-ownership, peer-delivery or physical refund proof; actual nonreused IDs and reaping are source/host/native boundaries outside this model. |
| `Expire`, `RejectWaitOrObservation` | deadline, non-queue submission/binding/observation failure; incomplete close Drop | Abort with all represented owners live; never cancel and reconstruct accepted close. Model preserves owners after abort. Actual domain retirement, wider field destructors and whole-service shutdown remain separate. |
| `Park`, `Wake` | drain CQ, recheck close pool and other phases, wait on earliest original deadline/backoff | A held close does not suppress another ready acknowledgement. Wake itself releases no position. The Rust shutdown branch drains existing closes with original budgets while retaining other pools; timing/destructor qualification remains outside this model. |

The one-owner `Blocking` configuration also maps `RetainAdmission` to
`socket_close::Blocking`/`finish_transaction`: an unstarted final rejection
retains the complete operation without destructor work or another submission.
`UnsafeFallback` re-enables work after that terminal state and must violate
`NoReentry`. This retained state is not recoverable custody, a remote refund or
an automatic retry owner. `ClosingSocket::wait_timeout` and CQ polling share
`observe`; the existing checked-wait model qualifies the lower readiness loop
separately. Exact one/five-second policy, actual returned-capability cleanup,
parent field order and runtime domain abort are source/test boundaries.

Safe checks require `NoReentry`, `AcceptedOnce`, `OwnersUntilAck`, `OnlyAckReleases` and
`ParkCoversAll`. Negative controls reenter terminal retention, replay accepted
close or release its owner without acknowledgement. See the
[reactor evidence](../reports/audits/2026-10-22-security-deployment-close.md) and
[synchronous evidence](../reports/audits/2026-10-22-security-synchronous-close.md).

## Deployment output receipts

| Model state/action | Implementation | Qualification |
|---|---|---|
| `active`, `attempts`, `confirmed`, `ObservePrefix` | `deployment_output::Machine`, `Output`, `arm`, `advance` | One accepted moved send per output. Only a checked positive scalar receipt advances the offset and submits the remainder. Model prefixes are units; actual 4080-byte chunking, string capacity and frame codec use host/native tests. |
| `ProducerReceipt`, `ready` | TCP/IP `PendingSend`, `send_ready`, `Enqueue`, consuming reply | Real readiness comes from handshake/send-buffer state; zero enqueue retains the request, positive enqueue fences repetition. The model abstracts producer delivery, without TCP correctness or peer-delivery proof. |
| `EndUncertain`, `deadline` | expiry, invalid/service/observation result; existing PendingCall cancellation then logical socket close | Uncertainty never authorizes resending. Original five-second output budget persists across chunks. Model termination includes failure and expiry; actual close and physical cleanup are omitted. |
| `Park`, `Wake`, `RejectWait` | drain CQ, recheck all phases, wait for minimum remaining deadline | Wake alone advances no bytes. Binding/wait rejection aborts with all owners live. Model starts after admission, assuming bound sources, with two outputs/two prefixes; no whole-service fairness or wall-clock guarantee. |

Safe TLC checks require `PrefixesOnce`, `OriginalBudget` and `ParkCoversAll`.
Negative controls replay an uncertain prefix or restart a chunk's deadline.
Source/host/native evidence is recorded in the
[output audit](../reports/audits/2026-10-22-security-deployment-output.md).
Socket setup/close, real saturated buffers, rendering, dynamic admission,
allocator/map faults and kernel scheduling remain separate qualification work.

## Deployment controller observation

| TLA+ action/state | Rust implementation | Correspondence |
|---|---|---|
| `Bind`, `bound` | `deployment_control::Machine::start`, `PendingCall::bind_cq` | Accepted call, socket, reply kind, original deadline and logical reservation are stored before binding. Rejection aborts with the operation live. The model starts after admission with two pending calls and omits request preparation. |
| `Complete`, `Observe` | `PendingCall::poll`, `Machine::advance`, kind-specific reply decoding | A ready reply can be observed beside an unanswered call. Actual CQ entries are hints; polling adopts returned resources. Mapping/reply-shape failure releases those owners through existing runtime policy. The model omits result bytes, capability cleanup and rendering. |
| `Tick`, `Expire`, `submissions` | original five-second deadline, pending owner Drop | Cancel exactly once without replay. The model's calls share an initial start for its bounded pair; Rust captures a separate deadline before each request preparation. A timeout is uncertain, not proof the server did no work. |
| `receiverBacking`, `ReleaseReceiver` | moved request memory retained by controller; original kernel sponsor charge | The receiver may retain backing after caller timeout. Releasing an active logical reservation is not proof of physical release or refund. The model abstracts a separate receiver release action, with no hardware/allocator refinement claim. |
| `Park`, `RejectAdmission` | minimum input/controller deadline on default CQ; shared fail-closed wait policy | Wait covers all pending sources, not one selected controller. Rejected binding/wait retains every current owner through abort. TCP/IP setup/send/close and actual controller-server blocking are separate. |

`CharlotteDeploymentControl` rejects a serial wait while another response is
ready and a replay after timeout. It is a bounded safety projection of accepted
calls. It omits dynamic admission, request-byte budgets, response fairness,
server execution, kernel scheduling and physical teardown. Host budget/owner
tests and the native held moved-request fixture cover additional scoped paths;
see the [evidence](../reports/audits/2026-10-22-security-deployment-control.md).

## Deployment input isolation

| TLA+ action/state | Rust implementation | Correspondence |
|---|---|---|
| `Advance`, `cursor`, `healthyAge` | `deployment_input::Machine::advance` | A completed poll/append/rearm or terminal input is one turn. Scanning from the rotating cursor bounds intervening ready turns; omitted empty slots are skipped. This is a turn bound, not a wall-clock or whole-service liveness proof. |
| `buffered`, `BoundedBytes` | `RequestBuffer::buffered_len`, pool `byte_limit` | Sum of retained input prefix bytes. Four moved receive pages, allocator overhead and TCP buffers are separate. Production starts with empty slots; the model starts with four admitted inputs. |
| `deadline`, `OriginalBudget`, `Tick` | `Deadline::at`, `Input::remaining` | One abstract total budget captured on observed accept completion. Rust also has a separate header deadline. Neither budget restarts after progress or unrelated wakes. |
| `ProducerReady`, `bound`, `Park` | deferred TCP/IP accept/receive, `PendingCall::bind_cq`, default-CQ drain then owner polling | Producer and notification delivery are assumptions. The model omits TCP handshake/EOF transitions, binding preparation and CQ overflow; host producer-state tests and native socket probes qualify those separately. |
| `RejectWait`, `owner` | `reactor::wait` / binding failure `domain_abort` | Terminal abort retains the containing owners; no new submission or cleanup proof follows rejection. Kernel cancellation/physical teardown are omitted. |

`CharlotteDeploymentInput` is a new bounded safety projection. Its cursor
negative control starves a ready healthy input for more than four turns; its
deadline control restarts an existing budget. It covers no incoming admission
fairness, controller execution, response backpressure, explicit socket close,
whole-service shutdown latency or kernel scheduling fairness. See the
[implementation evidence](../reports/audits/2026-10-22-security-deployment-input.md).

## IPC

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `MemoryCreate` | `memory::object::allocate` and capability insertion | Abstract: frames, sizes, rights and mappings are omitted. |
| `EndpointCreate` | `ipc::endpoint_create` | Direct for owner, capacity, open state and endpoint capability. Interface/version and CQ notification binding are omitted. |
| `ConnectionMint` | `ipc::connection_mint` and `mintable_endpoint` | Direct for endpoint identity and attenuated rights. `mintable_endpoint` also accepts a `Connection` source capability, so re-delegation is a concrete superset of the model's endpoint-only precondition. |
| `ScalarSend` | `ipc::scalar_send`, `enqueue_scalar` | Direct for authorization, closure, queue capacity and enqueue. |
| `ScalarCall` | `ipc::scalar_call` | Direct for pending-call creation, internal reply-token creation and enqueue. The token identity remains kernel-internal while queued; `receive` installs the server-visible one-shot capability. |
| `ScalarCallMove` | `ipc::scalar_call_with_memory_move`, `memory::object::move_to` | Abstract: one attachment only; concrete rollback and mapping checks are omitted. |
| `ScalarCallCopy` | `ipc::scalar_call_with_memory_copy`, `memory::object::copy_to` | Abstract: contents and allocation failures are omitted. |
| `ScalarCallBorrowRead` | `ipc::scalar_call_with_memory_borrow_read`, `memory::object::lend_read` | Direct for distinct borrower, shared readers and reply-bound revocation. Rights and mapping checks are omitted. |
| `ScalarCallBorrowWrite` | `ipc::scalar_call_with_memory_borrow_write`, `memory::object::lend_write` | Direct for distinct borrower, exclusivity and reply-bound revocation. Rights and mapping checks are omitted. |
| `Receive` | `ipc::receive`, `PreparedReceive` | Direct for successful authorization, FIFO dequeue, kernel-captured sender identity and receiver-visible reply authority. Shared admission precedes dequeue/result writing; result-write failure drops speculative authority. The model omits pressure, staging, result bytes and retirement. Vector attachments are refined separately below. |
| `Reply` | `ipc::reply`, `PreparedReply` with loans; `complete_reply` without loans | Abstract for successful token consumption, loan revocation, result publication and observer wake; detached claims, partial failure and namespace leases are omitted. |
| `ReplyReturnMemory` | `ipc::reply_with_memory_move`, `PreparedReply` with loans; `complete_reply` without loans, `PreparedTransfer` | Abstract: source escrow/pin/close fencing, returned connection composition and concrete rollback ordering are omitted. |
| `CancelPendingCall` | explicit `ipc::close_cap`/`PreparedCancellation`; `cancel_queued_call` for serialized root cleanup | Abstract: successful explicit cancellation removes the pending record only after every loan completes; the model retains a completed record for invariant inspection. Claims, queued ownership retention, live-root leases, partial failure and failed-close capability retention are omitted. |
| `EndpointClose` | `ipc::close_cap` for `Endpoint`, queued-message cleanup | Abstract bulk transition over queued calls, capabilities and borrows. |
| `DomainTeardown` | `ipc::close_address_space`, repeated non-leasing `close_cap_serialized`, `memory::object::close_address_space` | Abstract bulk transition. Concrete root cleanup has drained live operation leases and retains lifecycle/IPC through its bulk loan cleanup; it must not acquire new leases under those guards. |
| `ObserveResult` | `ipc::poll_reply` | Direct for the unobserved-to-observed result transition. |

### IPC concrete-to-abstract state

| Model state | Concrete source |
|---|---|
| `capTable[asid][cap]` | Unified `capability` namespace plus the matching entry in `IpcRegistry::caps` or the memory-object capability table. |
| `endpoints` | `IpcRegistry::endpoints`; observers, interface/version and `notify_cq` are hidden. |
| `replyTokens` | `IpcRegistry::reply_tokens`; each token retains a bounded vector of `MemoryBorrow` owner/borrower pairs, and the model reduces the loan state to one memory-object identity. |
| `pendingCalls` | `IpcRegistry::pending_calls`; `None` maps to `NoResult`, `Some` maps to a result record. |
| `memObjects` | Memory-object owner and `LendState`; mappings, rights, physical frames and DMA pins are hidden. |

Call creation queues only an internal token identity. `receive` atomically
turns that identity into a capability in the endpoint owner's table. The
model's `delivered` bit records this linearization point; a live token has a
receiver-visible capability exactly after delivery. Cancellation and teardown
invalidate either queued identities or already delivered capabilities.

### IPC attachment transaction refinement

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `BeginVector` / `TransferMove` / `BeginBorrow` | vector parsing plus `prepare_move`, `prepare_copy`, and `prepare_loan` | Abstract preparation across the memory/capability registries. All destination authority stays hidden; copies own private charged backing, and loans have no live borrower state before commit. `Items` represents two or more vector memory attachments. The model omits copy backing admission and source escrow/pins. |
| `AttachConnection` | `scalar_call_with_connection*`, `PreparedConnection` in `PreparedCall` | Abstract staged connection from the scalar attachment ABI. The kernel owner composes connections with memory, including mixed-mode private fixtures; no public syscall carries both a connection and a full memory vector. |
| `Commit` | `PreparedCall::commit`, `commit_transfers_with_authority`, `publish_batch`, payload insertion/enqueue | Joint IPC/memory authority publishes atomically after complete validation; IPC/memory guards serialize the remaining payload updates. Scalar-only publication skips the memory registry. The model collapses these boundaries and does not prove staging/pin, admission, generation or namespace-retirement lifetimes. |
| `FailAndRollback` | `PreparedCall`/`PreparedConnection` field Drop; `PreparedTransfer` Drop; `PreparedReply::abort_prepared` and prepared no-loan reply authority | Moves/loans restore exact source slots without re-admission; copies release private backing; fresh call/grant reservations and family charges refund. Source escrow/pin/fence completion is serialized under the memory registry. No receiver alias needs unmap or reverse transfer. Successful reply loan revocations are removed before retry/cancellation; the model omits close waiting, partial revocation/publication failure and late retirement. |
| `Deliver` | `receive_vec`, `PreparedReceive` | Abstract successful delivery of the complete vector and reply authority. Shared rejection preserves queue/result bytes; result-write failure releases speculative reply authority without consuming the token/attachments. Encoding, admission failures and retirement are omitted. |
| `WaitTimeout` | non-terminal `ipc::wait_reply_timeout` result | Direct: timeout reports that no reply was observed; it neither closes the pending call nor ends its memory loan. The EL0 ABI now exposes `IpcReplyWaitTimeout` (86), backed by the same observable and deadline watchdog. `PendingCall::wait_timeout` retains its owner/borrow on timeout or waiter admission rejection; the latter reports `RESOURCE_LIMIT`, not timeout. The model omits waiter/watchdog admission and scheduler registration. |
| `Reply` / `ObserveReply` | `PreparedReply` for all existing borrowed-memory reply variants; `complete_reply` for no-loan replies; then reply wait/poll observation | Abstract for releasing the token's whole loan vector followed by observation/close. Concrete replies lease both roots and claim the record across IPC-unlocked revocation; call/reply/minting-source close waits outside IPC, and returned-memory source close waits outside its registry for escrow/pin completion. Hidden returned memory/connection reservations publish jointly before result visibility; the internal fixture's combined output does not add a combined wire API. The model omits claims, hidden grants, source delivery qualification, partial cleanup, lease admission and abandonment; its single loan state does not prove every vector loan's revocation. |

The retained unsafe rollback leaves one target-registry entry and its abstract
connection attachment after failure. The retained unsafe timeout releases a
loan while the pending call remains live. Both are required counterexamples in
the model runner.

### Captured admission and hidden delivery

`CharlotteIpcPublication` supplements the older atomic IPC and capability
models. Its single `st` record contains one transaction, the exact original
namespace/account identities, admitted records and detached charged nodes.
The generation labels distinguish namespace-budget instances; concrete tokens
also validate the retained budget's identity, not just a scalar generation.
`authority = Live` alone is insufficient for ordinary use: payload `delivery`
must also be visible. `source` and `target` retain their captured generation
when `current` advances to an unrelated successor at the same numeric ASID.

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `PrepareRecord`, `RejectStorage`, `Reserve`, `RejectReservation` | `PreparingRecord`, `PreparedReservation`, `reserve_prepared_captured` | Abstract fallible storage and record admission before payload publication. A successful reservation consumes its serial and original account charge; ordinary rejection publishes nothing. Allocator bytes, platform class and exact root admission are omitted. |
| `PrepareMemory` / `PrepareConnection` | `PreparedTransfer`, `SourceEscrow`, `PreparedConnection` | Abstract move/copy preparation and an already delivered minting source. Move authority enters escrow; a pin retains backing, and copy preparation owns private backing. Read/write loans, mappings, copy contents and explicit source-close claim waits are outside this projection. |
| `Commit` / `RejectCommit` | `publish_batch`, `commit_undelivered_transfers_with_authority`, `PreparedCall::commit_retained`; returned output publication in `PreparedReply` | The model checks validation of the complete mixed batch before joint authority/payload publication. Both output grants remain pending delivery. Concrete participating registry guards bridge the remaining allocation-free payload writes. A call's own pending/reply records, queue admission and fallible loan revocation are omitted. |
| `FinishPreparation` | consuming prepared-transfer completion/Drop after committed payload installation | Ends transient pins and unused preparation before the destination can receive/use the grants. The memory registry's local hold has ended; the enclosing IPC serialization remains a separate context. Detached source authority is explicitly disposed afterwards. |
| `AbortPreparation` | prepared-transfer cancellation, `SourceEscrow::rollback`, `Reservation` cancellation | Restores only an existing captured source slot, without fresh quota, including retired-but-present teardown authority. Cancels staged destinations into detached metadata, ends private preparation and retains serial consumption. Removed namespaces remain absent. Actual field Drop, backing-release rejection and outer guards are not proved. |
| `PrepareReceive`, `RejectReceive`, `ReceiveWriteFailed`, `CommitReceive` | `PreparedReceive::new`, result writing, speculative reply cleanup and `commit` | IPC's exclusive borrow hides speculative reply authority; the model represents it as staged. Rejection leaves the queued attachments hidden. Successful dequeue publishes the memory and connection together. Result bytes, one-way sends and root leases are omitted. |
| `Readiness`, `ObserveReply`, `RepeatObservation` | reply/CQ readiness, first `poll_reply` observation, later stored-result reads | A readiness hint leaves grants hidden. First observation delivers both once. Repeat observation changes no authority, including after application close. Applications must still use owned wrappers to avoid adopting the same returned numeric values twice. |
| `Use` / `ApplicationClose` | ordinary memory/IPC lookup and typed close | Both family payloads reject pending delivery even when unified authority exists. Close after delivery detaches its record; cancellation of the original call cannot reclaim already observed output. Child delegation and attenuation are outside this model. |
| `CancelHidden` | queued cancellation/endpoint cleanup and unobserved-result cancellation | Only the container's hidden output grants are consumed. A committed move is never restored to its sender. Confirmed payload retirement is assumed; loan cancellation, endpoint reference accounting and fallible physical release remain separate obligations. |
| `RetireNamespace`, `DetachNamespace`, `ReplaceNamespace` | budget retirement, complete namespace detachment and exact-generation registration | Retiring admission prevents fresh publication; existing escrow may restore for teardown. Detached nodes retain original charges. A new namespace may reuse scalar handles but old tokens cannot alter it. These adversarial namespace schedules abstract family-level fixture boundaries; they do not authorize bypassing a live root lease. |
| `ReleaseRetired` | `RetiredRecord::release`, namespace disposal, `SourceEscrow::finish_retired` | Node disposal precedes refund to the exact captured account. A committed move's detached source remains with its transient pin until preparation finishes. Physical backing charges and process-wide heap budgets are omitted. |
| `Fill` / `Abandon` | admitted pressure fixtures / terminal preparation retention | Ordinary records can fill the source account while escrow is live. Abandonment retains preparation; the model does not qualify active reservation/escrow Drop beneath arbitrary outer guards. |

Ten separately weakened specifications check both payload families' hidden
lookup, batch atomicity, consumed serials, exact successor protection, detached
node charges, readiness/result separation, repeated observation and rollback
at the namespace ceiling. This adds a bounded projection of C18 and selected
publication boundaries used by C07–C11; it supplies no physical cleanup proof.
The older models' omission statements below still apply to those older models.

## Endpoint observers

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `ArmReadiness` | `EndpointObservable::register_observer` | Direct: installs a waiter in `Endpoint::readiness_observers`. |
| `ArmCloseWatch` | `watch_connection_closed` | Direct: installs a completion observer in `Endpoint::close_observers`, with an atomic already-closed check. |
| `Send` | `enqueue_message` | Direct for the empty-to-readable event: the CQ wake is gated on that transition. Readiness observers are drained on every enqueue; close observers are never drained here, which is the property the model checks. |
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
| `OpenCq` | `completion::open_cq`, `open_cq_phys`, `open_address_space_with_cq` | Abstract: allocation and shared mapping are omitted. Concrete open replaces any existing queue and starts `work_generation` at zero, whereas the model opens a closed unowned slot and increments the generation. |
| `SubmitNoBuffer` / `SubmitWithBuffer` | `completion::submit`, `Completion::new`; `catten_rt::owned::ReadOperation::submit` | A successful submission begins in `InFlight`. Buffered submission also transfers the mutable borrow to the in-flight operation (`kernelOwnsBuffer`). The concrete submit path has no CQ-owner precondition and succeeds even when no queue is open. |
| `Complete` / `Fail` | `completion::complete`, `Completion::complete`, `post_to_cq`; `ReadOperation::wait` | Direct for the terminal transition, CQ publication, generation increment and notification. The terminal transition and CQ insertion occur under one completion-registry hold, so a concurrent poll or close cannot remove the capability in between. Only this terminal boundary releases the kernel's buffer loan. |
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
| `cqDrained` | Whether userspace has consumed the operation's CQ entry; this is deliberately separate from `OpState::Observed`. No per-operation bit is stored: the value is derivable only while the entry remains in the ring, so it does not survive ring consumption. |
| `cqRings.entries` | Entries visible in `CompletionQueueRing`. |
| `cqRings.backlog` | `CqState::backlog`. |
| `cqRings.gen` | `CqState::work_generation`. |
| `waiters` | Queue observer registration and queue-wide `last_seen_generation`, reduced to one reactor per CQ. |
| `kernelOwnsBuffer` | Whether an in-flight or cancel-pending operation can still access a submitted mutable buffer. |

`CharlotteCQ_buffer_unsafe.cfg` makes cancellation release the buffer
immediately. TLC must violate `NonTerminalBufferRemainsLoaned`, retaining the
use-after-cancel scenario as an executable negative regression.

`CharlotteTimedWait` splits work-generation publication from wake delivery,
which exposes the interval in which a timer can run after work exists but
before the ordinary wake is processed. `TimerFire` rechecks the registered
generation via `classify_timed_wait`, used by `wait_on_cq` and
`wait_on_cq_timeout`; a changed generation returns work rather than a false
timeout. (`block_until` rechecks the caller's condition, not the generation.)
`CharlotteTimedWait_unsafe.cfg` omits that recheck and must violate
`TimeoutObservedNoWork`.

### Fallible checked waits

`CharlotteCheckedWait` starts after request acceptance. `owner` and `loan`
represent the still-held operation and its buffer borrow; `submissions` stays
one throughout. `epoch` represents an exact source/claim episode, distinct from
the enclosing operation's terminal result. It is not a reusable ASID or TID.

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `BeginWait`, `PrepareWatchdog`, `PrepareSource` | `try_block_until_with_constraint`, timer `prepare_watchdog`, scheduler `prepare_wait`, and source `try_register_waiter` | Abstract admission phases with one original deadline. Time advances independently during preparation. Sponsor identity, quota amounts, weak observers, candidate nodes and partial allocator construction are omitted. |
| `RejectWatchdog` / `RejectSource` | `Error::WaitRegistrationFailed`; kernel completion `WaitAdmissionFailed`; ABI `WAIT_ADMISSION_FAILED` or IPC `RESOURCE_LIMIT` | Abstract unstarted wait rejection: explicitly dispose unused local wait storage while retaining the accepted operation and loan. The model keeps rejection distinct from timeout; rejection does not cancel or replay the operation. Concrete condition rechecks can return ready instead of rejection. |
| `Register` / `PublishPark` | scheduler `publish_wait`, the condition recheck in `try_block_until_with_constraint` | Abstract lost-wake boundary: register against a captured episode, recheck terminal state, early hint, changed episode and remaining time before parking. Concrete waker/thread-generation validation, IRQ policy and physical context switching are not proved. |
| `PublishTerminal` / `ChangeEpisode` | operation producer completion; checked IPC claim completion/rollback and source closure | Boundary assumption: a terminal result is durable, and an ended source episode notifies its registered interest. Changing a claim episode preserves the enclosing pending operation. This does not prove producer-LP deferred delivery or particular hardware completion. |
| `Wake`, `UnrelatedWake`, `Recheck` | wake/watchdog delivery and the scheduler's absolute-deadline loop | Every wake releases unused wait storage and rechecks the operation. A hint cannot adopt returned resources, establish quiescence or authorize replay. Rejection may be followed by another wait only while retaining the same operation. |
| `ObserveTerminal` | owned completion observation or `PendingCall::finish` after reply wait | Abstract confirmed terminal observation transfers returned authority once and ends the borrow. CQ readiness alone never performs this action. Cancellation is represented only after confirmation. |
| `Abandon` | inert preparation/operation retention; containing fail-closed policy | Retains the existing owner, loan and any wait storage. Concrete runtime Drop may attempt cancellation or abort the domain; those typed policies, active token disposal and outer guards remain outside this projection. |

The six negative configurations require their respective unsafe actions to
violate owner/loan retention, single submission, fixed deadline, result
qualification, park recheck and timeout classification. This is a safety model;
finite admission, producer progress and eventual disposal are not established.

## Deliberately omitted behavior

The models currently omit memory contents, concrete page-table mappings,
completion observers, detached operations, address-space validation failures,
and the byte-level layout of vector/result pages. Vector IPC, connection
attachments, multiple moved entries, reply-wait timeout, and cross-registry
rollback are covered by the transaction refinement above.

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
| `RequestRemoteAbort` | `abort_thread`, `abort_requested`, `abort_owner_lp`, scheduler IPI | Direct for cross-LP termination: the caller records the physical owner but leaves the executing context in the master table. Concrete `abort_thread_generation` also defers an off-CPU `Ready` target through this path instead of removing it immediately as `AbortNotRunning`; end state and safety are unchanged. |
| `RetireRemoteAbort` | `RoundRobin::next`, `retire_requested_threads` | Owner-LP transition after switching away. Run-queue selection and `add_thread` reject the requested generation, including block/wake races. |
| `AbortNotRunning` / `SelfAbort` | `abort_thread`, `take_element`, `stage_dead_thread` | Non-running contexts can be removed immediately; self-exit is staged while still on its stack and switches away before reaping. |
| `BeginDomainAbort` | `domain_abort`, `abort_address_space`, `abort_as_threads` | The concrete refinement holds the publication gate, records the current address-space generation, snapshots every owned thread, and retains the gate through the sweep. This is the linearization boundary for the model's bulk transition. |
| `Reap` | `reap_dead_threads` | Direct for post-context-switch destruction; the concrete stack-pointer check may defer a context again. |
| `DestroyAddressSpace` | `domain_exited`, `teardown_domain`, `close_user_address_space_handle` | Requires every master-table and deferred-dead thread owned by the ASID to be gone. `OnCpuHasLiveAddressSpace` checks the resulting safety boundary. |

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

`CharlotteThreadJoin` isolates the userspace join contract from the larger
scheduler state space. `spawn_thread_with_generation` returns the generation
captured before thread publication; `ThreadHandle` retains it; and
`observe_thread_exit_generation` passes it back to
`observe_thread_exit_with_generation`. A missing or recycled slot completes
immediately. `CharlotteThreadJoin_unsafe.cfg` restores TID-only registration
and must violate `ObserverMatchesCapturedHandle` after slot reuse. Generation
allocation uses `charlotte_lifecycle::claim_generation` and the scheduler's
single global atomic `try_update`; it starts at one and reserves `u64::MAX` as
the exhausted state instead of ever issuing it. Concrete exhaustion panics
(fail-stop) instead of returning the model's non-fatal `RejectExhaustedSpawn`;
the never-issued property is preserved. The safe model exercises
`RejectExhaustedSpawn`, while
`CharlotteThreadJoin_wrap_unsafe.cfg` wraps to generation one and must violate
`GenerationNeverReused`.

## Reusable address spaces and interrupt routes

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| Address-space `Allocate` / `CaptureHandle` | `register_user_address_space`, `AddressSpaceHandle` | Direct for recyclable numeric ASID plus monotonic software generation. The scratch-window allocator is keyed by ASID with a stored generation field, so a recycled ASID receives fresh allocation state. Unmapped ranges are recycled only after page-table removal and TLB invalidation. |
| Address-space `CloseExact` | `close_user_address_space_handle`; generation checks, lifecycle serialization, `RetiredAddressSpace` and `RetiredEntry` | Direct for rejecting stale handles. Logical subsystem cleanup remains lifecycle-serialized; a detached root leases its slot through post-guard invalidation/destruction before reuse. The atomic model omits this intermediate invisible-but-leased state, completion metadata, failed invalidation and quarantine. Public live map/unmap, explicit device close and direct loan revocation now retain operation leases through their own finish. |
| Hardware-ASID `Allocate` / `Retire` / `Invalidate` | AArch64 hardware-ASID allocator and TLB invalidation | Abstract: page-table contents are omitted; tag reuse is allowed only after invalidation removes stale translations. |
| Interrupt-route `Bind` / `QueueWake` / `Unbind` / `DrainSafe` | device interrupt binding, route generation, deferred wake drain | Abstract generation-fencing check for one route. The implementation now has independent atomic mailboxes with retained watermarks and a guarded CQ preparation step; mailbox capacity, atomic claim/publication, binding exhaustion and controller MMIO are not modeled here. |

### Root-retirement projection

`CharlotteRootRetirement` supplements the atomic `CharlotteAddressSpace` close.
`slot` is the generation currently holding one reusable numeric ASID;
per-generation records preserve terminal backing/charge history after reuse.
Two abstract pages represent complete versus partially rejected physical
release. `metadata` groups detached mailbox/queue, authority, limit, usage and
signed-identity storage; individual family counters and principal history are
not independently modeled.

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Allocate` | `PreparingRegistration`/registration publication and `PreparingMetadata` | Abstract successful root publication with its original metadata and charge. Fallible construction and signed identity installation remain outside the model. |
| `Acquire`, `FinishOperation`, `AbandonOperation` | `AddressSpaceOperation::acquire`, consuming `release`, inert `SlotLease` fallback | Captures the exact generation. Closing rejects new ordinary acquisition; abandoned leases retain their count. Special peer leases borrowed from a closing owner are omitted. |
| `BeginClose` / `ClosePending` | `ClosingAddressSpace::begin`, `RetirementProgress::Pending` | Fences new ordinary operation admission while retaining older leases and the complete closing owner. Thread quiescence is assumed before entry. Immediate busy-close rejection is covered by Rust fixtures, outside this projection. |
| `FinishCleanup` / `Detach` | `ClosingAddressSpace::prepare_retirement`, subsystem completion and root extraction | Devices precede IPC, then memory; zero leases and confirmed subsystem completion precede detachment. Each typed subsystem proof is an input, not proved by this model. Cleanup failure/abandonment retains the fence and owner. |
| `BeginInvalidation`, `InvalidationFailed`, `RetainForRetry`, `ClaimRetry` | `RetiredAddressSpace::release_retry_with_physical`, `recovery::retain`, `Registry::claim` and `Attempt` | Complete root/slot/metadata/original charges survive failure and owner transfer. Retry requires custody and no physical start. The bound counts whole invalidation invocations, abstracting initial versus controller attempts; fixed registry capacity, ticket serials, IRQ rejection and inner x86 acknowledgement epochs are omitted. |
| `InvalidationSucceeded` / `StartPhysical` | confirmed invalidation, explicit metadata release, `RetiredEntry::release_value_with` | Hardware acknowledgement precedes detached metadata disposal and physical teardown. The model assumes genuine acknowledgement and post-guard context; it does not prove hardware or allocator behavior. |
| `FinishPhysical` | `release_retired_with`, account retention/refund, `finish_retirement` | Successful physical release refunds the backing charge. Partial rejection retains the whole original charge with terminal backing, while the now-quiescent software slot can complete. No partial physical walk is replayed. Individual frame outcomes and interruption inside the walk are omitted. |
| `Abandon` | consuming cleanup error, rejected custody admission, or inert root/attempt fallback before physical start | Retains all represented fields with no controller re-adoption. Abandonment after physical start/status-publication races remains outside this projection. |

`ClosingAddressSpace::pending` now retains an inline diagnostic observation of
its actual Pending branch. Supervisor failure logs it with the captured root
before abandonment, outside local guards. Lease counts, selected IPC token/epoch
and memory pin/transfer/revocation fields are observations, with no transition,
release/retry authority or new model invariant. IPC selection precedes preparation
and can be stale after an episode change. This projection and its typed physical
completion assumptions remain unchanged; see the
[diagnostic evidence](../reports/audits/2026-10-22-root-retirement-diagnostics.md).

Five separate negative specifications expose detachment with live leases,
release without invalidation, physical retry after partial release, adoption of
an abandoned owner, and refund of retained backing. These supplement C01–C03
in the [cleanup coverage map](../reference/cleanup-recovery.md); they do not
qualify other owner families or close SEC-07/18. Existing omission statements
below refer to the older atomic models and to unmodeled concrete phases.

This lifetime model abstracts hardware walk/TLB quiescence. Concrete kernel
ranges now retain removed data in `RetiredKernelRange` through post-guard
invalidation; Drop quarantines and x86 delivery failure is fail-stop. The model
does not prove that protocol, recipient progress, or the still-open x86
user/device/domain masking-guard retirement paths (SEC-18).

Memory-object mapping retirement now retains a separate backing pin through
invalidation and fences borrower authority until completion. Partial detach,
failed rollback and abandoned receipts retain the creating generation's charge;
installed-prefix and physical-identity checks protect foreign leaves. These
concrete pin/quarantine and teardown-vector removal paths are not modeled by
the atomic lifetime actions. Concrete unmap/loan-revoke/domain cleanup now
prepare records under the registry and walk tables using fixed pinned-frame
batches after releasing it. The atomic actions do not model those lock phases,
batch bounds or their serialized guard-availability fixtures. They do not
discharge the remaining lifecycle/IPC
masking-guard or hardware-quiescence obligations.

Scratch admission now prepares a sorted live-extent vector fallibly before
reservation publication; release removes one exact entry without allocation.
Bulk retirement retains its backing pin and loan restrictions on any scratch
completion error. Six direct host allocator tests and a two-borrower guest
failure adapter cover these concrete paths. Atomic lifetime actions omit
virtual extents, metadata allocation, partial completion and quarantine counts;
this change is not an additional model proof or TLC run. Scalar extent matching
also does not supply an allocation nonce or a live-generation lease.

Concrete root/intermediate allocation now uses `PreparingTable` through
publication. Private/lower-half allocation preserves the physical progress
floor, while shared higher-half kernel tables may consume it. Local per-walker
fault adapters check partial linked prefixes, retry/reuse and exact inactive-tree
teardown. The atomic lifetime/load models omit preparation owners, floor policy,
hardware tags and table counts; this is not a new model proof or TLC result.

Final root close now detaches into `RetiredAddressSpace`: software-slot ownership
and the captured ARM hardware tag survive lifecycle/table guard release. Root
destruction follows invalidation; slot completion follows physical teardown.
Failed invalidation/abandonment retains hierarchy, accounts and slot. After
proven quiescence, owning-root `FrameRelease` attempts each physical release;
any rejection retains complete heap/image charges even though the slot/tag may
retire normally. Only fully successful release permits account refund. The
atomic model omits allocator errors, partial physical release and these retained
node charges; it does not prove their fail-closed accounting.
Heap/image provisional preparation now jointly owns the frame, reservation and
exclusive address-space borrow. Rejected release and uncertain publication
retain original domain/node counts; normal root destruction excludes those
quarantined pages from refunds. The atomic lifetime/load actions omit these
preparation phases, retained pool identity and partial physical outcomes. Fault
adapters validate serialized concrete states, not panic recovery or hardware
quiescence; no new model/TLC result is claimed.
Generic table identity and software generation fence completion tokens, whose
storage is preflighted before logical cleanup. The model's atomic close does not
prove those phases, and earlier IPC/MMIO/mapping lock-held invalidations remain
SEC-18 work. No new TLC result is claimed for this implementation change.

Concrete live operation leases now retain a table/slot/generation through
explicit completion. Root close rejects their nonzero count before namespace or
backing retirement; abandonment retains a live root. Host and guest fixtures
check identity, counter limits, vector growth, busy-close non-mutation and
retention. The atomic model has no lease counts, busy-close result or abandoned
live state; `CaptureHandle` alone does not model this retention. Production
mapping and MMIO paths use these leases through invalidation; explicit device
close releases lifecycle after detaching its device object. Direct loan
revocation composes both roots with a transaction owning the existing borrower
state and backing pin. Its failed/abandoned completion leaves `Revoking` and the
pin retained; ordinary failure explicitly completes root leases. New boot
fixtures check guard availability, both-root close rejection, staged close,
preparation rollback, remaining-reader preservation and four quarantines. The
atomic loan actions omit those phases, pin/fence state and retained charges.
Plain replies now compose both root leases, loan receipts and an exclusive reply
claim across IPC-unlocked detachment/invalidation/scratch completion. Close of
call/reply authority waits outside IPC; preparation rolls back only unstarted
receipts, ordinary failure retains uncertain backing, and abandonment retains
claim and live roots. Deterministic guest interleavings check these phases; the
atomic model does not represent the claim/wait or prove multi-LP progress.
Returned connections now retain delivered/observed minting sources under that
claim, own requester-sponsored hidden grants through publication and refund them
before lease release on ordinary failure. Source-close waits; unrelated endpoint
owner-close preserves the closed record via the existing delegated capability.
Queued/unobserved sources reject before preparation to exclude indirect cleanup.
Concrete tests cover those interleavings; the model omits their authority and
source-publication phases. Returned-memory replies now own source escrow, its
backing pin and hidden destination in the same leased operation. Source-close
waits outside the registry; rollback restores escrow and releases the pin under
one registry hold. Joint memory/connection publication precedes result
visibility; operation Drop may restore an unstarted output move but retains
uncertain input loans and reply/root claims. The internal combined-output fixture
does not add a combined wire API. The model omits source-close fencing and
atomic escrow/pin completion. Explicit call/reply cancellation now owns both
live roots, all loan receipts and an exclusive `completing` claim. Queue messages
stay in place while cleanup runs outside IPC; receive rejects Pending and
endpoint/call/reply close waits outside IPC. Ordinary cleanup failure retains
authority and uncertain backing without terminal publication; abandonment retains
claim, queue and roots. The runtime's explicit close returns its borrow-owning
call on rejection; failed Drop/error-wait close takes the non-returning domain
abort path. Bulk reply-token cleanup no longer reports uncertain revocation as
terminal, but bulk endpoint/domain paths remain serialized. The model omits
these claims, failure/abort states and partial completion; deterministic host/guest
tests are not a refinement proof or concurrent scheduling result.
Endpoint readiness now excludes claimed/failed queue fronts; queued cancellation
re-signals readiness/CQ after removal. The model's atomic cancellation omits
that temporary unavailable state and wake restoration. Failed cancellation
tokens cannot resume delivery or reply.
An owned staged-close request now fences new leases and returns its owner while old leases drain;
timeout/abandonment retains the closing state even after the last completion.
Host and guest fixtures check these serialized states. The model also omits
closing admission, linear close authority, pending polls, capacity refresh and
timeout retention; its atomic close does not prove this protocol. Production
controller now retains and polls a staged teardown owner; x86 rendezvous
progress remains open. No model source
or TLC result changed.

The August `memory_map_any` work did not change memory ownership in
`CharlotteIPC`; it changed address-space placement. Its safety-relevant part
is the generation-keyed scratch cursor and lifecycle serialization represented
by `CharlotteAddressSpace` (introduced across the August lifecycle-hardening
commits, not by the cursor-arithmetic repair itself), not a new IPC transfer
mode.

## Service lifecycle

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `StageTrusted` / `StageUntrusted` / `RejectUntrustedLoad` | signed service bundle/object-store staging; `verify_image_signature`, `try_load_domain` | Direct for the trust gate: unsigned, tampered, or artifact-mismatched bytes cannot reach address-space allocation/mapping. Cryptography and ELF parsing are abstracted to the trust bit. |
| `Load` | `loader::try_load_domain` | Abstract: bounded ELF layout/planning, image/runtime backing admission, heap commitment, partial-preparation rollback, fallible initial root creation, retained intermediate-table ownership/publication and finite hardware-ASID allocation are omitted. The model's atomic successful Load is not a proof of these concrete budgets, walk quiescence or failure paths. |
| `Start` | `start_domain`, `spawn_thread_on_lp` | Direct for the initial `(tid, generation)` domain handle. |
| `Prepare` | `NameCatalog::apply_command(CMD_REGISTER)` | Direct: increments the retained generation, records the owner, and stores an inactive entry. A replacement is intentionally unresolvable until activation. Exhaustion returns generation zero without changing the entry. |
| `PublishLocal` | node-local `ns::register` from DNS registration flow | Direct for installing the re-delegable local connection before distributed visibility. The local name service independently allocates and returns its checked generation. |
| `Activate` / `RejectStaleActivate` | `CMD_ACTIVATE` | Direct: only the exact prepared generation with a nonempty owner becomes active. |
| `Lookup` | `NameCatalog::lookup`, quorum-contact query path | Direct for filtering inactive entries/tombstones. Linearizable read-barrier mechanics remain in the Raft layer. |
| `FencedUnregister` / `RejectStaleUnregister` | `CMD_UNREGISTER_GENERATION`; local `OP_UNREGISTER_GENERATION` | Direct owner-and-generation fence. A delayed cleanup request cannot unpublish a replacement. The unsafe model removes this check and retains the corresponding counterexample. |
| `CleanupLocal` | `pending_local_unregistrations`, `LocalPublication::local_cleanup_submitted` | Abstract asynchronous cleanup, separately fenced by the node-local generation. |
| `RequestStop` / `Exit` | `ipc_connection_watch_closed`; service shutdown or `abort_as_threads` | Endpoint closure completes a retained kernel watch; the owner proposes the tombstone directly or sends a source-authenticated, retried request to the known leader. The catalog may remain briefly active while this asynchronous transition is in flight. |
| `DomainAbort` | Rust panic handler, fatal EL0 exception handling, `DOMAIN_ABORT` | Abstract spontaneous failure: scheduler retirement of the complete address space is collapsed into `Exited`; catalog cleanup, reaping, and teardown retain their ordinary fences. |
| `Reap` | `wait_domain_exit`, scheduler master/dead-table observations | Direct for the condition required before teardown. |
| `Teardown` | `teardown_domain`, `close_user_address_space_handle` | Direct for the reaping precondition and resource/address-space release. |

Concrete teardown treats a domain as exited only after every master-table and
deferred-dead thread with that ASID is gone. Looking only at the initial TID
allowed a secondary EL0 thread to enter SVC after its address space had been
removed; the strengthened check implements the model's domain-wide `Reap`
precondition.

The userspace Raft reactors combine a bounded CQ wait with a detached timer
that supplies the next election-clock tick. The bounded timeout is kept as an
independent watchdog, so a lost timer completion cannot freeze elections; the
code never performs an indefinite CQ wait plus a detached timer.

## DMA and SMMUv3 lifecycle

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `CreateMemory` | `memory::object::allocate` | Abstract: frame count and physical addresses are omitted. |
| `CreateDomain` | `device::grant_dma_domain`, `smmu::create_domain` | Direct for unique requester-stream ownership and domain authority. Page-table allocation is omitted. |
| `CpuMap` / `CpuUnmap` | `memory::object::map`, `unmap` | Abstract boolean projection of CPU page mappings. Any retained DMA pin blocks a fresh mapping; an existing coherent mapping remains valid. Concrete virtual addresses and permissions are omitted. |
| `BeginLoan` / `EndLoan` | `lend_read`, `lend_write`, `revoke_lend` | Abstract projection of IPC memory loans. A new loan is rejected while any DMA pin exists. |
| `BeginMap(..., "Coherent")` | unsafe `dma_map`, `memory::object::pin_for_dma` | The raw coherent-sharing path may coexist with CPU mappings or a previously established loan; synchronization remains the unsafe caller's obligation. It cannot coexist with an exclusive pin. |
| `BeginMap(..., "Exclusive")` | safe `dma_map_exclusive`, `OwnedMemory::begin_dma` | Direct ownership transfer: requires no CPU mappings, loans, or other DMA pins. The mode remains exclusive until acknowledged unmap and pin release. |
| `CommitMap` | `Domain::map`, `invalidate_asid`, successful return from `smmu::map` | Abstract: per-page PTE installation and IOVA arithmetic are collapsed. Publication occurs only after invalidation succeeds. A second mapping of the same memory object in one domain is rejected, preserving the model's per-domain uniqueness. |
| `FailMap` | confirmed partial-PTE cleanup or unstarted rejection, then `memory::object::unpin_dma` | Abstract confirmed rollback only. An error or an incomplete installation does not by itself prove rollback; uncertain started maintenance retains its pin. Admission/completion ownership is split in `CharlotteDmaOperation` below. |
| `QuarantineMap` | failed `invalidate_asid` after `Domain::map` | Direct safety policy: the unpublished internal mapping retains its pin because hardware translation state is uncertain. A later acknowledged domain destroy reclaims it. |
| `RevokeMap` / `ReleasePin` | `Domain::clear_mapping`, `invalidate_asid`, then `unpin_dma` | Direct ordering: translation removal is acknowledged before the object becomes reclaimable. |
| `BeginDestroy` | `smmu::destroy_domain` before the aborting STE is acknowledged | Abstract in-progress management state under the SMMU lock. |
| `AcknowledgeDestroy` | successful `write_ste(sid, None)` | Direct linearization point at which the requester stream is forced to abort and mappings may be consumed. |
| `QuarantineDestroy` | `destroy_domain` error return | Direct safety policy: the domain and pins remain retained when hardware acknowledgement is uncertain. |
| `ExitDriver` | `device::close_address_space`, `memory::object::close_address_space` | Abstract bulk teardown; pinned owned memory becomes `destroy_when_unpinned`. |
| `ReclaimMemory` | final `unpin_dma` for a destroy-pending object | Direct for last-pin removal, capability cleanup and frame reclamation. A destroy-pending object rejects further CPU mapping and DMA pinning, so no mapping can outlive the final unpin. |

`ExclusiveDmaHasNoCpuAuthority` checks that an exclusive pin cannot overlap a
CPU mapping, IPC loan, or second DMA pin. `CharlotteDMA_unsafe.cfg` removes the
exclusive precondition and must produce a counterexample.

### DMA operation and readiness projection

`CharlotteDmaOperation` follows one successful or failed map and its possible
unmap/CPU teardown. Its `root`, `claim`, `pin`, `sourceStorage` and `interest`
represent separate owners; `unit` holds the actual backend domain/engine
while their installed cells are absent. `backendAck` is distinct from the
runtime's `dmaConfirmed`. The captured root/pin/sponsor identity is a fixed
input, not an additional address-space reuse proof.

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `BeginMap` | `map_with`, `DmaOperation::begin`, original `PendingPin` installation | Abstract successful public admission with the same captured root/classification and memory pin. Early direction/capability/pin allocation failures are outside this projection. |
| `PrepareInterest`, `LinkInterest`, `ReadyDuringPreparation` | `domain_creation::readiness::admit_with`, `Source::prepare` and borrowed `Owner::interest` | Storage leaves local backend guards before preparation; readiness is rechecked before linking the exact source episode. The model assumes qualified post-guard execution and does not prove allocator or notification-list internals. |
| `RejectSourcePreparation`, `RejectContext`, `RejectWaiter`, `CleanupUnstarted` | source/waiter admission failure, qualified scheduler/IRQ checks and ordinary unstarted-pin cleanup | Failure retains the public root/claim and pin until explicit cleanup; source/interest disposal precedes public root completion. Masked ready paths may claim immediately, while busy waiting requires qualified context. No IRQ state is enabled by the model. |
| `Park`, `RestoreUnit`, `NotifySource`, `ObserveClosedSource`, `CompetingClaim` | owned unit subscription, complete unit restoration and producer-LP deferred delivery | Notification closes an old source episode after restoration. A competing claimant can win before wake; the original owner rechecks without rerunning work. Actual scheduler/lost-wake mechanics are abstracted separately in checked-wait models. |
| `ClaimMap` / `FinishMap` | backend admission callback returning `MappingMaintenance`, mapping maintenance and `Records::publish`/`quarantine`/ordinary rollback | Requires the actual engine and recorded domain cell together; work runs once. Confirmed rollback releases its pin, uncertain completion retains it in the registered backend. Hardware acknowledgements and backend-specific page walks are assumed inputs. |
| `CompleteMapPublic` / `RejectMapPublicCompletion` | `DmaOperation::complete`, `MapError`, runtime `begin_dma` / `map_shared_dma` | Ordinary completion releases the public claim/root after backend restoration. Rejected public completion retains them and returns no usable map success. A map error never creates replay authority. Failed shared preparation retains a restricted mapping even after ordinary pin rollback. |
| `BeginUnmap` | `DmaUnmap::finish` records `Retained` before calling `kernel::dma_unmap` | Both explicit finish and initial Drop fallback record their attempt before entry. The model bounds total submissions and tracks which attempt remains uncertain. |
| `RejectUnmapAdmission` / `ClaimUnmap` | typed `UnmapError::Admission` versus actual backend domain/engine extraction | Only pre-claim `OperationInFlight` can become ABI `OPERATION_IN_FLIGHT`. Unmap uses the rejecting path; this model introduces no unmap readiness or polling. |
| `FinishUnmap`, `CompleteUnmapPublic`, `RejectUnmapPublicCompletion` | typed backend completion, `unmap_with`, runtime `DmaUnmap` | Backend success releases the DMA pin; ABI success records runtime confirmation. Completion errors, including busy-looking backend/final-claim failures, do not rearm. Ordinary backend errors may release the public root while retaining quarantined pins and original backing charges. |
| `CpuUnmapFailure` / `CpuUnmapSuccess` | `SharedDmaMemory::finish`; exclusive `DmaTransfer::finish` returns unmapped memory directly | Confirmed DMA completion persists through failed CPU teardown. Shared retry performs only CPU unmap; exclusive return needs no CPU unmap call. The model's `Owned` state is an unmapped CPU owner, not an exposed Rust slice. |
| `FailedSharedCpuUnmap`, `PrepareLocalDrop`, `DropLocal`, `ReleaseBacking` | `FailedSharedDmaMemory::unmap`, runtime Drop, kernel pinned-memory retention | Local teardown can close CPU access and request memory close without discharging an uncertain DMA pin. Confirmed release is an input; allocator failure and actual field-destructor ordering remain outside the model. |
| `FreshCpuMap` | `memory::object::map_locked_with_cleanup` after `FailedSharedDmaMemory::unmap` | Every DMA pin fences a fresh CPU mapping, including coherent pins retained after uncertain map failure. Existing coherent CPU mappings precede pinning. The [remap regression](../reports/audits/2026-10-18-coherent-dma-remap-fence.md) reproduced the missing gate before correction. |
| `RejectRootClose` / `Abandon` / `AbandonExternal` | operation leases, inert `DmaOperation` / `Maintenance` / `PendingPin` fallback, abandoned unit holder | Existing complete owners and fences remain retained. The abstract diagnostic close probe grants no quiescence or recovery permission. Grant creation/cancellation and wider root/IRQ contexts remain separate. |

Thirteen required negative specifications cover readiness, context, retained
owners, map replay, unmap admission/completion classification and local CPU
cleanup, including a fresh CPU mapping after consuming the restricted wrapper. This projects selected C13/C14/C18 boundaries in
[cleanup/recovery](../reference/cleanup-recovery.md). It does not qualify a new
custody family or close SEC-07/18; older pinning-model omissions remain relevant.

## Raft election and durable voting

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `StartElection` | `RaftNode::start_election` | Abstract atomic transition. Rust persists the incremented term and self-vote as two durable writes before sending vote requests; a crash between them leaves a durable term/vote pair the model cannot represent, but never a second vote in one term. |
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
| `AppendLeader` | `submit_command` (`handle_client_command`), `LogStore::append` | Direct for a durable leader append. Client response and state-machine application are omitted. |
| `ReplicateOne` | `handle_append_entries`, `LogStore::truncate_from`, `append` | One-entry projection. A matching entry retains the existing suffix; a conflict truncates from that index before appending. The store flushes each mutation before returning, so a conflict repair is two durable writes rather than the model's atomic replacement. |
| `CommitLeader` | `advance_commit_index` | Direct: a configured-voter majority must contain the index, and Raft advances by counting only an entry from the leader's current term. |
| `PropagateCommit` | follower `handle_append_entries` update of `commit_index` | Direct for `min(leader_commit, last_new_index)` after prefix validation. |
| `Crash` / `Restart` | service-domain exit and `RaftNode::new` with its `LogStore` | Durable logs survive. The model conservatively retains commit knowledge; concrete restart currently reconstructs it from snapshot/application progress and subsequent leader messages. |

The second layer checks log matching, committed-entry agreement, and leader
completeness under bounded conflict repair and restart. It relies on the first
layer for election safety rather than reimplementing durable one-vote-per-term
inside the log model. Snapshots and joint membership are checked by separate
layers. Failed storage operations, state-machine application, transport
framing, and temporal liveness are outside this abstraction.

## Raft response term and progress fencing

`CharlotteRaftResponse` isolates asynchronous acknowledgement handling from the
log model. Its fixed schedule captures an honest term-one acknowledgement for
index three, replaces the local prefix under term two, then elects this node in
term three and appends a new no-op at the same index. Election legitimacy and
prefix selection are assumptions from the other Raft layers.

| Model action | Rust implementation | Correspondence |
|---|---|---|
| `CaptureOldReply`, `ReplacePrefix`, `Reelect` | delayed `RpcCompletion`; follower conflict repair; `become_leader` | The old response describes different bytes. New leadership resets follower match indices and contact evidence. The exact reproduced Rust trace is retained in the audit evidence. |
| `Replicate`, `Acknowledge` | append handling, completed snapshot reply, `record_replication_progress` | Current-term successes retain the maximum confirmed index and keep next-index above it. Actual follower prefix agreement is assumed for a truthful response. Only append replies update the concrete contact clock; the model's contact abstraction represents that path. |
| `Failure` | rejected current-term append response | Backoff cannot cross the confirmed prefix. An older-term rejection cannot back off current work. |
| `RejectOld`, `RejectInvalid` | exact term check, `accepts_replication_response`, local-tail check | Reject old terms, mismatched/self/unadmitted sources and success beyond the local tail. Committed pending joiners are admitted for replication before promotion; they do not supply voter quorum by that status. Detailed field parsing and transport authentication remain outside the model. |
| `Commit` | `advance_commit_index` | A fixed self-plus-one-voter quorum can commit the current-term no-op only after that voter has the current prefix. Learner/joiner acknowledgements cannot supply its second vote. General and joint quorum selection remain in the other models. |
| `ObserveHigherTerm` | qualified higher-term response and `step_down` | Leadership and contact evidence clear. Higher terms from an unadmitted or mismatched source are rejected before this transition. |

Four controls retain the old-term unsafe commit, regressed success, backoff
below confirmed progress and invalid-response credit. The invalid-source control
is a coarse admission abstraction; field-by-field rejection is tested in Rust.
Snapshot chunk offsets, request/session correlation, removal/re-admission epochs,
Byzantine peers, clock/lease bounds, election liveness, persistence and allocation
are not proved. In particular, accepting only this term's responses does not
establish that the existing receive-time contact lease is linearizable.

## Raft membership and joint consensus

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Elect` | `has_election_majority`, `become_leader` | Membership projection of election: a candidate must obtain both current and next voter majorities while joint. Durable voting remains in the election model. Both the model and `ClusterConfiguration::is_voter` admit current or next voters as candidates. `ElectNextOnly` supplies separate positive coverage for the next-only path; both joint voter majorities remain required. |
| `SubmitJoint` | `submit_joint_configuration`, `submit_command` | Direct for appending the encoded `JOINT` command while the old configuration is authoritative. |
| `Replicate` | append-response handling and `match_index` | Abstract replication progress for configured voters and learners. Concrete append and completed-snapshot successes now require a matching admitted peer, exact current term and an index within the local tail. Progress retains its maximum and append rejection respects its floor; `CharlotteRaftResponse` checks this boundary. Commit counting reads configured voters. Log contents and conflict repair remain in the log model. |
| `CommitJoint` | `advance_commit_index`, then `apply_configuration_command(Joint)` | The `JOINT` entry commits under its preceding configuration; applying it activates the old/new union and records the finalization fence. |
| `SubmitFinalize`, `RejectLaggingFinalize` | `maybe_auto_finalize_joint_configuration` | Restored Graft Java/Rust automatic catch-up policy: JOINT is committed and every proposed remote voter/learner has reached its fence. The leader owns that prefix locally. Explicit/manual FINALIZE submission is outside this automatic-path projection. |
| `CommitFinalize` | `advance_commit_index`, then `apply_configuration_command(Finalize)` | Requires both voter majorities while joint, installs the next configuration, and decommissions nodes absent from the resulting voter/learner set. A node retained only as a learner becomes a follower immediately, matching the model. It remains a member and keeps its durable vote for the existing term. A captured joint-quorum result is checked even after the model replaces the old voter set. |
| `Crash` / `Restart` | service-domain exit and `RaftNode::new` | Abstract volatile availability. Durable configuration recovery is checked in the snapshot layer. |

The [Graft comparison and correction](../reports/audits/2026-10-18-raft-membership-parity.md)
records upstream revision `1a818d416a40330bea2055e03f61580679baa9f6`, narrow
source extracts and the Charlotte regression. Three negative controls cover
automatic finalization before catch-up, retaining a learner as leader and
committing FINALIZE with only the old quorum. The source regression preserves
its existing-term vote; the membership model does not model durable voting.

The model represents peer identity and role as voter/learner sets. Peer
addresses, protobuf encoding, transport retries, and temporal availability are
outside this abstraction. Pre-membership join admission is checked separately.

## Raft per-read fences

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Elect` | `become_leader` | Abstract election with inherited state and an uncommitted no-op. Election validity remains a separate proof. |
| `AdmitRead` | `begin_read`, `PendingRead` | Prepares query and exact domain identity, then appends a fresh no-op. The owner captures term, index and membership epoch. Rust uses a lazy fallible `Arc` identity; it is not reconstructed from scalar IDs. |
| `CaptureReply`, `ReceiveOldPrefix`, `DeliverReply` | append response creation/delivery and existing term/progress fencing | Separates reply creation from delivery. A response below a new fence cannot commit it. A quorum that already confirmed the fence after admission may reply later. The ghost proof records this distinction. |
| `CommitQuorum`, `Apply` | `advance_commit_index`, `apply_committed` | Abstract current-term quorum commit and application; quorum/log validity comes from other models. |
| `Read`, `RejectRead` | `read_ready`, consuming `handle_client_query(PendingRead)` | Requires the same node/domain, term and membership epoch, commitment through the fresh index and application through current commit. Readiness never executes the query. Compacted fences use the current committed anchor's term. |
| `CancelRead` | drop/timeout of a pending read | Abandons query/result authority. Its replicated no-op may commit later; no business query executes or replays. |
| `ChangeLeader`, `StepDown` | unknown external election / locally observed term change | The unknown change cannot make an old response prove a new index. Observed step-down rejects the captured owner. |
| `ChangeEpoch`, `ReplaceDomain` | membership change, different node instance or `begin_joining` | Captured identity rejects even when scalar indices coincide. Arc ownership prevents an old token matching a reused allocation. |
| `UnsafeReadBeforeCommit`, `UnsafeReadBeforeApply` | omitted prerequisite controls | Violate `ReadHasCurrentCommit` and `ReadHasAppliedCommit`. |
| `UnsafeReceiveLease`, `UnsafeReadBinding` | retired receive-time query gate / omitted owner checks | Violate `ReadHasFenceProof` and `ReadBoundToAdmission`. |

The [read-fence review](../reports/audits/2026-10-19-raft-read-fences.md) records
9,916 safe states, fourteen required actions and all four controls. DNS retains
each request, fence and original monotonic deadline in one bounded operation.
Placement lookup returns metadata, with no local capability grant or attempted
re-delegation of an NS-returned SEND/CALL connection. Invocations begin only
after routing and deadline checks, using the existing owned NS lookup/call
phases on the shared CQ. Queue admission precedes the no-op; rejected invocation
readiness binding retains its established fail-closed policy. There is no
new per-read timer or blocking reply wait. Existing raw transport/readiness
contexts remain separate qualification work.

The former `ReadHasAuthority` diagnostic is retained in historical evidence.
The current model checks quorum proof after request admission; it does not
require the old node to know about a later election at response time. Clock
conversion, syscall readiness, heap admission and byte decoding remain outside
the model. Status/ingress bounded observations still use receive-time freshness;
this batch does not qualify those leases.

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

Dynamic admission runs in the DNS-owned Raft node, whose disk-backed log and
persistent-state store also hold the catalog and cluster events. The generic
network-disabled Raft process remains only as a storage-recovery fixture.
Volatile nodes must still refuse auto-join because a process restart cannot
preserve the admission fence.

## Raft snapshot installation and recovery

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `BeginReceive` / `ReceiveChunk` | `handle_install_snapshot`, `PendingSnapshot` | Abstracts bounded chunk accumulation. Exact offset and sender/term binding are checked by `CharlotteRaftStream` and Rust regressions below. |
| `DiscardStale` | early completed response from `handle_install_snapshot` | Direct: an index at or below `commit_index` is acknowledged but cannot replace newer state or move progress backwards. The concrete early return also clears any pending snapshot buffer, which the model leaves unchanged. |
| `PersistSnapshot` | `LogStore::install_snapshot`, `DiskLogStore::persist_log_state` | The durable boundary, bytes, current/next membership, and compatible suffix are one serialized object-store replacement. The object store publishes it copy-on-write after data and metadata reach stable storage. |
| `ActivateSnapshot` | commit/last-applied update and membership reconstruction after successful `StateMachine::restore` | Application restore rejects before durable snapshot publication; the model's split represents successful logical activation after persistence. Application rejection and temporary restoration before a failed write are checked separately by `CharlotteSnapshotRestore` below. Membership activation also recomputes local decommissioning. |
| `Crash` | service-domain exit | Pending chunks and volatile state-machine contents disappear; the atomically published log-state object survives. |
| `Restart` | `disk_raft::open`, then fallible `RaftNode::new` | The tagged state/log pair validates jointly before construction restores snapshot bytes and current/next membership and exposes its index as committed and applied. Incomplete/unsupported pairs or rejected application snapshots stop recovery without migration or reset. Membership is only restored when the snapshot payload decodes to a non-empty `current_members` envelope; locally produced snapshots always satisfy this. Application validation is abstracted separately by `CharlotteSnapshotRestore`. |

The model abstracts snapshot contents to one value, membership to peer-ID sets,
and chunks to a bounded count. It omits peer roles and addresses within those
sets, checksums below the object-store interface, network framing, storage
exhaustion, and state-machine-specific validation of snapshot bytes.

## Raft snapshot stream ownership

[`CharlotteRaftStream`](CharlotteRaftStream.tla) isolates sender/term and offset
checks from the durable snapshot model. Both senders share one snapshot boundary;
matching that boundary cannot establish continuation ownership.

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `BeginReceive`, `RestartOffsetZero` | offset-zero `PendingSnapshot` construction in `handle_install_snapshot` | Captures leader ID, RPC term, boundary and owned byte buffer. A zero-offset request replaces the previous partial buffer. Boundary is fixed in this model; Rust tests exercise mismatched index and term too. |
| `ChangeTerm`, `RecognizeLeader` | `start_election`, `step_down`, accepted append/snapshot leader recognition | A local election or higher-term observation discards partial bytes. The model combines higher-term observation and optional leader recognition; recognizing a leader after a local election is separate. |
| `ReceiveChunk` | exact leader/term, boundary and offset checks before append | Two unit chunks represent prefix and final suffix. Installation requires one owner and offsets zero then one. Application validation and durable activation belong to the other snapshot models. |
| `RejectStale`, `RejectLeader`, `RejectOffset`, `RejectBoundary` | early term/leader rejection and unmatched pending-buffer checks | Abstract observations preserve the current buffer; field-by-field rejection is exercised by Rust tests. These observations do not establish authentication or transport provenance. |
| `CancelReceive` | stale completed snapshot discard, join reset or node destruction | Abstract local buffer disposal, without capability or hardware release. |

The safe instance explores **400 distinct states**; all ten required actions
have positive total coverage. Removing episode invalidation and the continuation
binding produces `InstalledSingleOwner`; accepting a wrong offset produces
`InstalledInOrder`. Both controls require `ReceiveChunk` coverage.

This model assumes an immutable byte image within one sender/term/boundary.
There is no shared stream ID or digest in the current wire grammar. Same-episode
image replacement/retries, authenticated identity, heap admission, actual bytes,
device persistence and concurrent raw transport remain separate qualifications.
The [source review and evidence](../reports/audits/2026-10-19-raft-snapshot-streams.md)
retain the failing mixed-byte installation and cross-port source comparison.

## Application snapshot acceptance and rejection

`CharlotteSnapshotRestore` isolates the rejection boundary from the chunk/suffix
model above. Its `exposed` flag means the calling Raft operation can return or
continue with the node; it is not a global endpoint-availability assertion.

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `ReadHeader`, `ParseSections`, `RejectUnsupported`, `RejectMalformed` | `NameCatalog::restore_bytes` | Unsupported format or any malformed section rejects before live publication. Temporary section maps remain local. Exact bytes, bounds, signatures and field dependencies collapse to a verdict; the host decoder tests qualify this abstraction. |
| `Validate`, `Restore` | final length check and collection replacement in `restore_bytes`; `StateMachine::restore` | All validation precedes live replacement. Several section locks remain distinct; the model assumes serialized Raft execution, not concurrent-reader isolation or fallible allocation. |
| `Persist` | `handle_install_snapshot` after successful restore; `DiskLogStore::install_snapshot` | The existing whole-object replacement is assumed. Chunk ordering, term checks and suffix selection belong to the separate model. |
| `DefinitePersistenceFailure`, `AmbiguousPersistenceFailure` | `persist_bytes`, `persist_log_state`, `persist_fail_stop` | Write/flush rejection calls `thread_exit` and never returns to publish progress. The model allows either the old or whole new durable object after failure, then requires fresh recovery. No new storage failure injection, global domain abort or physical quiescence is proved. |
| `ActivateStreamJoin`, `ActivateStreamRetained` | progress/membership updates and join-fence clear in `handle_install_snapshot` | Only a restored, persisted snapshot completes the reply. The finite candidate is newer than the join-admission snapshot; membership excluding self retains admission. |
| `ActivateRecoveryJoin`, `ActivateRecoveryRetained` | `RaftNode::new` | Restore must succeed before `Ok(Self)` exposes progress or clears admission. The recorded admission index is one: restoring that same old snapshot cannot complete it, even when membership includes self. |
| `Crash`, `BeginRecovery` | stopped caller/service restart, fallible node reconstruction | Volatile progress is reconstructed from durable bytes and join admission. Decoder/context verdict may differ on a later process start; this is not spontaneous disk corruption or an in-place migration. |

Join completion and retained admission each have separate required action
coverage in streamed and recovery paths. The preceding 124-state instance
accidentally excluded join clearing through boolean-assignment precedence; the
[response/coverage follow-up](../reports/audits/2026-10-18-raft-response-fencing.md)
records the correction and fresh checks.

The six rejected-publication controls cover application, durable snapshot,
progress, membership, join admission and recovery exposure; the seventh publishes
a snapshot before successful restore. Each must fail its named invariant.
Term/vote changes and leader-contact processing may precede application rejection
in Rust and are deliberately outside the preserved-state tuple. The no-snapshot
constructor path, raw membership envelope parser, election/log replay, concurrent
queries, allocator failure and endpoint teardown remain outside this model.

## Cluster placement, readiness, and DSR ingress

`CharlotteClusterIngress` composes already committed Raft application state
with its asynchronous projection into the frame router. It deliberately does
not duplicate log replication: `PublishReady`, `ReplaceDeployment`,
`CommitDrain`, `BeginJoint`, and `FinalizeJoint` begin at their state-machine
application linearization point. `CommitIngressAssignment` likewise begins
after signature, role, cluster-id, and trusted-UTC admission and represents the
catalog application of the resulting Raft command.

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `PublishReady` / `WithdrawReady` | `NameCatalog::apply_command(CMD_REGISTER)` (dispatched by `StateMachine::apply`), activation and generation-fenced unregister; `NameCatalog::ingress_placement` | Abstract across prepare/activate. The resulting ready node must be both a desired replica and active for the exact deployment generation. |
| `ReplaceDeployment` | deployment application in `NameCatalog`; leader `reconcile_replica_placements` | Abstract for a committed generation and sorted concrete replica set. Artifact verification, descriptor decoding, and placement ranking are outside this model. |
| `CommitDrain` | application of a signed shutdown intent; `NameCatalog::ingress_draining_nodes` | Direct for adding a generation-bearing drain decision to applied catalog state. Signature, expiry, and trusted UTC checks precede this boundary. |
| `CommitIngressAssignment` | `NameCatalog::apply` for `CMD_INGRESS_POLICY`; `effective_ingress_bindings`; frame-router and TCP/IP assignment polling | Abstracts the signed multi-service table to one service and an assigned/withdrawn Boolean. The monotonic sequence corresponds directly to the catalog replay fence and is hashed into each service epoch, so a policy replacement is visible even when placement, readiness, and membership inputs are unchanged. |
| `BeginJoint` / `FinalizeJoint` | application of Raft `JOINT` and `FINALIZE`; `active_voting_members` | Abstract membership projection. Quorum and durable ordering are established by `CharlotteRaftMembership`; this layer checks the old/new voter intersection during joint consensus and the final voter set afterward. |
| `LearnRoute` / `ForgetRoute` | `RelmsgRaftTransport::mac_for_peer`; discovery updates | Abstract availability of an authenticated node-to-MAC route. The trusted-L2 assumption and frame authentication are not proved. |
| `InstallSnapshot` | `RaftNode::can_serve_bounded_read`, `ingress_membership_snapshot`, `BackendSnapshot::new_with_members`, `MembershipClient::poll`, `SnapshotHistory::install` | Direct for a cluster-fresh, all-or-nothing materialization and immutable bounded history. A leader needs quorum contact; a follower needs a recent successful leader-log match. Bindings may outlive history entries but remain pinned tombstones. The router holds at most one membership request in flight per service and installs each response in request order, so an older snapshot cannot replace a newer one; re-installing the current epoch is a content no-op that renews the lease. |
| `ExpireLease` | terminal observation of the monotonic `snapshot_lease` timer | Abstract timing correspondence. Weak fairness says an unrenewed lease eventually expires; Rust fixes the interval at five seconds. |
| `StartNewFlow` | `classify_ingress`, `snapshot_for_flow`, `FlowEpochTable::observe`, `select_backend` | Direct for deterministic selection from a complete snapshot while its five-second monotonic lease is fresh. Each successful one-second refresh replaces the lease; timer failure or expiry stops unbound-flow admission. |
| `ExistingFlowPacket` / `EndFlow` | `FlowEpochTable::observe` | Direct while the binding's snapshot remains retained. FIN/RST removes the binding. Timing, TCP state, table-pressure eviction, and packets arriving through a different advertiser are omitted. |
| `DropWithdrawnFlow` | `reconcile_ingress_services` after an assignment withdrawal | Direct: a committed withdrawal drops the service's retained snapshots and flow bindings, so established flows fail and a reconnect selects from live assignments. |
| `DropDepartedBackend` | `remove_absent_backends` on snapshot install | Direct: a binding whose selected backend left the installed member set is removed, so a reconnect can select a live backend instead of black-holing. |
| `DropStaleNewFlow` | `snapshot_for_flow` with an expired or unavailable `snapshot_lease`; freshness-aware `local_advertises_vip` | Direct. Bound traffic may retain an old policy, while unbound packets and VIP ARP requests fail closed after lease expiry. |
| `DropUnretainedFlow` | `snapshot_for_flow`; `remove_absent_backends` retains bindings whose epoch is missing | Direct. Missing history returns `FlowPolicyError::MissingEpoch`; there is no current-snapshot fallback. |
| `UnsafeStartStaleFlow` | former unconditional use of `SnapshotHistory::current` | Negative regression. It demonstrates why an expired local policy must not authorize an unbound flow. |
| `UnsafeFallbackExistingPacket` | former `history.get(epoch).unwrap_or(current)` | Negative regression. It demonstrates why an unretained binding cannot be reinterpreted through the current snapshot. |
| `UnsafeAdmitStaleReadiness` | missing generation comparison, retained only as a negative regression | Negative model only. Current `ingress_placement` compares each active registration's deployment generation with the desired generation before adding the node. |

The model parameterizes both selections: a policy names exactly one advertiser
drawn from the non-draining members, and each flow is pinned to one backend of
its policy version's eligible set. The concrete leader-preferred advertiser
and rendezvous-hash winner are valid instances, so the protocol obligations the
model checks (one advertiser per version, a stable pinned backend) apply to
them; only a determinism test is needed for the concrete functions. The model
ends a binding when the committed assignment is withdrawn or when the pinned
backend leaves the installed member set, matching the router's hard-stop
behavior; `CommitDrain` remains the graceful path that retains established
flows while excluding the node from new selection. Snapshot install order is
guaranteed by the router's single in-flight request against monotonic applied
state rather than by a numeric epoch comparison, and the signed assignment
sequence is part of the service epoch digest. Platform-compatibility services
have no per-service assignment and keep the reduced membership/drain digest.

The model treats a policy version as the complete identity of one immutable
snapshot. Rust compresses membership epoch, the signed assignment sequence,
deployment and service generations, eligible nodes, and drain generations into
a 64-bit digest.
Collision resistance of that digest is outside the TLA+ claim. The model also
does not claim instant convergence among routers or globally unique VIP
advertisement during ARP and leader handover. Lease freshness is one abstract
Boolean plus weak fairness, rather than a proof of the five-second timer. A
later refinement should add advertiser failover and packets that move between
ingress participants.

## Cluster placement feedback control

`CharlottePlacementControl` models the leader-local controller before its
output enters `CharlotteClusterIngress` as `ReplaceDeployment`.

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Observe` | `filter_capacity_report`; `CapacityFilter::observe` | Direct for separating raw reports from the filtered placement value and rejecting duplicate/decreasing epochs. The model uses alpha 1/2; Rust uses integer alpha 1/4 and resets on boot-incarnation or usable-memory change. |
| `SelectCandidate` | `resolve_descriptor_assignments_with_capacity` | Abstracts deterministic bucket and rendezvous ranking to selection from filtered, feasible nodes. |
| `Tick` | `RaftNode::millis`; `ReassignmentGate::ready` | Abstracts monotonic dwell and cooldown progress. Model ticks are logical units; Rust uses 30-second dwell and 60-second cooldown. |
| `PressureMove` | `ReassignmentGate::ready`, then `encode_reassign` and `submit_command` | Direct for refusing pressure-only movement before dwell or during cooldown. Submission records the cooldown; catalog application supplies the generation fence. |
| `RemoveEligibility` / `ForcedMove` | `placement_nodes`; the forced branch in `reconcile_replica_placements` | Abstracts committed drain and membership loss. An invalid current node bypasses dwell, but the resulting assignment still enters through Raft. |
| `Admit` | `capacity_with_reservations`; `reserve_node_capacity` | Direct for reconstructing other deployments' promises and charging each selected instance against static and dynamic headroom. |
| `UnsafeEarlyPressureMove` / `UnsafeAdmit` | Retained negative models | Demonstrate oscillation-prone actuation and cross-release overcommit when the corresponding guards are removed. |

## Remote-call identity and uncertainty

| TLA+ action | Rust implementation | Correspondence |
|---|---|---|
| `Start` | `dns::OP_CALL`, `InFlightCall` | Captures caller DNS session, call ID, expected peer and replicated target generation before dispatch. The ID is monotonic until it wraps at `u64::MAX` and resets to 1. |
| `ReplaceTarget` / `RejectStale` | `NameCatalog` generation transition; inbound `rcall` validation | The target executes only when the request generation equals the active catalog generation; otherwise it returns `ERR_STALE_GENERATION`. |
| `Execute` / `DuplicateRequest` | inbound `TAG_REQUEST`; `CompletedCall` cache | First delivery invokes the local endpoint and caches its result; the same caller/session/call identity reuses the cached result. |
| `QueueReply` / `DeliverReply` | `TAG_REPLY`; relmsg acknowledgement counter | Abstract split between application result creation, reliable-message delivery, and client reply completion. |
| `Timeout` | `REMOTE_CALL_TIMEOUT_MS`, in-flight expiry | Direct: once dispatch may have executed, expiry returns `ERR_UNCERTAIN`, not a retry-safe transport error. |
| `SettleTransport` / `Evict` | per-peer relmsg reply-ACK ordinal and bounded result window | Direct for transport settlement: Rust evicts only an entry whose reply ordinal is covered by that peer's acknowledged reply count. The ordinal is reserved when the reply is actually sent, so it matches the transport's per-peer FIFO send order. The `ERR_BUSY` gate is also triggered by pending executions, before evaluation of settlement. Explicit uncertain-session retirement remains a modeled extension. |
| `RetireUncertainSession` | modeled extension only | No Rust counterpart: `expire_remote_calls` drops the in-flight record and returns `ERR_UNCERTAIN` without recording a retirement marker. |

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
| `Allocate` | `capability::try_allocate`, `Reservation::publish`, `publish_batch` | Direct only for successful fresh per-AS serial allocation and authoritative kind insertion. All six kinds use bounded admission; the unbounded allocator/budget bypass is removed. Device grants own lifecycle through publication; observer grants verify an exact handle. Quotas, staging/escrow lifetimes, captured generation and retained charges are outside this model. |
| `Remove` | typed-registry removal followed by `capability::remove` | Direct for owner-and-kind checked removal. Concrete callers assert that the unified entry exists, including optimized builds, so payload and authority tables cannot silently diverge. |
| `DelegateCopy` | `ipc::connection_delegate`, `PreparedConnection`, `reserve_cap`, `Reservation::publish`/`publish_batch` | Abstract: every direct/call-side/returned IPC connection uses bounded captured-generation admission before payload insertion. Memory-bearing transactions compose publication. Payload tables, quotas, failure and generation lifetimes are omitted. |
| `BeginMove` / `CommitMove` | `PreparedTransfer`, `memory::object::commit_transfers`, `capability::publish_batch` | Abstract split around source escrow and batch publication. Concrete payload ownership remains at the source until atomic authority commit; backing-retention pins and quotas are omitted. |
| `RollbackMove` | `PreparedTransfer` Drop and `SourceEscrow::rollback` | Restores the original source slot without fresh admission, including a retiring but still-present namespace for cleanup. Removed/replaced namespaces cannot be revived. The multi-state `try_spawn_upgrade` helper owns prepared moves until endpoint delegation succeeds; it is not called by the actual single-state upgrade syscall. The model omits these captured-namespace lifetimes and late teardown/reuse; kernel fixtures, not this model, test them. |
| `CloseAddressSpace` | `capability::close_address_space` | Direct for dropping the complete authority namespace after payload teardown. |

## Authorization policy and capability issuance

The target-independent state machine is implemented and host-tested in
`charlotte-authorization` and the co-located production path is wired into
`ns`. The signed loader derives a stable principal from authenticated artifact
metadata, installs its exact address-space generation and roles in a
kernel-only table, and IPC enqueue snapshots those values for the authenticated
twelve-register receive ABI (71–73); retired nine-register receives reject.
The name service synchronizes that exact identity into its policy
store, evaluates default-deny rules, delegates only requested rights, and
records bounded audit entries. The design contract is in
[`../architecture/authorization-policy.md`](../architecture/authorization-policy.md).

| TLA+ action | Intended CharlotteOS implementation | Correspondence |
|---|---|---|
| `PublishService` / `ReplaceService` / `UnpublishService` | `PolicyStore::publish_service` / `unpublish_service`, `ns::OP_REGISTER_AUTHORIZED`, and generation-fenced unregister | Direct for kernel-authenticated service-manager role, rights ceiling, checked generation, and stale-unpublish rejection. `OP_UNREGISTER` requires an authenticated service-manager sender. `OP_UNREGISTER_GENERATION` requires the authenticated sender's stable principal to match the principal recorded at registration, or a service-manager sender. Public bootstrap registration shares policy-store generation bookkeeping, but NS records its separate publication class: public lookup cannot resolve an authorized entry, authorized lookup of either class still requires its policy decision, and public replacement requires the same stable registrar and cannot displace an authorized publication. That class distinction and fallible endpoint-death watch admission are outside this model. Distributed policy replication is not implemented. |
| `SetPolicy` | `PolicyStore::set_policy`, `ns::OP_SET_POLICY` | Direct for authenticated administrator role, exact subject/service rule, optimistic version fence, explicit deny, and exhaustion failure. State and its bounded audit are currently volatile. |
| `IssueTicket` | `PolicyStore::issue_ticket` | Direct in the engine for exact generation-aware identity, requested rights, current rule and binding, attenuation, bounded outstanding decisions, and default deny. The co-located runtime path uses `authorize_now` instead of exporting a ticket. |
| `CancelTicket` | `PolicyStore::cancel_ticket` | Direct for subject-bound removal. Expiry is not implemented. A co-located adapter can keep decisions internal by using `authorize_now`. |
| `Redeem` | `PolicyStore::redeem_ticket`, or co-located `authorize_now` followed by `ipc_reply_connection` | The split-engine path is direct for single use and revalidation. `OP_LOOKUP_AUTHORIZED` performs decision and attenuated delegation serially inside `ns`, records issuance or denial, and retains the caller identity for deferred lookup. |
| `CloseCapability` | ordinary `ipc_close` or endpoint teardown | Direct for capability lifetime. Policy changes intentionally do not retract an already issued direct connection. |

The five negative actions define concrete regression obligations for the
implementation: ordinary callers cannot mutate rules; a decision cannot be
redeemed for another principal; policy and service replacement fence stale
decisions; and minting cannot amplify rights.

## Demand-grown stack budget

`CharlotteStackGrowth.tla` models the resource accounting the guard-fault paths
implement. The mapping below is implemented on both architectures.

| TLA+ action | CharlotteOS implementation | Correspondence |
|---|---|---|
| `Fault` | A lower-EL fault whose faulting address lies in the thread's stack region below its committed pages | Direct: the AArch64 `sync_dispatcher` classifies translation faults (`cpu/isa/aarch64/interrupts/mod.rs`); the x86-64 `#PF` handler classifies not-present user-mode data faults (`cpu/isa/x86_64/interrupts/fixed/exceptions/mod.rs`). |
| `Grow` | Map the faulting page(s) under the address-space table lock and retry the instruction | Direct for accounting: `ThreadContext::grow_user_stack` first checks that the complete multi-page growth fits both the per-thread budget and `free_frames - ReserveFrames`, then commits each page so a partial allocation failure leaves an exact count. The model collapses that loop into one transition. |
| `Kill` | Budget exhausted or pressure reserve breached: fail closed through the ordinary domain teardown path | Direct: the fault falls through to `abort_address_space`, retiring threads and releasing committed frames exactly once in `ThreadContext::drop`. |
| `Exit` | Clean thread/domain exit releasing the committed stack | Direct: `deallocate_user_stack` returns every committed page. |

The two negative configurations (`UnsafeGrowOverBudget` and
`UnsafeKillWithoutRelease`) fix the regression obligations: growth must never
pass the signed or adaptive budget, and a killed domain must return every
committed frame before its slot is reused. `FreeReservePreserved` additionally
checks that successful growth cannot consume the pressure reserve.

## Local NS publication and death watches

`CharlotteNamePublication` supplements authorization and endpoint-observer models.
Names and authenticated principals are distinct atoms. One registry mutation runs
at a time; kernel endpoint closure and CQ hints can occur during it. All real CQ
hints may alias a current watch in the projection, so a stale matching hint still
requires the current producer's own result. This is a stress abstraction, not a
claim that concrete completion IDs are reused or that arbitrary CQ results prove
death. `closed` is an authentic per-publication producer outcome input.

| Action | Implementation | Correspondence and boundary |
|---|---|---|
| `Begin`, `RejectBeforeWatch`, `PrepareWatch` | `ns::register`, `Publication::prepare`, `publication_admitted` | Candidate source is owned before watch admission; rejection preserves current policy/registry. Public replacement checks its existing class and nonzero registrar. Actual fallible allocation and kernel watch registration internals are abstracted. |
| `PublishPolicy`, `RejectPolicy`, `RejectExhausted` | `PolicyStore::publish_service` | Role and generation checks precede policy mutation. The signed-result ceiling is `MAX_SERVICE_GENERATION`; rejected active or tombstone state stays intact. Tree limits/allocations and name decoding are omitted. |
| `Install`, `FinishInstall` | `registry.insert`, replacement Drop | Registry insertion is the replacement linearization point. No request runs between policy publication and insertion. Post-insertion deferred replies precede old-owner disposal. Abort during the private intermediate phase retains owners and permits no further issuance. |
| `Lookup`, `QueueLookup`, `ResolveDeferred` | `try_lookup`, `lookup_or_defer`, `register` waitlist firing, authorization helpers | Immediate and deferred public paths reject policy-published entries. Authorized decisions are input outcomes from the separate authorization model; a public entry may also be authorized by policy. Results snapshot their generation at issuance, without promising later revocation of an issued connection. One lookup/waiter is modeled. |
| `CqHint`, `ObservePending`, `ObserveUnrelated`, `RetireDeath` | `Completion::matches_cq` / `poll`, `Publication::observe`, `retire_closed` | Hints give no death authority. Current publication's own confirmed endpoint-close result authorizes logical unpublication of its full identity/generation. Actual completion result authenticity and kernel wake registration are assumptions. |
| `RetireExplicit`, `ExhaustionRetire`, `RejectRetirement` | `unregister_generation`, `retirement_allowed`, policy unpublish | Exact positive generation plus nonzero captured actor and registrar/manager authorization. No lookup-derived retirement authority. Decoding, identity provisioning and physical teardown are outside this projection. |
| `CancelWatch`, `CloseSource` | `Publication` watch-before-connection field order | Disposes the watch before source close for unused candidates, replacement and retirement. No physical-release proof or destructor-context qualification. |
| `WatchError`, `WaitRejected`, `LifecycleAbort`, `PublicationAbort` | fail-closed `domain_abort` paths | Retains existing connection/watch/registry owners. No replay, fabricated death, cooperative cleanup acknowledgement or physical release. Private publication abort also covers terminal interruption between policy and registry; actual allocator failure behavior is not proved. |

The general instance has two names/principals/candidates and live limit one.
A separate three-candidate exhaustion projection omits lookups/hints/endpoint
closure/abort and reaches the generation-two boundary. Thirteen required negative
configurations weaken the guarded actions. The 64+1 trusted launch budget,
aggregate metadata accounting, raw receive/transport ownership, full-name wire
parsing, outer contexts and physical cleanup remain separate C18 work.

## Boot trust and conditional protected installation

`CharlotteBootTrust` has two explicitly different configurations. `fixture`
projects existing development boot and the x86 test fixture, with protected
production support absent. `contract` assumes that missing boundary to explore
installation ordering; it is a specification obligation, not implemented storage
or production admission. Record identities stand for complete signed-fields
digests; sequences/predecessors are finite, and authentication is an input bit.

| Action | Source or conditional boundary | Correspondence |
|---|---|---|
| `ReadInstalled`, `ReadEnrollment`, `ReadFailure` | explicit `PolicyExpectation` constructors, CLI mode parsing, compiled fixture expectation | Current callers supply state independently and never infer enrollment after failure. Protected reads and authorized initial enrollment in the contract instance are assumptions. Missing/corrupt reads collapse to failure. |
| `Verify`, `RejectRecord` | `signed_policy::verify`, `PolicyExpectation::accept` | Relative to a supplied anchor/cluster: exact digest at current sequence, direct successor with correct predecessor, explicit enrollment floor. Cryptography, key/role qualification and byte parsing collapse to authenticity; the model fixes the floor to one. |
| `CommitPolicy`, `CompetingCommit` | **unimplemented protected installer** | Conditional atomic compare-and-swap of exact revision/digest against captured expectation. Two signed successors may verify against the same old expectation; only one can commit it. Installation is fenced once artifact authority is selected, before bootstrap loading; this serialization remains conditional. No host-file or kernel-slot write implements this boundary. |
| `CommitFailure`, `CommitInterrupted` | **unimplemented protected commit outcome contract** | Definite rejection changes no state. Interruption may leave the whole old or new record committed; cached receipt publication is blocked until a fresh protected read. Atomicity and a recovery fence for unfinished writers are assumed, not proved. An ordinary read of old bytes is not that fence, and no arbitrary hardware error authorizes retry. |
| `AcquireRecipient`, `PrepareSigned`, `RejectRecipient` | `signed_policy::verify_for_boot`, `boot_fixture::prepare`, `PreparedBootTrust::verified` | Shared exact-installed verification produces the distinct `VerifiedBootPolicy` required by the kernel constructor. Enrollment and uninstalled successors cannot supply it. The fixture now uses this shared check; cluster/recipient checks and zeroizing ownership remain. The model compresses cluster matching into authenticated context and treats correct recipient binding symbolically. Protected custody is absent. |
| `PrepareDevelopment` | explicit `PreparedBootTrust::development` in boot composition | Allowed only for an explicitly configured development instance. It proves no production authenticity or custody. |
| `SelectAuthority`, `LoadBootstrap`, `LoadPlatform` | `initialize_boot_policy`, `select_into`, platform loader/store/image verification | The existing slot retains the preparation and immutable public authority before the first NS image. Missing selection rejects; the same selected key gates later platform loading. Signature bytes, heap/stack budgets and actual execution require native evidence. |
| `Publish`, `RejectPublication` | `publish_into`, `launch_steady_state_with_trust` | Composition takes the existing preparation once, leaving selected authority intact; publication requires that same public policy/digest, publishes once and returns unused preparation on rejection. Shared boot verification checks supplied installed state; atomic publication against concurrently changing protected state is only a contract assumption. No runtime policy reset/overwrite API exists. |
| `BorrowView` | `BootTrust::public`, manifest construction, `with_installed_trust` | Public policy is copied from the same boot slot as the kernel gate. Private key bytes, zeroization and callback/IRQ/guard behavior are not proved. |
| `DropPreparation`, `Reboot` | unpublished preparation Drop; abstract machine restart | Drop publishes nothing. Reset removes volatile proof/slot/consumer state while preserving committed acceptance identity. Actual erasure, custody reacquisition and boot authentication are outside the model; reboot is not a kernel reset API. |

Required negative configurations check each weakened boundary. The unavailable
production-boundary control corresponds to the build gate's continued refusal,
not an implemented protected selector. SEC-04 remains partial. This model adds no
root lease, cleanup custody or physical-release qualification.

The concrete boot receipt captures supplied state once, before recipient
acquisition. It does not re-read protected state during preparation/publication.
The conditional instance's live-state guards require the adapter's enclosing
installation/publication serialization; the receipt alone does not implement
them. The fixture's compiled state is immutable within its boot.

### Release admission follow-up (2026-10-21)

The [release-generation audit](../reports/audits/2026-10-21-security-release-generation.md)
found two concrete `reserve_node_capacity` mutations inside `debug_assert!`.
Optimized Rust omitted those charges, violating the intended cumulative promise
boundary. They now execute as ordinary rejecting control flow for singleton and
replica placement. `CharlottePlacementControl` already updates `reserved` in
`Admit` and checks the reserve invariant; its overcommit negative control
retains the forbidden behavior. No model transition is weakened to match the bug.
Both Rust profiles and the selected safe/negative TLC controls qualify the fix.

The complete `CRLADM01` admission receipt and the host's per-component observation
loop remain outside the existing placement/ingress abstractions. Codec, catalog,
scripted CLI and guest execution evidence qualify those paths separately. The
model does not prove receipt byte framing, transport correlation, allocation
failure cleanup, host deadlines or production trust. The formal input inventory
is unchanged.

## Thread context/node admission

| Model action | Rust boundary | Correspondence |
| --- | --- | --- |
| `Admit` | `memory::budget::reserve_thread_metadata`; principal-history `ThreadMetadataCharge`; node `Charge::reserve` | Abstract exact-generation capture and both requested-byte/count admissions before allocation. Node reserve classification is tested natively, outside the projection. |
| `AllocateNode` / `AllocateContext` / `Publish` | `PreparedEntry::try_new_charged`; `Thread::try_new_with_storage`; scheduler submission | Direct ordering with fallible allocation and stack preparation collapsed. The context stays in a stable ordinary Box. |
| `DisposeContext` / `FreeNode` / `Refund` | `Thread::dispose_retired`; collection `release_node`; original charge Drop | Abstract confirmed stack-pair proof, then actual payload/context and node destruction before counter refund. Unused reservation/node cancellation is also represented. |
| `Abandon` | Thread's retained payload; published node/list fallback | Direct retention without counter/allocator entry. Kernel panic does not unwind. |
| `Retire` / `Reuse` | Exact-root close and generation reuse with retained metadata | Abstract root state; captured principal/classification survives reuse. |

The four expected controls violate shared admission, deallocation ordering,
abandonment retention or original sponsorship. This is no executable refinement
or hardware/IRQ/progress proof. Actual bytes, ordinary-node exhaustion and kernel
reserve checks are in the [audit evidence](../reports/audits/2026-10-23-security-thread-metadata.md).

## Memory-map admission qualification

The [23 October mapping admission follow-up](../reports/audits/2026-10-23-security-map-admission.md)
separates pin-free preparation rejection from an admitted prefix. Only the former
can skip maintenance. Existing checked-epoch and retirement projections do not
model this object-mapping pipeline or its scratch allocator. No model invariants
or configurations change in this batch. Kernel controls exercise real preparation,
pins and prefixes with per-call failure adapters. The host shootdown tests check
the diagnostic first-incomplete sample against the owned epoch and stale ACKs;
this supplies no new physical completion or recovery proof. The historical x86
mapping panic remains outside the models' causal claims.

## Principal-history allocation

| Model action | Rust boundary | Correspondence |
| --- | --- | --- |
| `Prepare` | `PreparingHistory::try_new_with`, history `admission::reserve`, `Storage::try_new_charged` | Abstract fixed requested layout and node count. Classification comes from the verified-loader context or unpublished root account. |
| `Attach` | Root metadata publication into `DomainAuthoritySlot` | Abstract candidate transfer to the exact root receipt. Generation/slot ownership remains in existing root models. |
| `Decide` | `PreparingHistory::publish_into` | Direct one record per principal, retaining a losing candidate and preserving the published record's original class. |
| `Exit`, `Confirm`, `ReleaseRoot` | Closing/final-root receipt and `RetiredMetadata::release` | Abstract exact invalidation and actual node deallocation as required inputs before refund. Logical exit alone supplies neither. |
| `Cancel` | Explicit `cancel_unpublished` | Abstract confirmed unused node disposal and post-deallocation refund. |
| `Abandon` | `PreparingHistory` retained field, charged map/list fallback | Direct retention without allocator or counter entry. Published records remain boot-lifetime. |

Six expected controls violate ceilings, ownership/refund ordering, captured
classification or permanent history. This does not model allocator internals,
authority authentication, exact hardware/root-generation proofs, lookup/IRQ
cost or pressure fairness. The [audit](../reports/audits/2026-10-23-security-history-admission.md)
records seven focused checks; no expanded full-suite result is inferred.

## Root-record storage admission

The [24 October root metadata batch](../reports/audits/2026-10-24-security-root-metadata-admission.md)
attaches requested-byte/count charges to the three existing root policy/usage/
authority nodes. `CharlotteRootRetirement` already projects retained metadata
and confirmation before release; it does not represent the individual three-node
quota or partial allocation rollback. Those boundaries are checked by native
one/two-slot, allocator rejection and real-node exhaustion controls plus the
shared charged-collection host tests. No model/configuration changes accompany
this batch. The complete 190-case suite passes (47 safe / 143 expected counterexamples)
separately from these implementation controls; it supplies no executable refinement, allocator-byte,
IRQ-context or pressure-fairness proof.


## IPC namespace construction and root handoff

The [24 October namespace batch](../reports/audits/2026-10-24-security-ipc-namespace-publication.md)
adds a first exact-root operation lease to the existing root-registration owner.
It protects prepared namespace publication after lifecycle/table unlock; it is
released only after IPC unlock. `CharlotteRootRetirement` projects that lease as
an active operation: cleanup/detachment require all such leases to finish and
abandoned operations retain the root. `CharlotteNamespaceDrain` checks the
existing exact-token drain boundary before namespace removal.

Nine focused cases pass: two safe configurations and seven expected
counterexamples. The prior complete 190-case result remains the full-suite
checkpoint; no model/configuration changes accompany this implementation batch.
`CharlotteIpcPublication` starts with live namespaces and does not model their
three allocations or the registration handoff. Native phase-rejection,
heap-held relinking/detachment, missing-namespace rejection and early-close
controls qualify those boundaries. These checks provide no executable
refinement, allocator/IRQ proof or sustained progress guarantee.


## Pending-call node lifetime

The [pending-call storage batch](../reports/audits/2026-10-24-security-ipc-pending-storage.md)
keeps exact caller-root retention and call/reply family counts in the existing
preparation owner. Pending-node allocation leaves IPC; ordinary unused resources
finish post-guard and the root finishes last. Publication only relinks storage;
close/cancellation retain detached nodes until explicit disposal before refund.
Abandonment retains the whole preparation.

`CharlotteIpcPublication` checks joint shared-authority publication, hidden
payload delivery, rollback and charge retention. Its prepared storage represents
shared authority staging, not these family-record node allocations or the
call/reply count dimensions. `CharlotteRootRetirement` projects the caller lease
that prevents teardown/slot reuse while preparation is live. The allocation,
post-IPC disposal and counter lifetime are qualified by native controls and the
shared collection host test of late-bound admission through actual node free.
No model/configuration changes accompany this batch. Focused formal results are
recorded in the audit; the preceding full 190-case result remains a separate
checkpoint. No executable refinement, allocator/IRQ or fairness proof is implied.


## Reply-token node lifetime

The [reply storage batch](../reports/audits/2026-10-24-security-ipc-reply-storage.md)
prepares the second charged node inside the same call owner. Failure cancels the
pending prefix after local guards; successful publication relinks both nodes.
Reply/cancellation and serialized close carry detached token nodes through
post-IPC disposal before refund. Scalar completion fallback also retains its
detached notifications. Original token classification supplies later claim-source
admission and survives caller promotion and root reuse.

Twenty focused cases pass: three safe configurations and 17 expected
counterexamples from `CharlotteIpcPublication`, `CharlotteIpcTransaction` and
`CharlotteRootRetirement`. Shared authority/delivery, rollback, non-terminal
timeout and exact-root gates are their existing projections. The two family
record allocations, token count's node lifetime, guard exit and source-class
copy are not explicit model variables; native controls and shared collection
host tests qualify those implementation boundaries. No model/configuration
changed. The prior full 190-case checkpoint remains separate; these checks give
no executable refinement, allocator/IRQ or sustained-pressure fairness proof.

## Endpoint storage and queue growth

The [endpoint storage batch](../reports/audits/2026-10-24-security-ipc-endpoint-storage.md)
retains exact roots through unlocked endpoint/list/backing preparation and checked
publication. Growth keeps its original sponsor/classification and both backings
through FIFO replacement; post-IPC disposal precedes refund. Existing cleanup
owners carry final closed endpoint nodes. Complete fallback retains all fields.

Twenty focused cases pass: three safe configurations and 17 expected
counterexamples from `CharlotteIpcPublication`, `CharlotteIpcTransaction` and
`CharlotteRootRetirement`. Their shared authority/delivery, rollback,
non-terminal timeout and root gates remain unchanged. Endpoint node allocation,
atomic record/slot admission, old-backing lifetime, post-guard disposal and
source-class preservation are native/host qualification; these models do not
add variables for them. No model/configuration changed. All 235 inputs of the
prior full 190-case checkpoint remain unchanged. These checks provide no
executable refinement, allocator/IRQ, linear-scan hold-time or fairness proof.

## IPC capability payload storage

The [capability storage batch](../reports/audits/2026-10-24-security-ipc-capability-storage.md)
prepares payload nodes before authority mutation and publishes by relinking.
Existing owners keep unused preparation and detached payload/shared-authority
pairs through post-IPC disposal; connection counts follow node lifetime.
Speculative receive rollback and scalar/claimed reply share these owners.

`CharlotteIpcPublication` projects joint authority/delivery and rollback;
`CharlotteIpcTransaction` checks rollback and non-terminal timeout;
`CharlotteRootRetirement` projects existing exact-root gates. Private IPC payload
node allocation, its shared-count prefix, paired metadata disposal, field-drop
contexts and linear traversal remain native/host qualification. No
model/configuration changes accompany this batch. All 20 focused cases pass:
three safe configurations and 17 expected counterexamples. The prior full
190-case checkpoint remains separate, with all 235 inputs unchanged.
These models provide no executable refinement, allocator/IRQ or sustained
pressure fairness proof.

## Unlocked call and endpoint preparation

The [preparation follow-up](../reports/audits/2026-10-24-security-ipc-unlocked-preparation.md)
retains shared-authority metadata, admitted identity and payload allocation in
existing exact-root parents across IPC unlock. Calls also prepare their pending
waiter list outside IPC. Captured-namespace checks precede shared admission and
publication; ordinary disposal remains post-guard, root last.

All 20 focused IPC publication/transaction and root-retirement cases pass:
three safe configurations and 17 expected counterexamples. Existing shared
authority/delivery, rollback, non-terminal timeout and active-root projections
remain unchanged. The three implementation phases, actual allocator/guard
boundaries, per-family metadata and original-class lifetimes are native/host
qualification. No model/configuration changed; all 235 inputs of the prior full
190-case checkpoint remain unchanged. These are not executable refinement,
allocator/IRQ or pressure/fairness proofs.

## Borrowed authority batches

The [publication follow-up](../reports/audits/2026-10-24-security-ipc-borrowed-publication.md)
removes the temporary authority vector. Stable borrowed providers enumerate the
existing reservations/escrows twice under one capability hold, validating the
whole set before mutation. Existing containing roots, pins, copied backing and
charges remain unchanged.

`CharlotteIpcPublication` retains its atomic authority/delivery projection;
`CharlotteIpcTransaction` retains rollback/non-terminal timeout;
`CharlotteRootRetirement` retains exact-root gates. Iterator membership stability,
actual auxiliary storage, allocation/guard boundaries and 257-member execution
remain native qualification. Mixed memory payload maps still allocate after the
authority gate. No model/configuration changes accompany this batch. All 20 focused cases
pass: three safe configurations and 17 expected counterexamples. The full
190-case checkpoint stays separate, with all 235 inputs unchanged. There is no executable refinement, allocator/IRQ or fairness proof.

## Memory-object record lifetime

The [object storage batch](../reports/audits/2026-10-24-security-memory-object-storage.md)
prepares object nodes with private backing before authority publication. Existing
backing receipts retain detached complete nodes and original charges through
physical release and actual node disposal; failure remains terminal without
refund or replay. Global object identities are claimed before publication and
never restored on cancellation.

Focused IPC publication/transaction, root-retirement and principal-backing checks
retain their existing authority/rollback/root/charge projections. Object-node
allocation/deallocation, the concrete pre-attempt fence, field destructors,
nonreused object-ID gaps and actual registry/allocator contexts remain native/
host qualification. No model/configuration changed. The prior full 190-case
checkpoint stays separate; these checks imply no executable refinement,
allocator/IRQ or pressure/fairness proof.

## Read-loan borrower-node lifetime

The [borrower storage batch](../reports/audits/2026-10-24-security-memory-borrower-storage.md) preserves the existing IPC loan,
rollback and root-retirement projections. Destination admission and fallible
borrower-node preparation precede source escrow; commit relinks the same node.
Revocation's prior map retains every node on uncertain completion/abandonment.
Confirmed cleanup carries the selected node and detached shared authority past
memory unlock; borrower free precedes authority admission release. Existing
physical proof and final-object ownership remain unchanged.

Focused `CharlotteIpcTransaction` and `CharlotteRootRetirement` safe/negative
checks are rerun. Models/configurations are unchanged. Concrete node allocation,
deallocation/refund order, containing destructors and registry/heap/IRQ contexts
are native/host qualification, without executable refinement or pressure proof.
The earlier full-suite checkpoint remains separate.

## Memory capability and namespace storage

The [capability storage batch](../reports/audits/2026-10-24-security-memory-capability-storage.md) prepares both remaining payload node
types before authority mutation. Commit preserves existing atomic publication,
hidden-delivery and source-escrow projections while relinking all memory nodes.
Existing source/borrower/namespace/object cleanup owners now retain detached
payload plus exact authority through post-guard payload-before-authority disposal;
failed physical release retains original backing and authority admission.

Focused publication/transaction/root-retirement safe and negative configurations
are rerun without model/configuration changes. Concrete namespace candidates,
field destruction order, node deallocation/refund and heap-held complete commit
are native/host qualification, without executable refinement or allocator/IRQ/
pressure fairness proof. The previous full-suite checkpoint remains separate.

## Memory mapping-node and snapshot ownership

The [mapping storage batch](../reports/audits/2026-10-25-security-memory-mapping-storage.md) prepares node/snapshot storage outside
memory serialization and revalidates the captured object before publication.
Existing pins and retirement receipts retain complete nodes through uncertainty;
explicit node free follows proof, and captured peer leases finish after node free.
The table-walk locking, source identities, physical proof and replay policy remain.

Focused IPC-transaction/root-retirement safe/negative configurations are rerun
without model changes. As in the earlier mapping-admission qualification, these
models do not implement the object mapping/scratch pipeline. Concrete factory
placement, revalidation, prefix descriptors, node disposal, implicit fields and
allocator/IRQ contexts are native/host qualification. No executable refinement,
new custody, sustained-pressure fairness or historical failure attribution is
implied. Earlier full-suite checkpoints remain separate.

## Scratch namespace/extent storage in the root receipt

The [scratch storage admission batch](../reports/audits/2026-10-25-security-scratch-admission.md)
adds concrete requested-layout/count ceilings before node allocation. Original
charges follow those same metadata owners through rejection, detached retention
and actual deallocation. Root-retirement charge projections remain relevant, but
neither focused model represents these counter dimensions, captured reserve
classification, allocator byte layouts or the counter-lock context. Native real
pressure, guarded fallback and exact refund controls qualify that boundary;
no model input, physical proof or refinement claim changes.

The [scratch storage batch](../reports/audits/2026-10-25-security-scratch-storage.md) prepares namespace/extent nodes outside
local scratch/device guards and revalidates captured identity/current space before
publication. Whole namespace nodes now remain in existing root metadata through
final invalidation and explicit post-guard disposal. Bounded root retry remains
about that same complete root; no uncertain mapping/device cleanup becomes
retryable. MMIO mapped-state admission rechecks atomically with its existing claim.

Focused root-retirement/IPC-transaction safe/negative configurations are rerun
without changes. Root metadata retention and physical-once/charge projections
remain; concrete virtual extents, first-fit implementation, MMIO admission probe
race, node construction/disposal and allocator/IRQ/destructor contexts are native/
host qualification. These models do not implement the scratch pipeline. There is
no executable refinement, general metadata budget, sustained-pressure fairness
or historical failure attribution claim. Earlier full-suite checkpoints stay
separate.

## Copy-pin snapshot storage

The [copy snapshot storage batch](../reports/audits/2026-10-25-security-copy-snapshot-storage.md)
prepares vector capacity outside memory serialization and revalidates source
identity/state before snapshot fill and pin publication. Bare pin fallback retains
the allocation with its existing backing fence; explicit snapshot disposal now
precedes unpin. Existing IPC loan/rollback/timeout and root original-charge
projections are unchanged. Neither focused model implements the copy snapshot
factory/revalidation interval, concrete allocator or destructor contexts. These
remain native/source qualification, without a refinement or pressure/fairness proof.

## DMA-pin snapshot storage

The [DMA snapshot batch](../reports/audits/2026-10-25-security-dma-snapshot-storage.md)
shares fallible vector preparation with copy snapshots before source pin mutation.
Captured source/access validation precedes fill and both DMA counter updates; bare
pin fallback retains metadata, while explicit snapshot free precedes unpin. Existing
DMA-operation backend claim/confirmation/abandonment, IPC loan and root-charge
projections remain unchanged. None models concrete snapshot allocation, the
pin-free source-interleaving interval, vector disposal or allocator/IRQ contexts.
Those are native/source qualification, without executable refinement or sustained
concurrent pressure/fairness proof. No model input or replay policy changes.

## Mapping preparation ownership

The [mapping preparation batch](../reports/audits/2026-10-25-security-mapping-preparation-ownership.md)
retains partial node/snapshot fields inertly and explicitly finishes ordinary
constructed returns after local guards. Full namespace generation revalidates
before publication. Root original-charge/physical-once, IPC loan/rollback/timeout
and DMA ownership projections remain; none implements these preparation fields,
CPU-map source-interleaving interval, allocator entry or destructor/IRQ contexts.
Native/source controls qualify those boundaries, without executable refinement,
arbitrary-schedule or sustained-pressure fairness proof. No model input, custody
or physical/replay policy changes.

## Shared snapshot-buffer admission

The [snapshot admission batch](../reports/audits/2026-10-25-security-snapshot-buffer-admission.md)
adds requested buffer-byte/count enforcement and moves the charge with the same
allocation through mapping/copy/DMA publication and retirement. Explicit buffer
free precedes refund; captured classification survives root reuse/promotion.
Existing root original-charge, IPC loan/rollback/timeout and DMA ownership
projections remain. None represents these concrete pool dimensions, allocator
layouts, per-allocation classification or destructor/IRQ contexts. Actual-buffer
pressure and guarded ownership controls qualify those boundaries natively; no
model input, executable refinement, physical/replay or sustained fairness claim
changes.
