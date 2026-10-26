# RNG caller-funded reply backing — 2026-10-26

This follows `e5b9f2bd` and the
[entropy deadline work](2026-10-26-security-entropy-deadlines.md). That batch
recorded a producer-retention gap: the RNG service allocated a fresh reply
object for every scalar fill request, so an authority-holding client could
retain producer-funded backing. The current service fills and returns supplied
caller backing. There is no producer-allocating compatibility path.

The [tracked evidence](evidence/2026-10-26-security-rng-caller-buffers.json)
records commands, source/executable hashes, complete signed bundle maps,
kernel/boot-image/initial-disk hashes, original failures and final results.
Temporary logs and ignored captures are not evidence dependencies.

## Current protocol and owners

RNG interface version 2 uses `OP_FILL = 2`. It requires exactly one current
protocol-sized writable buffer and an explicit count in `1..=MAX_REQUEST`.
Success fills only that prefix and returns the same object with the exact
count and no connection attachment. Retired scalar opcode 1 rejects, as do
missing/oversized buffers, invalid counts and connection attachments. No old
wire decoder or fallback allocation remains. All in-tree production callers
are migrated: TLS/SCRAM, TCP/IP and the DNS capacity boot-nonce path.

The shared `entropy_service::serve_fill` operation owns the whole incoming
transaction, reply/loan authority and transient mapping. Mapping rejection
restores the memory owner; unmapping rejection retains its mapping. Local
access and received memory end before an ordinary error reply or loan
cancellation. The hardware callback only borrows the validated prefix. It
retains the driver's existing protected DMA bounce buffer; caller reply memory
is not DMA-mapped. A device or mapping failure cannot become successful entropy.

The client allocates before submission within the original deadline, with a
distinct `EntropyError::Preparation`. `call_move` consumes that buffer;
submission rejection restores it to the existing operation for disposal.
Accepted pending work, output/source borrows, returned resources and mapping
retain the earlier deadline/cancellation contract. The client additionally
rejects a returned object of the wrong size. There is no accepted-request replay.

Kernel accounting is unchanged: moves retain the original allocating
generation's backing charge, including hidden/unobserved replies, return,
retention, sponsor retirement and final physical release. This correction
changes who supplies reply storage; it does not refund or reclassify charges.

## Configuration and DNS follow-through

`catten_services::entropy` groups `BUFFER_PAGES = 1`,
`BUFFER_BYTES = BUFFER_PAGES * 4096` and `MAX_REQUEST = BUFFER_BYTES`.
These compile-time dimensions control client allocations and the driver's
persistent private entropy DMA buffer together. The 4096-byte page unit is an
ABI constraint. Compile-time checks require a nonzero buffer and byte counts
that fit the descriptor's `u32` length. Changing the dimensions requires
rebuilding every caller and driver and qualifying minimum/maximum, rejection,
retention, cancellation, queue/memory pressure and DMA behavior. No boot-resource
parser, live resizing, kernel capacity change or enlarged runtime limit is added.
The installed positive entropy observation budget remains a separate policy,
defaulting to 5000 ms.

Inspection found a separate DNS caller using the retired scalar request and a
5 ms reply poll/sleep loop. It now uses the shared client, with one named
`CAPACITY_NONCE_TIMEOUT_MS = 100` budget through immediate name lookup and fill.
The old polling helper and obsolete imports are removed. DNS still collects
the boot nonce synchronously in its capacity-report context; a failed
collection follows the existing report interval and does not publish a nonce.
This is bounded observation, not a full asynchronous reactor migration or a
guarantee for scheduler/kernel cleanup latency. Its individual collection
budget is not restarted between lookup and entropy admission.

The [entropy reference](../../reference/entropy.md),
[Configuration guide](../../guides/configuration.md), TLS/secret-lifetime
references, API simplification ledger, cleanup family C18 and the manual's
networking/configuration chapters describe the current contract together.

## Executed qualification

Host debug and release each passed **374 runtime tests plus one doctest**:
53 runtime ownership tests, 244 service tests, 3 reconnect-example tests and
74 transport tests. Two existing runtime documentation examples remain
explicitly ignored; no runtime test is ignored. These include current selected
TLS cipher, OpenSSL 4.0.3 TLS/mTLS acceptance/rejection, source/handshake exclusion
and runtime loan/cancellation controls. They do not separately inject every new
client allocation/mapping failure. The new actual server controls run natively.

Both freestanding targets passed strict library/all-binary service Clippy and
workspace formatting. Forty AArch64 and thirty-seven x86-64 service images were
rebuilt and signed. The separate AArch64 Sitas image was reused with the same
hash and unchanged inputs. The lockfile is unchanged. The fresh production
advisory gate passed all 182 packages against RustSec
`a78282a1414d6c28cc228c4eae3570451affb0dc` (1378 advisories): zero
vulnerabilities/warnings, no ignores or target filtering.

| Passing native execution, four LPs / fresh storage | Deferred pass / fail / pending | HTTP checks | Current server/client probe executions | Real RNG accounting executions |
| --- | --- | --- | --- | --- |
| AArch64 S3 + security + HTTP, SMMUv3, corrected fixture | 22 / 0 / 0 | 14 | 2 / 2 | 2 |
| x86-64 security + HTTP, AMD IOMMU, corrected fixture | 18 / 0 / 0 | 14 | 2 / 2 | — |
| AArch64 Kafka, SMMUv3 | 20 / 0 / 0 | — | — | — |

