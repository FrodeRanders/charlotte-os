# RNG ring-index access width

Date: 2026-10-28. Follow-up to the [descriptor fence](2026-10-26-security-rng-descriptor-fence.md)
and the concurrent [status-observation work](2026-10-28-security-status-observation.md).
SEC-06/18 remain partial; the ledger remains **29 corrected in scope, seven
partial and two open**. Production remains disabled.

## Captured failure

The first authorized ARM status/security/HTTP campaign passes all 20 kernel
self-tests and both new status-snapshot controls, but fails later management
availability. Its kernel SHA-256 is
`c6ae7dd8b4ac6810fbc129ecac516b18bbab2a4d64468ae898c037e83ece15dd`.

The exact retained native diagnostics are:

```text
[+ 90.680532] asid=118 [rng] device retained after UnexpectedIndex { expected: 256, observed: 511 }, Some(Attempt { previous_index: 255, expected_index: 256, slot: 7, requested: 32, copied_prefix: 0, checked_completion: None }); no descriptor replay or cleanup
[+ 90.682713] asid=130 [panic] panicked at crates/catten-services/src/tls_client/entropy_client.rs:310:42:
system entropy unavailable during TLS crypto: Rejected(-2)
[+ 102.630006] asid=126 [panic] panicked at crates/catten-services/src/tls_client/entropy_client.rs:310:42:
system entropy unavailable during TLS crypto: Rejected(-2)
```

ASID 130 is deployment ingress and ASID 126 is httpd in this capture. The host
healthy deployment reader after the unregistered-client rejection times out in
its TLS handshake; subsequent curl keyhole controls reset. The whole campaign
fails. Earlier successful controls and self-tests do not turn that into a
passing ARM HTTP run. The [tracked evidence](evidence/2026-10-28-security-status-ring.json)
retains its separate result, public artifact hashes and selected diagnostics.

## Corrected access boundary

The adapter's `ring_read_u16` called the runtime's byte-copy volatile helper.
A device update from `0x00ff` to `0x0100` between its low/high byte reads can
produce `0x01ff` (511). `ring_write_u16` likewise published indices one byte at
a time, exposing an intermediate value. The split ring defines both live
indices as little-endian 16-bit fields in the
[VirtIO specification, sections 2.7.6 and 2.7.8](https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html).

`SharedDmaMemory` now supplies checked aligned little-endian u16 volatile
read/write methods. Bounds, address overflow and alignment reject before
access. The RNG adapter uses those methods for its u16 ring accesses; it keeps
the same descriptor-before-publication release fence and completion-before-
payload acquire fence. Byte-copy helpers remain for stable descriptor fields
and entropy payloads; their documentation explicitly excludes coherent scalar
observation/publication.

The ring-access defect is reproduced deterministically by the host model below.
The native capture is consistent with that tear, but it does not record the
individual machine/device accesses and cannot establish their exact interleaving.
No broader historical entropy or network failure is attributed by this change.

The existing descriptor owner remains armed before hardware mutation. Exact
completion checks, rejection of genuinely unexpected indices, retained complete
backend/charges, no replay and inert retained Drop are unchanged. This is an
access-width correction, not a reset/recovery API or permission to ignore a
fenced device. The [entropy contract](../../reference/entropy.md) and C18's
[owner-family row](../../reference/cleanup-recovery.md) retain those limits.

## Controls and qualification

`scripts/tests/test-rng-ring-access.py` compiles the actual production ring
helper bodies with a sequential public-data device model. It changes the index
between separate byte accesses without a Rust data race. The current helpers
pass both read/publication controls. Isolated restoration of the byte read fails
with `torn used index: 0x1ff`; restoring byte publication fails with `torn
available index`. Each mutation fails with exit 101 and is never deployed.
All three Python controls pass and the host runner now includes them.

The runtime suite passes **54 tests**, including live mapped-backing halfword
read/write, endian bytes, the final valid halfword, odd/out-of-bounds/overflow
rejection and untouched backing after rejection. The three existing descriptor
fence mutation controls still pass. These verify host behavior and production
adapter selection, not physical-device atomicity or cache coherence.

The security fixture explicitly grants `rng=call` in both signed descriptors.
Each primary launch makes 512 sequential service-only 32-byte reads under one
30-second deadline, using the existing owning entropy adapter. It bypasses
architectural entropy, cannot replay accepted requests, erases scratch on local
Drop, and logs no entropy bytes. Successful completions cross at least two
low-byte boundaries even from an unknown starting position. Both launches must
pass before their existing result bitmap completes; no new result bit or
production override is introduced. Full u16 hardware rollover and real devices
remain separate qualifications.

The complete service library suite passes **257 tests** after this correction. No full host
repository suite, new advisory scan, real-device timeout/reset, forced teardown
or physical-erasure guarantee is claimed.

## Separate x86 fixture and retirement failures

