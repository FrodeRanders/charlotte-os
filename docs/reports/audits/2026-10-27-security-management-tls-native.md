# Native management TLS and operator authentication

Date: 2026-10-27. Parent: `cd04a7ba` (portable TLS engine). This report records
the native integration that follows that component, preserving its historical
unwired checkpoint. **SEC-10 advances from open to partial.** The ledger now has
29 corrected in scope, seven partial and two open findings out of 38.
Production remains disabled; this is development-root qualification.

## Result and authority

Both native management interfaces require mutual TLS 1.3: status on `443` and
deployment on `7444`. Direct node and configured VIP paths terminate in the same
owning TLS adapter, with no exposed plaintext backend or verification fallback.
An exact leaf SHA-256 enrollment supplies `Reader` or `Administrator`; a client
CA signature alone grants no access. Readers can observe, while mutations
require administrator authorization before controller dispatch. Existing signed
deployment/release/operations/shutdown/ingress-policy bodies remain required.
Audit decisions include certificate digest, role, permission and a nonreused
local connection ID, without keys or request bodies.

One complete `CMGMTL01` profile contains server key/chain, client roots, exact
enrollments and all eleven adjustable TLS limits. Its signed HPKE `CMGTENC1`
envelope binds the entire canonical selected admission policy, including its
cluster, revision and four keys. Signature/context verification precedes private
work. Boot opens it using the same `PreparedBootTrust` recipient owner **before
trust publication** and launches both services through the existing owning
read-only profile transaction. It creates no independent service trust source.

`CATTEN_MANAGEMENT_TLS_RESOURCE` is an absolute ciphertext file input to the
kernel build. The current delivery mechanism requires rebuild/restart to
replace it; it is not a live update interface. Missing resources disable both
listeners while preserving cooperative shutdown. Invalid envelope/policy input
rejects steady-state composition, and invalid DER/configuration prevents the
affected service from opening its listener. These failure policies have source
and host coverage; a dedicated missing-resource/invalid-DER native boot campaign
is still required.

Replacement/revocation requires an advanced installed policy revision, complete
replacement resource and service restart. Same-revision competing signatures
are not a unique installation or rollback-resistance proof. Public development
roots do not supply protected provisioning, recipient custody or platform
authentication. Each client gets a fresh positive time-service reply before
TLS activation under its original handshake budget, but SNTP/holdover remains
unauthenticated. SEC-04, SEC-08, SEC-09 and SEC-15 retain their separate gaps.

## Ownership, readiness and adjustable dimensions

`management_io::Connection` retains the exact accepted RPC before the erasing
session and original owned socket. Observation/cancellation finishes before
those fields are released. Returned receive memory/mappings remain owned;
encrypted output prefixes remain retained until positive receipts. Final output
also confirms encrypted close-notify. A body-deadline response cancels the
accepted receive before output; cancellation failure follows the existing
abort-with-owner-live policy. Timeouts and rejected CQ binding cannot authorize
replay. Queue-full retries occur only before admission with the original budget.
Logical socket close uses the existing consuming close pool and is no physical
cleanup proof. C18 is updated without a new recovery registry or authority.

Deployment retains its concurrent input/control/output/close reactor and exact
CQ interests. The sequential keyhole waits on its CQ and lifecycle. TLS local
progress rotates with other deployment sources; an idle client does not cause a
blocking deployment handshake or timer-driven receive polling.

`charlotte-launch/src/management/limits.rs` is the common TLS policy definition.
The complete limits file rejects missing/duplicate/unknown/nondecimal settings
and inconsistent dimensions before private input/output. Counts, certificates,
wire buffers, fragmentation, original deadlines and local work co-vary; see the
[configuration contract](../../reference/management-tls.md).
Central build-time dimensions are `MAX_PROFILE_BYTES = 65536` and
`USER_STACK_PAGES = 32` (131072 reserved bytes, committed on demand). Both native
services use that same stack reservation. HTTP framing, aggregate bytes,
concurrency and remote close retain their separate existing limits.

