# Capability and IPC publication model — 18 October 2026

## Scope

This follows the [root and wait synchronization](2026-10-18-security-model-sync.md)
against implementation `1c6963a2`. It adds
[`CharlotteIpcPublication`](../../tla/CharlotteIpcPublication.tla), one safe
configuration and ten separately weakened specifications to the existing
formal harness. No production Rust changes are part of this batch.

The earlier capability model checks serial authority and atomic move rollback;
the IPC transaction model checks attachment rollback and loan retention through
timeout. Neither distinguishes unified live authority from an owning IPC
payload that is still pending delivery, or captures admitted/detached records
through namespace replacement. Those are the new model's boundaries.

## Source comparison

Reviewed the following implementation anchors:

- [`capability.rs`](../../../crates/catten/src/capability.rs):
  `PreparedReservation`, `Reservation`, `SourceEscrow`, `publish_batch`, captured
  cancellation and namespace detachment.
- [`capability/record.rs`](../../../crates/catten/src/capability/record.rs):
  prepared and retired authority metadata with its original charge.
- [`memory/object.rs`](../../../crates/catten/src/memory/object.rs):
  `PreparedTransfer`, move/copy preparation, mixed-batch publication,
  pending-delivery lookup and `deliver_memory`.
- [`ipc/mod.rs`](../../../crates/catten/src/ipc/mod.rs):
  `PreparedCall::commit_retained`, `PreparedConnection::install`,
  `IpcRegistry::cap`, `PreparedReceive`, first reply observation and cancellation.
- [`ipc/reply.rs`](../../../crates/catten/src/ipc/reply.rs) and
  [`ipc/attachments.rs`](../../../crates/catten/src/ipc/attachments.rs):
  prepared returned outputs and private cleanup of undelivered resources.
- The existing [memory delivery fixtures](../../../crates/catten/src/ipc/delivery_tests.rs)
  and [connection controls](../../../crates/catten/src/ipc/delivery_tests/connection.rs).
  These were read for correspondence; they were not rerun in this formal batch.

