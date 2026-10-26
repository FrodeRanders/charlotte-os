# Owned entropy request deadlines — 2026-10-26

This follows the [Embassy TLS migration](2026-10-26-security-embassy-tls.md),
starting at `3fd5f845`. The shared entropy client previously used an unbounded
`PendingCall::wait`. A stalled RNG peer could therefore hold the domain's source
checkout and a TLS/SCRAM or TCP/IP startup operation indefinitely. This batch
bounds reply observation and preserves the complete operation through rejection.
It does not close SEC-06/07/14/15/18: the ledger remains **29 corrected, 6 partial,
3 open**, with production trust disabled.

The [version-controlled evidence](evidence/2026-10-26-security-entropy-deadlines.json)
contains source, binary, complete signed-bundle, kernel, boot-image and initial
disk hashes, executed commands, test results and relevant failure diagnostics.
Temporary logs and ignored boot captures are not required to interpret it.

## Contract and configuration

Startup now consumes `EntropyConfig`, containing the owned optional source
connection and `EntropyLimits`. The sole default is a positive **5000 ms**
request budget, aligned with the existing five-second service-observation policy
as a starting value, not a hardware latency guarantee. Startup Rust code can
supply another positive value. Bundled S3, Kafka and TCP/IP select the central
default; changing it requires rebuilding their service images. This adds no
manifest/config-page field, boot-resource parser or live replacement mechanism.
Invalid or repeated installation returns the complete new configuration without
changing the installed owner/policy, including during a checkout.

Hardware entropy preparation, service admission and reply observation carry one
monotonic deadline. The service-only API accepts an existing deadline and does
not bypass the peer using architectural randomness. It rejects empty requests
and lengths above the existing `entropy::MAX_REQUEST = 4096` before submission;
the RNG wire grammar remains unchanged. TCP/IP retains its independent borrowed
source rather than installing global TLS authority.

`ServiceEntropyRead` owns one accepted pending call and borrows the exact source
and destination. Consuming observation adds returned memory/connection and a
read-only mapping in one completion owner. It requires an exact byte count,
memory and no connection attachment. A failed mapping returns its memory owner
to that transaction. Late results are disposed without publication; crossing
the deadline during copying clears the destination before rejection. Callers
may use output only on success; an architectural prefix can already have been
written when service filling fails.

Pre-admission expiry, submission failure, exact observation `IpcError`, accepted
timeout, peer rejection, malformed reply and mapping failure remain distinct.
Wait-admission rejection is not timeout or completion. No accepted request is
replayed. Explicit consuming cancellation returns the entire operation and
borrows if close rejects; ordinary Drop uses the runtime's checked cancellation
and domain-abort policy before ending those borrows. The global source/policy
lease survives the whole operation without holding its metadata mutex.

This is an observation deadline: deferred `start` does not arm autonomous
cancellation. A caller retaining that owner can retain work/backing past its
deadline until observation, cancellation or Drop. Source discovery, whole
TLS/SCRAM exchanges, scheduling progress and blocking kernel mapping/cancellation
latency are outside this bound. RPC cancellation supplies no device-DMA
quiescence proof. Increasing the budget also retains source checkout, pending
metadata, destination borrow and possible returned backing longer; qualify timing,
queue/memory pressure and cleanup together. The
[configuration guide](../../guides/configuration.md),
[TLS reference](../../reference/tls-client.md), C18 in the
[cleanup matrix](../../reference/cleanup-recovery.md) and both relevant manual
chapters describe these consequences.

## Executed qualification

Host debug and release each passed **374 runtime tests plus one doctest**:
53 runtime ownership tests, 244 service tests, 3 reconnect-example tests and
74 transport tests. Two existing runtime documentation examples remain explicitly
ignored; no runtime test is ignored. This includes two new entropy configuration/empty-input
controls, existing source checkout/handshake exclusion controls, OpenSSL 4.0.3
TLS/mTLS acceptance and rejection controls, and the runtime's timed-loan test
for timeout and waiter-admission rejection. Host interop uses the test-only host
entropy source. These totals do not include the previous batch's separate
signing/launch suites, which were not rerun here.

Strict service-library/all-binary Clippy passed on both freestanding targets,
workspace formatting passed, and all three GCM backend controls passed (one
accepted erasing configuration, two intentionally rejected configurations).
The AArch64 forty-service and x86-64 thirty-seven-service bundles were rebuilt
and signed. The separate AArch64 Sitas image was reused: its inputs and hash
remain unchanged from the preceding qualification.