Host management commands now require CA/client-certificate/private-key PEM files
and verify the expected DNS/IP server identity before submitting HTTP. Their
socket wrapper recomputes one original deadline for internal Rustls I/O too.
Private DER/PEM/profile input uses bounded no-follow regular-file reads and,
on Unix, effective-user ownership with no group/other permissions. Browser
PKCS#12 enrollment and file-based resource creation are documented in Markdown
and the manual's Configuration chapter.

## Executed evidence

The tracked [evidence record](2026-10-27-security-management-tls-native-evidence.json)
contains source/lock/compiler identities, exact final artifact hashes, complete
host/native control names, relevant failed-run diagnostics and the fresh advisory
report. Temporary logs are not required to interpret these results.

| Check | Result |
| --- | --- |
| Selected host regression suite | **434 passed, 0 failed, 0 ignored**, 26 test binaries |
| Verified host HTTP fixture suites | **43 passed**, deployment status/generation, release generation/startup, complete responses and original deadlines |
| Optimized TLS engine tests | **11 passed, 0 failed, 0 ignored**, including OpenSSL interop |
| Common profile/envelope controls | Eight tests: full limits/roles, every prefix/extension, reserved fields, duplicates, policy substitution, wrong recipient/signature and resigned ciphertext authentication failure |
| Actual native service target JSONs | Complete signed bundles and strict all-bin Clippy pass on ARM and x86 |
| Kernel target JSONs | Linked native runs plus strict Clippy on both architectures |
| Final ARM TCG run | **19/0/0** deferred self-tests; **21** host PASS controls; both configured VIP interfaces qualified |
| Final x86 TCG security + HTTPS run | **18/0/0** deferred self-tests; **20** host PASS controls |
| Fresh complete advisory gate | **205 packages, 0 vulnerabilities, 0 warnings**, RustSec `99670cfc0bc574433034b75876a4b791ccaa2efb`, 1380 advisories |
| Workspace formatting and whitespace | Passed |
| Manual | Two final LaTeX passes, 253 pages; changed/adjacent pages rendered and reviewed; no overfull warning in Configuration or undefined references; 190 preexisting global overfull warnings remain |

Both final native runs use fresh disposable P-256 credentials and the public
development authorities, with actual entropy and positive time-service replies.
Python/OpenSSL-backed verified TLS clients exercise reader/administrator reads,
missing and CA-issued-but-unregistered clients, wrong root/name, plaintext
rejection, early reader POST denial and healthy recovery. The host signing tool's
Rustls/ring client also reads over verified mTLS and rejects a wrong server name
before HTTP submission. The empty ingress-policy observation correctly returns
HTTP 404; this is not a TLS rejection or a fabricated active policy.

All six host HTTP suites now use one disposable mutual-TLS fixture owner per
test process. Standard-library TLS verifies client certificates; the real CLI
verifies the server CA/IP identity. The generator's explicit test-only server-key
retention keeps that PEM inside the mode-0700 fixture owner until cleanup; its
normal native-boot behavior still removes transient server/profile key files.
The existing startup retries, exact signed envelopes, uncertain-outcome fencing,
malformed acknowledgements and monotonic budgets are exercised over TLS.
These mock HTTP servers do not substitute for native exact-enrollment checks.

On ARM, status forwarding targets `10.0.2.42:443`. After direct deployment
controls, the isolated QEMU monitor remaps its existing deployment forward to
`10.0.2.42:7444`. Both VIP observations verify the certificate against the VIP
IP; a VIP reader mutation also rejects. This is a single-node configured DSR
fixture, not multi-node leader-movement qualification.

Healthy deployment requests retain their existing three-second single-attempt
budget while up to three peers hold incomplete authenticated requests. Final
mixed-peer replies arrive in **1.053 s (ARM)** and **0.918 s (x86)**; the evidence
records all measured cases. Original total body deadlines reject trickled input
in **29.48 s (ARM)** and **29.69 s (x86)** after the completed host handshake,
then recover. No test budget or admission limit was enlarged to pass these
controls. Existing EOF, reset, unauthenticated idle peers, malformed lengths,
oversized framing and invalid signed-body rejection remain covered.