The [action map](../../tla/CONFORMANCE.md#captured-admission-and-hidden-delivery)
records the concrete-to-abstract correspondence and local serialization
assumptions. This adds a projection of publication/rollback metadata in C18 and
selected hidden-output boundaries used by C07–C11 in the
[cleanup map](../../reference/cleanup-recovery.md). It does not provide those
families with new physical cleanup or retry authority.

## Modeled contract

One transaction retains owning memory plus a delegated connection. The request
path permits move or copy; returned memory uses move. These compose the existing
kernel owning phases; the model does not introduce a new combined wire API.

1. Prepare storage before reservation. Reserve hidden identities against the
   captured namespace/account; successful staging consumes serials and charges.
2. Keep a moved source in escrow with its original charge and a backing pin.
   Copy preparation owns private backing. The connection's minting source is
   already delivered and usable at entry.
3. Validate the complete batch before publishing any output. Unified authority
   becomes live while both family payloads remain pending delivery. The queue
   or unobserved result owns them.
4. On unstarted failure, cancel staged destinations and restore only an
   existing captured source slot, including retiring authority needed for
   teardown. Restoration needs no fresh admission, even at the source ceiling.
   A removed namespace stays removed. Consumed serials never return.
5. Admit speculative receive authority before dequeue. Admission rejection or
   result-writing failure leaves the queued attachments hidden. Only successful
   receive commit or first reply observation exposes both output grants.
6. Readiness alone exposes nothing. Unobserved cancellation consumes the hidden
   grants without restoring a committed move. Repeated reply observation cannot
   revive an output the caller has already closed.
7. Detached authority records retain their original account charge until
   explicit post-guard node disposal. Namespace replacement and reused numeric
   handles cannot redirect old token cleanup, restoration or refund.

## Finite bounds and counterexamples

The instance uses two numeric namespace roles, two generations per role, four
serial values per namespace, a three-record namespace ceiling and a six-record
node ceiling. Replacement namespaces install unrelated sentinel payloads at
handles 1 and 2. An old token can outlive namespace detachment and replacement;
the sentinels make wrong-generation mutation observable.
These generation labels represent distinct namespace-budget instances. The
concrete checks retain and compare the original budget identity; a numeric
generation alone is not a replacement for that owner.

Ordinary pressure can fill the sender's third slot while source escrow remains
charged. A correct rollback restores the original entry without admission.
One receive result-writing failure can consume a speculative serial and still
leave room for a later successful receive using a fresh serial.

Each negative configuration enables one unsafe action alongside the safe
actions. They are deliberately weakened contracts, not newly reproduced Rust
defects.

| Unsafe action | Required violated invariant |
|---|---|
| `UnsafeUseMemory` | `UseRequiresDelivery` |
| `UnsafeUseConnection` | `UseRequiresDelivery` |
| `UnsafePublishPrefix` | `AtomicPublication` |
| `UnsafeReuseSerial` | `SerialsConsumed` |
| `UnsafeCancelSuccessor` | `ReplacementIntact` |
| `UnsafeRestoreSuccessor` | `ReplacementIntact` |
| `UnsafeRefundRetained` | `RetainedNodesCharged` |
| `UnsafeReadinessDelivers` | `VisibleRequiresDelivery` |
| `UnsafeRepublishObserved` | `ObservedCloseStaysClosed` |
| `UnsafeRollbackNeedsAdmission` | `RollbackRestoresCapturedSource` |

## Validation

Results and checked-input hashes are recorded in the
[evidence manifest](evidence/2026-10-18-ipc-publication-model.json).

`docs/tla/check.sh /path/to/tla2tools.jar` exited 0. All 27 safe configurations
completed with no invariant violation and zero states left on their queues;
all 51 expected counterexamples violated their named invariant after exercising
the required unsafe action. Every required safe action had nonzero total
successor coverage.

The new model generated 36,122,253 states, found 7,634,722 distinct states and
reached depth 42. Across the separate safe configurations, the suite generated
113,894,515 states and found 17,689,055 distinct states. Those totals are sums
over different models, not a composed operating-system state space. The
manifest retains all case results, required action counts, 106 checked-input
hashes, reviewed source hashes and the ten new counterexamples' action
sequences. It is self-contained; temporary checker working data is not cited
as audit evidence.

Bash syntax and whitespace checks passed. Chapter 21's model inventory matches
all 27 safe modules, and local Markdown links were checked. The complete manual
compiled twice into a temporary 227-page PDF; the changed inventory, discussion
and counterexample pages were rendered for visual review. Two long identifiers
in the same chapter were given break points. The existing tracked PDF and
unrelated Chapter 6 edits were preserved.

The checker uses the CI-pinned TLA+ Tools v1.7.4 JAR, reporting TLC 2.19,
revision `5a47802`, SHA-256
`936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88`.
Local Java is Azul OpenJDK 25.0.3; CI uses Java 17. The harness requires
nonzero successor coverage for named actions and rejects negative runs that
fail for a different reason. TLC runs outside the filesystem sandbox because
worker coordination needs a local socket.

An initial draft failed semantic analysis before exploration: the dependent
bound in `SerialsConsumed` referred to `ns` in the same quantified variable
list. Splitting it into nested quantifiers corrected `Unknown operator: ns`.
This was a specification-authoring error, not a kernel finding. The first
explored draft passed; the complete suite additionally checks strengthened
rollback, hidden-container and detached-record invariants against the same
safe transition graph. Earlier draft input files were not separately archived;
only final checked inputs are hashed in the manifest.

## Limits and remaining work

Receive's speculative authority is opaque while IPC is exclusively borrowed;
the model represents it as staged even though the concrete shared entry is
live under that borrow. Namespace detachment assumes confirmed payload
retirement. Adversarial namespace replacement schedules abstract capability
family fixtures; they cannot bypass a root operation lease in production.

The model omits read/write loans, rights attenuation, endpoint reference counts,
call/reply record admission, actual result bytes, allocator/IRQ/lock mechanics,
physical backing-release errors, platform headroom and aggregate byte budgets.
It checks one transaction rather than arbitrary simultaneous transactions.
Abandonment retains preparation; it does not qualify ordinary active-token
Drop or containing field destructors beneath unknown guards. Physical pins are
represented as retained ownership, not hardware quiescence proofs.

This is bounded safety evidence. It does not establish liveness, a complete Rust
refinement proof, pressure progress, a general recovery controller, or production
trust readiness. SEC-07/18 remain partial. No new host, native boot, multi-node
or hardware qualification is claimed.

The next formal gaps remain DMA pre-claim rejection versus started maintenance,
exact unit readiness and uncertain map/unmap owners; NS publication classes and
death watches; boot policy installation ordering; and rejected application
snapshot restoration before durable progress publication. The earlier atomic
models remain useful within their documented projections.
