# Bounded status dependency observation

Date: 2026-10-28. Parent: `9f3d5203`. This advances the remaining SEC-06
status-render/dependency boundary. SEC-06/18 remain partial; the ledger remains
**29 corrected in scope, seven partial and two open**. Production remains disabled.

## Source boundary and correction

The parent status service has four indefinite RPC observation sites: name
lookup, scalar status, moved snapshot replies and caller-buffered TCP/IP
status. `OP_TRY_LOOKUP` avoids waiting for publication of an absent name, but
its accepted IPC call can still wait indefinitely for a reply. Since httpd
serves one connection at a time, a silent dependency can hold that observation
and prevent the handler from returning to its listener. This is a source
finding; no parent guest stall reproduction is claimed.

The production `status_observation::Snapshot` now carries one monotonic
deadline through every dependency of a requested node or cluster snapshot.
`SNAPSHOT_MS` is 5,000 ms, starting before lookup/collection. It does not restart
for later sources. Optional startup lookups share a separate bounded observation.
All four observation paths use `PendingCall::wait_timeout` with the remaining
budget. After expiry the adapter rejects another source before allocating or
submitting work. The renderer keeps its existing unavailable-data paths,
including HTTP 503 for an unavailable committed cluster snapshot.

Preparation/submission rejection, observation rejection and timeout retain
distinct typed errors. A rejected waiter is not reported as timeout. Each
accepted call remains owned until result observation or checked cancellation;
the existing PendingCall Drop policy confirms close or aborts before ending an
uncertain borrow. Returned memory/connection owners are observed exactly once,
and mappings retain their existing scoped lifetime. There is no automatic
request replay, reply polling, new cleanup registry or physical cleanup proof.
C18's [owner-family row](../../reference/cleanup-recovery.md) and the
[living contract](../../reference/status-observation.md) record the scope.

The deadline bounds observation, not receiver-held cancellation, synchronous
report rendering, listener setup or teardown. Header input, TLS/session and
response output retain their existing budgets. This does not turn the serial
status service into a fair multi-client reactor or close SEC-06/18.

## Native controls

The signed security probe uses the same production adapter as httpd against
an eight-slot endpoint that never receives. At the caller's existing full
memory-object budget it checks:

- An accepted scalar call times out under a 100 ms observation budget and a
  two-second outer fixture bound.
- Reusing that expired snapshot rejects scalar and buffered requests before
  admission, even with both IPC and caller memory exhausted.
- A live snapshot distinguishes scalar queue-full, buffer allocation rejection
  and moved-buffer queue-full rejection.
- Rejected moved submission and an accepted, unreceived moved-buffer timeout
  each restore exactly one allocatable page, with all other pages still held.
- All eight silent-endpoint queue slots can be filled again after cancellation.
- A healthy TCP/IP status call returns the caller-funded page; mapping and
  dropping its reply restores exactly one page at the same original ceiling.

The fixture limits do not change the production five-second budget. These
controls do not inject waiter admission failure, mapping rejection or a
receiver-held cancellation claim. The existing native `--http-test` campaign
separately exercises real authenticated status collection and management role/
identity rejection with healthy-reader recovery. It does not stall a real
httpd dependency or establish sustained flood fairness.

## Validation

The preliminary x86 campaign passes all 18 registered self-tests and its full
HTTP/mTLS campaign. The preliminary ARM campaign passes all 20 self-tests and
both new snapshot controls but fails subsequent HTTP/mTLS availability: the RNG
retains an unexpected 255-to-256 index observation, followed by management
entropy rejection and domain aborts. The separate
[ring-access report](2026-10-28-security-rng-ring-access.md) preserves that failure
and its correction. Final combined results and public artifact receipts are
recorded in the [tracked evidence](evidence/2026-10-28-security-status-ring.json).

The service library suite passes **257 tests**. Strict Clippy passes for the
actual AArch64 and freestanding x86 httpd/security-probe targets; signed service
bundles, workspace formatting and whitespace checks pass. No dependency or
wire-format change is made; no fresh advisory scan or full repository host
suite is claimed.

## Setup failures retained separately

- The sandboxed library run passes 254 tests but fails its three existing TLS
  interoperability tests at local socket bind with `PermissionDenied / Operation
  not permitted`. The authorized complete repeat passes all 257.
- One sandboxed AArch64 signing attempt ends with `Killed: 9`; its cause is not
  diagnosed. An early runner/build attempt also fails before boot because its
  `objstore_client` staging input is absent. The completed authorized bundle
  build succeeds. Neither attempt supplies native execution evidence.
- Two sandboxed x86 attempts and one sandboxed ARM attempt exit before guest
  execution because QEMU cannot create its forwarded deployment listener. Moving
  to isolated ports alone does not resolve the sandboxed failures. These are
  not TLS rejection, healthy-reader or security-probe passes.

Broader connection/packet floods, all-position admission pressure, lifecycle
interruption during collection, receiver-held cleanup and historical IPC/root
retirement failures retain their separate acceptance requirements. Protected
provisioning, security time and peer authentication are unchanged.

## Final combined execution

The ring-corrected ARM campaign passes all **20 registered self-tests** and its
complete HTTP/mTLS campaign. The final x86 Intel VT-d campaign passes all
**18 registered self-tests** and the same host campaign after the readiness
correction described in the companion report. Both architectures produce two
status-snapshot receipts and two 512-request real-entropy receipts. The ARM
pass precedes the host-readiness helper; the helper's actual updated gates are
exercised by eight host controls on both runners and the final x86 guest.

Passing host campaigns retain their bounded transport-reset/EOF retries in the
evidence; no error-free transport or sustained fairness claim follows. An
intervening x86 run passes the added controls but fails overall at root `157:8`,
object `35194`, with one retirement pin and `OperationDrainTimedOut`. Another
fresh run passes all 18 self-tests but begins HTTPS before its delayed UTC
sample and fails host acceptance. Both failures are kept separately. The final
passes do not attribute or resolve the retirement-pin failure.