The x86 signed security probe executes both publication generations. Its local
controller CQ fixture retains the original signed-controller checks through a
generic owning socket carrier. The native output CQ fixture now qualifies the
underlying moved wire-frame ownership/terminal TCP/UDP rejection, using a local
pending-plus-socket owner: it does **not** fabricate a TLS identity to instantiate
the public TLS output pool. TLS output behavior has host partial-receipt/fairness
tests and live HTTPS coverage; native fault injection of its new mapping,
binding and waiter-admission paths remains separate work.

## Failed runs retained separately

1. **Initial native stack faults:** ARM kernel
   `3852bc9119e4176bd677808c54536ee5dd9364a170326176601be50e31c20156`
   used the general four-page service stack. Deployment startup faulted at
   `ELR=0x20a24 FAR=0xffff70`; status faulted during its first handshake at
   `ELR=0x4893c FAR=0xfffad0`, resolved at capture time to
   `_embassy_crypto_p256_ecdsa_verify`. The guard addresses and passing repeats
   with the central 32-page reservation support stack exhaustion. The original
   service ELFs were not separately retained; the kernel identity and fault
   diagnostics are preserved rather than claiming a fresh old-artifact replay.
2. **Authenticated body deadline:** the first enlarged-stack ARM run passed
   concurrent HTTPS controls but returned EOF for trickled input. The native
   diagnostic was `[deployd] response ended: Transport(State)` at 96.610828 s.
   The accepted receive still lived inside the TLS owner when output began.
   Explicit cancellation before response fixes that transition; final runs
   return the deadline rejection and recover. No accepted receive is replayed.
3. **Parallel host fixture disappearance:** the preliminary host run aborted
   while OpenSSL could not open `ca.key` (`No such file or directory`), then
   cleanup also found the directory absent. Timestamp-only fixture naming could
   collide; names now include an atomic process-local sequence. The final
   parallel suite and optimized engine suite pass. This is fixture evidence,
   not evidence of a native credential-directory attack.
4. **Host expectation error:** an ARM VIP run passed the native TLS/role controls
   but the new tool assertion demanded success for an absent ingress policy.
   The actual verified response was HTTP 404 with
   `{"error":"ingress policy not found"}`. The corrected expectation retains
   that absence result, and separately demands verification failure for the
   wrong server name. The complete final probe passes.
5. **Interrupted x86 signing:** an earlier x86 service-signing subprocess exited
   `137` with `Killed: 9`. No cause was established. The later complete bundle,
   kernel and native security/HTTPS runs pass; that does not explain the signal.
6. **Partial manual VIP probe:** a preliminary direct monitor probe verified the
   deployment VIP against its IP certificate, then a status handshake reset.
   The cause was not established. The final automated sequential VIP checks
   pass both interfaces; those results do not explain the earlier reset.

Routine listener-handoff transport retries in the existing keyhole liveness
probe remain explicit in the evidence. The new identity controls retry only
pre-HTTP transport admission, never submitted operations or certificate
verification. Historical IPC exact-exit/persistence failures remain unresolved
where previously recorded; this batch does not reattribute or close them.

## Remaining acceptance work

SEC-10 remains partial until native installed-resource rotation/revocation,
expired credentials/restarts, missing/invalid startup-resource behavior,
multi-node leader changes, broad pressure and new adapter fault paths are
qualified. Host engine controls cover validity, clock loss, reconnect removal,
tamper/replay and partial receipts, but do not substitute for those native
campaigns. Protected installation/custody/rollback state, authenticated cluster
peers and authenticated time remain SEC-04/08/09; Rustls/Embassy internal copies,
read-only launch backing, compiler/allocator history and forced-termination
erasure remain SEC-15. No production-readiness or full cleanup proof is claimed.
