# RNG publication and idle readiness — 2026-10-26

This follows `1d1b6b1d` and the
[caller-funded reply correction](2026-10-26-security-rng-caller-buffers.md).
The RNG server still observed registration with an unbounded wait and woke
every 10 ms to inspect its request endpoint. This batch uses the existing
bounded publication policy and real endpoint/default-CQ readiness. It adds no
protocol version, resource pool, hardware completion interrupt or physical
cleanup authority.

The [tracked evidence](evidence/2026-10-26-security-rng-readiness.json) contains
commands, source/executable hashes, complete signed-bundle maps, kernel,
boot-image and initial-disk hashes, failure diagnostics and passing results.
Temporary logs and ignored captures are not evidence dependencies.

## Behavior and ownership

The owned endpoint binds to the launch-owned default CQ before publication.
`publication::register` retains that endpoint and retries only an unaccepted
exact queue-full rejection. An accepted registration is observed once; no
timeout, malformed reply or admission rejection permits replay. Binding and
publication failure record their stage/error and abort with the initialized
device and endpoint owners live, without reporting device quiescence.

The main loop checks lifecycle, drains CQ hints and tries to receive one
request. An empty receive parks with `reactor::wait(None, 0)`. The existing CQ
generation/registration/recheck contract covers an enqueue between inspection
and parking. Endpoint enqueue and exact-root lifecycle request publication
signal that same CQ. A wake causes another source check; it does not receive a
message, adopt attachments or establish teardown completion. Rejected wait
admission follows the shared fail-closed domain-abort policy with owners live.
No IRQ policy, pending-call owner, caller backing or original charge changes.

The removed timer was ordinary idle polling. The separate existing VirtIO
descriptor/status poll remains bounded and has no completion-interrupt
notification. This batch does not call that hardware path event-driven.

## Shared configuration

No new limits are introduced. RNG now shares the compile-time publication
settings in `crates/catten-services/src/publication.rs`:

| Setting | Current milliseconds | Scope |
| --- | --- | --- |
| `RETRY_BUDGET_MS` | 1000 | Admission; exact queue-full-only retries |
| `OBSERVATION_BUDGET_MS` | 5000 | One accepted call's observation |
| `RETRY_PAUSE_MS` | 10 | Admission backoff without a queue-capacity subscription |

Admission and accepted observation have separate existing phase budgets; this
is not one five-second end-to-end startup guarantee. Unrelated wakes do not
restart an accepted observation, and accepted work is never retried. Changing
these central settings affects every helper consumer: rebuild and qualify
boot ordering, loader/readiness deadlines, queue-full/silent publication and
retained owners together. There is no boot-resource parser, per-instance live
setting or idle timer to tune. The [Configuration guide](../../guides/configuration.md),
[entropy reference](../../reference/entropy.md), cleanup family C18 and both
manual chapters record this scope.

## Native negative control and replacement

The new opt-in fixture retains the boot service's captured TID, thread
generation and root identity. Within a five-second observation bound it checks
Blocked state, the `CompletionQueueWait` migration constraint, zero use of that
root's scheduler-timer sponsor, and exactly one registered CQ waiter. A timer
sleep cannot satisfy that combination. The fixture's own 1 ms observation sleep
is bounded test instrumentation, not the server's wait implementation. No
sponsor is replaced, resized, reclassified or saturated.

For an intentional negative control, the preceding signed RNG ELF was staged
with the current bundle and new kernel fixture. The fresh-storage/four-LP
AArch64 security run reached the application's `0xffff` result, then failed
the new check at guest time 50.335548: **18 passed, 1 failed, 0 pending**,
failed bitmap `0x20000000`. Its diagnostic was:

```text
panicked at crates/catten/src/self_test/el0_security.rs:573:18:
self-test deadline expired while waiting for RNG idle must register event-only CQ readiness
SELFTEST FAILED: scoped-launch-security
```

This is an expected failing old-server control, not a passing execution or an
unattributed intermittent. Its archived kernel hash, complete artifact identity
and diagnostics remain separate in the evidence. The candidate signed RNG image
was restored before the positive run; no source, deadline or capacity changed
between these two executions. The negative image remains the real prior driver,
not a synthetic `false` implementation.

The replacement's combined AArch64 S3/security/HTTP run passed **22 / 0 / 0**,
with all **14 HTTP checks**. Its complete manifest bundle equals the recorded
current signed bundle, and the archived kernel hash matches. In each of two
publication generations the real RNG thread:

- parks on the checked CQ interest without a scheduler watchdog;
- receives an unrelated explicit CQ wake, advances its dispatch count and
  returns to that same kind of idle registration;
- receives a wake from the actual retained-root `ServicePages::write_request`
  publication path, again dispatches and reparks;
- processes endpoint requests and returns to idle, including a ready but
  unobserved moved-memory result and the existing 32-page caller-retention,
  independent-progress and exact-sponsor retirement controls.

The lifecycle publication deliberately writes `STATE_RUNNING`. It exercises
the real wake transport without asking the boot hardware driver to stop or
fabricating a device-quiesced acknowledgement. These observations do not
qualify actual stop/reset/DMA teardown. The selected Cortex-A710 has no RNDR;
TCP/IP and S3/TLS use the real VirtIO RNG service, and S3 verified its 33-byte
object round trip. Existing HTTP transport retries are retained in the evidence
alongside the passing checks; they establish no cause for historical failures.

## Other validation and remaining scope

The debug host suite passed **374 runtime tests plus one doctest**: runtime 53,
services 244, reconnect example 3, transport 74. Two existing documentation
examples remain explicitly ignored; no runtime test is ignored. This includes
the shared publication queue-full/permanent-rejection controls and runtime CQ/
ownership boundaries. It is not a new driver-specific fault-injection result.
Both freestanding service libraries/all binaries passed strict Clippy; both
signed bundles built/staged, including unchanged separate AArch64 Sitas.
Workspace formatting and whitespace checks passed.

The manual built in two passes: 252 pages, no undefined references and the
unchanged 189 global overfull warnings. Contents page 14 and affected/adjacent
pages 117–119 and 198–201 were rendered and visually inspected. The remaining
pages were not re-audited. Kernel production code, dependency inputs, crypto
features and wire grammar are unchanged. The preceding report's fresh advisory
gate remains historical evidence; it was not represented as a new scan here.

No new driver-specific forced CQ admission rejection, binding failure or stalled
registration was injected. No current x86 native, release-host, Kafka, full
signing/launch, formal-model or GCM build-guard rerun is claimed. The actual
readiness/repark/accounting assertions ran on AArch64; x86 has build/Clippy
qualification only in this batch.

Hardware completion timeout remains a separate source-confirmed boundary:
`VirtioRng::fill` returns failure without confirming the descriptor outcome,
and a subsequent request can reach the same descriptor position. Fencing that
reuse and qualifying terminal retention/recovery require separate work; idle
readiness supplies no replay or physical-release authority. Actual shutdown,
hardware failure injection, broader flood fairness, full TLS erasure and other
cleanup/principal/heap contexts also remain open. Dispositions stay **29
corrected, 6 partial, 3 open**, with production trust disabled.