Both final security runs match the complete current signed-bundle maps and
their archived kernel hashes. Kafka's earlier passing run differs only in the
unused `security_probe.elf`; every production service image is byte-identical
to the final bundle. The complete earlier map is recorded separately. The
fixture-only correction did not require replaying Kafka qualification.

The signed probe uses an unregistered local peer and the production server
handler with public deterministic bytes, never installed as cryptographic RNG.
It checks retired/missing/oversized/invalid requests before the fill callback,
minimum/maximum counts and untouched suffixes, malformed count/memory/
connection/size replies, device rejection, explicit cancel, completed-result
Drop, late-result disposal with the original deadline, full-queue rejection,
accepted silent-peer timeout and recovery of all eight slots. Both publication
generations retain the existing `0xffff` result.

The real signed VirtIO RNG domain on AArch64 supplies additional controls using
the live ordinary probe root, after its application tests finish:

- Missing/retired/oversized requests reject without reply backing. A read-only
  loan reaches the distinct producer, writable mapping rejects, and safe
  caller writes afterward confirm this loan's revocation.
- A completed but unobserved reply remains charged to the caller; closing the
  pending operation releases that page without adopting its result.
- Thirty-two retained replies consume exactly 32 additional caller pages and
  objects; the next caller allocation rejects. Producer page/object usage stays
  at its captured baseline throughout. An independently funded fill succeeds
  while those pages remain retained. This is a focused independent-sponsor
  progress check, not sustained flood fairness.
- One RNG page and the existing TCP/IP retained page move to the kernel fixture
  before the original sponsor closes. Both remain charged to that exact retired
  account. Physical release then refunds it and leaves a successor root's
  account unchanged. The fixture does not substitute a timer sponsor or
  saturate the shared kernel account.

The x86 runner has no VirtIO RNG device: it executes the local protocol/client
controls, not the real RNG accounting controls. Cortex-A710 lacks RNDR, so the
AArch64 S3/Kafka operations exercise the real owned caller-buffer RPC path.
S3 verified its 33-byte object round trip. Kafka completed its full controlled
route-leader stop/restart oracle; exact counters and fault-window transport
diagnostics are retained in the evidence.

## Failed runs and limits

The first strict AArch64 Clippy run rejected obsolete `CallResult` and
`PendingCall` imports after the DNS polling helper was removed; final target
checks pass after removing them. The first sandboxed advisory refresh could
not fetch GitHub; the same production wrapper passed with network permission.
No offline/stale scan substituted for that final result.

The first AArch64 S3/security and AMD security runs failed: respectively
**21 passed / 1 failed / 0 pending** and **17 / 1 / 0**. Both new probes exited
before reporting completion; the coordinator panicked at
`self_test/el0_security.rs:116` with
`security probe exited before adversarial application grant checks`, then
reported scoped-launch-security failure. The initial AArch64 S3 oracle passed,
but the host also recorded `TimeoutError: timed out`; this remains a failed
combined execution.

The fixture had unwrapped a read-only loan to an endpoint in its own address
space. `memory::object::transfer::prepare_loan` explicitly rejects
`owner == target` with `WrongOwner` before submission. The correction asserts
that rejection and retains/maps the original owner; actual server mapping and
loan-revocation rejection uses the distinct signed RNG domain above. No kernel
ownership rule, capacity, deadline or retry budget changed. Original failed
artifact maps and diagnostics remain separate from corrected repeats.

The passing HTTP probes used nine existing transport retries: AArch64 had one
timeout and four connection resets; AMD had four connection resets. Kafka
emitted `TLS handshake transport failed (6)` at guest time 32.003887, then its
complete success oracle at 38.400544 (retry attempts 6, output-route produced 5,
lag 0). These diagnostics remain recorded alongside passing outcomes. They do
not establish causes for
historical scheduler/IPC/retirement intermittents. No old producer-allocation
negative-control image was executed: the narrow sponsorship conclusion rests
on the removed allocation path and actual caller/producer accounting checks.
Forced new-client allocation failure, returned-memory mapping failure,
unmapping failure and kernel waiter-admission rejection were not separately
injected in this batch. Server read-only mapping rejection is an executed
control, not evidence for all those other paths.

Two final manual passes produced 252 pages, no undefined references and the
prior 189 global overfull warnings. An initial long inline module name added
one overflow; descriptive wording removed it. Contents page 14 and affected/
adjacent pages 117–119 and 197–201 were rendered and visually inspected. The
rest of the manual was not re-audited.

The producer-funded RNG reply-retention path is corrected in this documented
scope. SEC-06/07/14/15/18 remain partial; totals stay **29 corrected, 6 partial,
3 open** and production trust remains disabled. Authorized flooding, shared IPC
metadata/mapping pressure, hardware timeout recovery/quiescence, full TLS
erasure, third-party provenance/license and broader principal/heap admission
remain separate work. Kernel edits here are opt-in fixture checks; production
accounting, IPC and DMA rules are unchanged. Intel, the full signing/launch
suite, formal models and GCM build-guard controls were not rerun for this batch.