All four final native runs used four LPs and fresh storage. Each complete
manifest bundle map matches the recorded current signed bundle, including the
unchanged Sitas image where applicable; archived kernel hashes also match.
No runtime expectation, admission capacity or runner deadline was increased.

| Scenario | Deferred pass / fail / pending | HTTP checks | New entropy probe executions |
| --- | --- | --- | --- |
| AArch64 S3 + security + HTTP, SMMUv3 | 22 / 0 / 0 | 14 | 2 |
| x86-64 security + HTTP, AMD IOMMU | 18 / 0 / 0 | 14 | 2 |
| AArch64 Kafka, SMMUv3 | 20 / 0 / 0 | — | — |
| x86-64 S3 + security + HTTP, Intel IOMMU | 20 / 0 / 0 | 14 | 2 |

The signed probe runs in both scoped publication generations. Its unregistered
local peer returns public deterministic fixture bytes only; it is never installed
as the domain RNG or used for cryptography. Controls cover moved memory, short
count, missing memory, unexpected connection, negative device status, explicit
cancel, Drop of a completed unobserved result, late-result disposal with the
original deadline, expired/invalid requests before enqueue, full-queue rejection,
an accepted silent-peer timeout and recovery of all eight queue slots. Both
publication generations retain the existing `0xffff` security result.

Real AArch64 S3/Kafka uses the owned VirtIO RNG RPC path because the selected
Cortex-A710 lacks RNDR. Both S3 runs verified the 33-byte object round trip.
Kafka passed after the controlled route-leader stop/restart: offset 1, commits 4,
produced 4, retries 2, DLQ 1, timeouts 1, group generation 3, heartbeats 3,
metadata refreshes 2, coordinator refreshes 0, retry attempts 4, terminal errors
2, output-route produced 5 and lag 1.

The fresh production advisory wrapper passed against RustSec
`a78282a1414d6c28cc228c4eae3570451affb0dc` (1378 advisories): all 182 locked
packages, zero vulnerabilities/warnings, no ignores or target filtering. The
lockfile and Embassy/TLS/crypto source pins are unchanged from the baseline.

## Retained failures, diagnostics and verification limits

Initial strict Clippy rejected the combined public operation's consuming cancel
error as at least 176 bytes (`result_large_err`). Separating the cancellable
pending phase from the private returned-resource completion phase fixed this
without allocation on failure or a lint suppression. Both final target checks
pass; the initial failure remains recorded separately in the evidence.

Healthy HTTP probes needed 13 existing transport retries: AArch64 S3 had one
timeout and four connection resets; AMD and Intel each had four connection
resets. All 42 probes passed within unchanged retry budgets. Kafka emitted
`TLS handshake transport failed (6)` at guest time 26.953134 during the
controlled route-leader fault window, then its complete success oracle at
33.236656. Those observations are retained separately from passing outcomes;
they do not establish causes for earlier scheduler, IPC or root-retirement
intermittents. No host suite or final native run failed in this batch.

The manual built in two passes: 252 pages, no undefined references, and the same
189 global overfull warnings as the preceding build. Contents page 14 and
affected/adjacent pages 116–118 and 197–200 were rendered and visually checked;
this is not a fresh layout audit of the entire manual.

New-client-specific mapping failure and forced kernel waiter-admission rejection
were not injected in the native probe. The mapping owner restoration and exact
observation-error propagation were inspected; the existing runtime mock test
qualifies its lower boundary's retained-loan behavior. This is not equivalent
to exhaustive execution of the new wrapper under those faults. No sponsor
replacement, shared kernel-account saturation or fabricated readiness was used
to claim coverage. Timer completion admission and scheduler watchdog admission
are distinct accounts.

The RNG server still allocates reply memory from its own account. A holder of
RNG authority can retain raw-protocol replies or deferred operation owners,
coupling producer availability to caller retention. The ordinary helper copies
and disposes the reply, but caller-funded reply storage and retention-pressure
qualification remain SEC-07 work. This is not a claim that an ungranted
application can obtain that authority. Startup name-service lookup and the RNG
server's own publication/receive paths remain separate readiness gaps.

Full TLS key-schedule/HKDF/IV and compiler-copy erasure, intermediate-chain
verification, third-party provenance/license work, physical-platform timing,
production enrollment and remaining SEC-18 cleanup contexts are unchanged.
No kernel behavior or formal model changed in this batch.