The first corrected-bundle x86 attempt lacks a VirtIO RNG in its runner
configuration: native driver preparation reports `entropy=false`. The newly
added service-only probe cannot be qualified on that machine, exits before its
rollover receipt, and scoped-launch-security fails (17 passed / 1 failed).
The acquire path did not log a per-return error, so its precise rejection is
not retained. Subsequent TLS host controls time out after the self-test panic.
This is preserved as a failed fixture integration attempt, not RNG rollover
or mTLS evidence.

The x86 runner now attaches the protected modern VirtIO RNG as ordinary
runtime hardware, independently of network/verifier selection. Architectural
entropy remains available; the explicit service probe alone guarantees use of
the real ring. On Intel VT-d the next run prepares `entropy=true`, passes both
512-request controls and both snapshot controls, and advances publication
generations 1 and 2. It subsequently fails retiring the separate root `157:8`:

```text
[+ 93.048801] [domain retirement] root=157:8 error=AddressSpace(OperationDrainTimedOut) pending=Some(Memory(PendingMemory { object: 35194, authority: false, transfer_in_flight: false, retirement_pins: 1, dma_pins: 0, copy_pins: 0, revoking: false }))
security probe reclamation: AddressSpace(OperationDrainTimedOut)
SELFTEST COMPLETE: passed=17 failed=1 pending=0 passed_bitmap=0x23bbff failed_bitmap=0x20000000 pending_bitmap=0x0
```

That is a failed overall campaign even though the added controls pass. The
captured pin does not identify its retaining operation or authorize a refund,
root release or replay. It remains unresolved; it is not attributed to the
ring correction or assumed identical in cause to historical retirement
failures. The fresh repeat and the final gated campaign are recorded below.

## Delayed-time readiness and final execution

The fresh x86 repeat passes all 18 self-tests and both sets of new controls,
without the retirement failure. Its overall exit status is still 1: the HTTP
probe starts while the native time service has no sample, receives TLS EOF,
and exhausts its bounded retries. The first native synchronization occurs only
at guest **82.702303 s**, after those exchanges; no application response or
authenticated identity was observed. The exact EOF diagnostics and that clock
sample are retained in the evidence.

Both runners now use the shared `catten_boot_http_probe_ready` precondition:
listener publication plus a positive native synchronized-time sample under the
existing runner budget. This is a host assertion hint; each native connection
still obtains a fresh real time-service reply. No synthetic UTC, verification
bypass or post-submission replay is introduced. Eight host gate controls pass,
including missing/zero/malformed/wrong-source time, missing listener, ordinary
positive/negative observations, and the isolated old listener-only behavior.

Final results (all use the same corrected service implementation):

| Campaign | Native self-tests | New controls | Whole host/guest campaign |
| --- | --- | --- | --- |
| Ring-corrected ARM / SMMUv3 | 20 passed / 0 failed / 0 pending | Two snapshot receipts; two 512-request hardware-entropy receipts | Pass, exit 0 |
| x86 / Intel VT-d with final HTTPS readiness | 18 passed / 0 failed / 0 pending | Two snapshot receipts; two 512-request hardware-entropy receipts | Pass, exit 0 |

Each passing campaign completes node/cluster collection, EOF/abortive/idle peer
recovery, deployment framing/slow-peer controls, both interfaces' enrolled roles
and negative client/server identity controls, healthy-reader recovery, reader
mutation rejection, and verified CLI checks. The ARM pass predates the added
host readiness helper; final helper behavior is covered on both runners by
the eight executable host controls and the final native x86 campaign. Both
passing campaigns retain their bounded reset/EOF retries in the evidence.

ARM kernel SHA-256: `25bd81f10df2ed66e9635a490a13411c1694dda15ba9175bdb9184a16723ed5f`.
Management resource SHA-256: `4213817ebc9fca3b66b466a33af629b7612c32265f0eb3dea6868820f6d32559`.

x86 kernel SHA-256: `a98dfc30c7b93f89d1bfb7ca3e154756c56461b6792ad479d6ffa41af616b38a`.
Management resource SHA-256: `f7c095a3f9b6cd24e8604d23c15ae7833546f8e4e5f0373378dafab542336be8`.

The tracked JSON retains all seven executed campaigns separately, complete
current signed-bundle digest maps, source hashes, individual host controls,
exact native receipts/failure diagnostics and the isolated regression harnesses.
No temporary log or ignored binary is needed to interpret these claims.
Strict Clippy passes for the host runtime's full target set and both actual
service targets (library/httpd/rng/security-probe); workspace rustfmt, script
syntax and whitespace checks pass. Four CI-input, two boot-image and eight
tooling-contract controls also pass.

The root `157:8` retirement-pin timeout remains unresolved despite two later
self-test passes. This batch does not qualify AMD-Vi execution of the newly
attached x86 RNG, real devices, full hardware u16 rollover, receiver-held
cleanup, forced termination or sustained pressure. SEC-06/18 and all numbered
finding dispositions remain unchanged.
