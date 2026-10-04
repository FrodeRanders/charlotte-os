# Security remediation — 2026-10-03

This records implementation passes following the
[security audit](2026-10-03-security-audit.md) of revision
`42183c57ce4c0b32a6010246f6eee1b6262ebb4e`. It is not a declaration that the
audit is closed or that CharlotteOS is ready for hostile production workloads.
Implementation and validation span 2026-10-03–04 local time.

## Finding ledger

“Implemented” means the identified code defect has a correction in the code.
Validation scope is listed separately; it does not imply that every
acceptance test proposed by the audit has run. “Mitigated” leaves part of the
finding open. “Open” means no correction was implemented in this pass.

| Finding | State | Correction or outstanding requirement |
| --- | --- | --- |
| SEC-01 | Implemented | Raw explicit mapping addresses and complete ranges are checked before address normalization or page-table mutation; both architectures reject user-accessible mappings outside the user window. |
| SEC-02 | Implemented | Kernel records the exact admitted descriptor digest against the domain generation; grantctl obtains attestation instead of accepting another signed policy for the same artifact name. |
| SEC-03 | Implemented | Legacy and capability mailbox queues are domain-local and discarded on domain teardown. |
| SEC-04 | Mitigated | Builds and boot logs identify development trust; production and unknown modes fail closed in scripted and direct kernel builds. Protected bootstrap roots, fixture-free production provisioning and recipient-key custody remain unimplemented. |
| SEC-05 | Implemented | tcpip rejects raw frame ingress unless the authenticated sender is the exact live, kernel-designated frouter. Separate socket/VIP binding policy remains future hardening. |
| SEC-06 | Mitigated | HTTP EOF, peer-raced accept and transport failures close one connection, not the server; listener-setup resource failures retry with backoff. httpd has a five-second request wait and bounded send retries; deployd has five-second header and thirty-second total receive budgets. Serial admission remains vulnerable to sustained connection floods. |
| SEC-07 | Partially implemented | Memory-object backing pages/counts, completion timer events, endpoint records/queue backing, retained completion objects/detached results, CQ registrations/kernel backing, endpoint-close registrations, completion/CQ/IPC/lock scheduler waiters and connection/pending-call/reply-token record counts have generation-scoped domain/node admission and platform reserves. Charges survive transfers, deferred cancellation, delegation, retained references or detached notifications as appropriate. Waiter admission precedes parking; owning cancellation handles competing wakes and reaping. Timed completion admission failure retains its owner; untimed completion/IPC waits preserve borrowed-buffer safety. Timed park/watchdog setup is non-preemptible. IPC call/reply preparation precedes attachment transfer, and retirement fences receive/connection publication before teardown. Aggregate limits for loader/heap/page tables (including physical CQ mappings), the complete capability namespace, legacy-observer metadata, general weak-only storage, other timer paths and comprehensive kernel metadata remain open. |
| SEC-08 | Open | Authenticate enrolled nodes and control/data peer traffic, add replay protection, and bound discovery state. A trusted L2 segment remains an explicit deployment prerequisite. |
| SEC-09 | Open | Distinguish authenticated security time from observational SNTP/holdover; enforce freshness and uncertainty at security-policy gates. |
| SEC-10 | Open | Authenticated encrypted access to node and cluster management, browser-client provisioning, and access policy remain necessary. |
| SEC-11 | Implemented for scoped applications | Scoped application mapping uses the configured artifact key; grantctl relies on launcher attestation under the configured deployment key. Independent roots are tested through real scoped launch and grant IPC. The complete S3/Raft release pipeline with those roots remains to be tested. Bundled platform-service trust and production provisioning still belong to SEC-04. |
| SEC-12 | Open | Attenuate local object-store authority to object sets/namespaces; retain an explicitly separate administration endpoint. |
| SEC-13 | Implemented in this repository | Signing/decryption commands enforce restricted bounded key files; generation never prints private material; build/deployment/shutdown scripts pass paths and reject the old secret-valued environment variable without expanding it. Only the exact public artifact fixture retains warned argv compatibility. Sibling broker/Durga callers still need file-path migration; host custody, ACL review and agent/HSM signing remain separate work. |
| SEC-14 | Mitigated; audit corrected | Cargo.lock is already tracked. Main build/test runners and CI now enforce --locked; CI actions are commit-pinned, token permissions are read-only, and checkout does not persist credentials. Advisory/license scans and a release dependency inventory remain. |
| SEC-15 | Mitigated | SigV4 prefixed secret, derived keys, HMAC block/pads and inner digest use zeroizing owners. TLS record buffers are wiped after dropping their borrower, including handshake failure. This is not a complete audit of crypto-library state or compiler-created secret copies. |
| SEC-16 | Implemented | grantctl polls bounded concurrent operations with per-sender/generation limits and total deadlines. Non-parking authorized lookup avoids a shared name-service waitlist leak. Acquisition retries and publication waits have total deadlines. A two-application cancellation stress and silent-endpoint publication timeout pass in the guest; many-client fairness and controller-replacement testing remain. |

## Enforced contracts

### User virtual addresses

`charlotte_launch::user_address` defines the current shared ABI: non-null
addresses in the lower 47-bit window, with page-aligned explicit mapping bases
and checked, nonzero lengths. The entire range must fit; validating only the
first page is insufficient.

The memory and MMIO syscalls reject the original register value before creating
a `VAddr`. This matters because normalization can turn an invalid raw integer
into another address. Memory-object and MMIO adapters check complete ranges
before changing mapping state; AArch64 and x86-64 page mappers provide a further
guard for user-accessible page types. The ELF loader uses the same window.
This does not attempt to enable a wider hardware address space.

### Launch-bound authority

The scoped launcher verifies the configured artifact and deployment roots,
descriptor/ELF identity and digest, then stores the descriptor SHA-256 in the
generation-bearing domain authority record before starting the application.
`LaunchDescriptorMatches` (syscall 83) compares that record, not a descriptor
revision supplied by the caller. Only the exact kernel-designated grant
controller can obtain a positive attestation.

The controller still checks the authenticated principal, exact service name
and granted rights. A newer or separately signed descriptor cannot enlarge the
authority of an older running application. No application receives the name
service's administrative connection. See the
[controller reference](../../reference/capability-grant-controller.md).

The kernel network launcher similarly records the exact frouter occupancy.
`IsFrameRouter` (syscall 84) checks its ASID, generation and current lifetime.
The ordinary TCP/IP CALL capability therefore no longer authorizes `OP_FRAME`.
Restarting a router requires explicit kernel designation of its replacement;
registration under the same name is insufficient.

### Bounded owning operations

Each pending grant owns its incoming message, name-service pending call,
requested rights and monotonic deadline. It is cancelled by dropping that
owner on expiry, failure or shutdown. Admission is bounded to 32 outstanding
operations, four per sender/generation, and 16 new messages per reactor cycle.
These limits provide bounded concurrency, not an aggregate per-principal
resource budget across all that principal's domains.

The controller uses the new immediate `OP_TRY_LOOKUP_FOR_GRANT` (name-service
opcode 17), with the same principal/role/rights checks as deferred opcode 15.
An absent name returns `ERR_NOT_FOUND` without parking any token. This avoids
stranding cancelled controller requests in the bounded shared waitlist.
`grant_client::acquire` handles temporary unavailability with 100 ms retries
inside a single five-second deadline and drops its pending call on expiry.
The legacy deferred lookup ABI remains unchanged. Its general waitlist
cancellation/reclamation behavior still needs separate hardening and stress
tests; this pass prevents grant-controller traffic from contributing parked
entries there.

HTTP request failures now leave the serving loop alive and release the socket
through its owner. Receive deadlines are total budgets: short retries do not
reset them. Both HTTP services remain serial, so this pass cannot establish
availability under an adversarial sustained connection load.

TLS record buffers have one owning guard whose destructor zeroizes and frees
them. The TLS connection is dropped first. Construction failure follows the
same ownership ordering without a separate manual cleanup ladder.

### Reproducible build inputs

The main AArch64/x86-64 build runners, service/user bundle builders, signer
invocations, host-test wrapper and CI compile/lint commands enforce the
tracked workspace lockfile with `--locked`. The misleading `Cargo.lock` ignore
entry now explicitly exempts the root lock, while obsolete per-package lock
files remain ignored. An intentional dependency update must update the
lockfile explicitly and be reviewed with the manifest change.

CI action references were resolved from the official repositories using
`git ls-remote` and pinned without changing their selected major versions:

| Action/reference | Pinned commit |
| --- | --- |
| actions/checkout v5 | fbc6f3992d24b796d5a048ff273f7fcc4a7b6c09 |
| actions/setup-java v5 | b6effb05e454b25005698d916606bdc6ffcbf961 |
| dtolnay/rust-toolchain master | 7e38f4b43b4db5c8dd498af069a4f6196df1d067 |

The workflow token is limited to `contents: read` and checkout disables
persistent Git credentials. Pinning provides immutable references, not proof
that the referenced code is safe. Rust nightly, the sibling Sitas revision and
the checksum-verified TLA+ jar retain their existing controls. Runner/system
package versions and downloaded advisory data still need release-level
provenance; no dependency safety claim is made.

## Validation

Completed checks:

- Host-testable library suites and cluster-sign self-test through
  `scripts/run-host-tests.sh`, including user-range rejection, deadline boundary
  tests, existing SigV4 vectors, and reply-token raw-boundary adoption/drop,
  successful consumption and failure cleanup.
- AArch64 and x86-64 signed userspace service bundle builds.
- AArch64 kernel and userspace Clippy with `-D warnings`; x86-64 kernel Clippy
  with `-D warnings`.
- An isolated AArch64 no-network boot: **17 passed, 0 failed, 0 pending**.
- An isolated AArch64 network/HTTP boot: **19 passed, 0 failed, 0 pending**, plus
  host node and cluster metrics checks, demonstrating the authorized frame
  path still works.
- HTTP host availability checks passed for initial metrics, EOF recovery,
  four abortive peer closes, idle-client timeout/recovery, and subsequent node
  and cluster metrics. Resets during serial listener recovery were observed
  and remain documented below.
- Shell syntax checks, CI YAML parsing, full-SHA action-reference and read-only
  permission assertions, `cargo fmt --all -- --check`, and `git diff --check`.

The final network capture used kernel SHA-256
`87692a78ece7824f447f762b644e94282edf5b7272860bc443a2c7c6e68f4506`.
Local evidence is in `/private/tmp/charlotte-security-http-run.log`,
`/private/tmp/charlotte-security-network-20261003-serial.log`, and
`/private/tmp/charlotte-security-host-tests.log`; these temporary logs are not
version-controlled and may be removed by later test runs.

Kernel self-tests cover raw null, noncanonical and kernel-half addresses,
complete-range boundary crossing without leaving a mapping, cross-domain
mailbox isolation, mailbox teardown, descriptor mismatch and generation
mismatch, absent descriptor policy, retired occupancy, controller-query
rejection for an ordinary domain, and rejection of ordinary router identity.
The launch library's host harness is now enabled so its new range test runs
in the existing host-test wrapper instead of silently being skipped.

`scripts/tests/test-http-keyhole-liveness.py` is integrated into `--http-test`
before the runner terminates its guest. It checks EOF/abortive-close recovery
and a successful metrics request while an idle client remains open. Its result
must be recorded separately from the kernel self-test bitmap. The probe reads Content-Length
rather than requiring a particular TCP teardown and allows at most three
attempts, with three-second backoff after transport errors so retries span the
five-second idle budget; malformed HTTP/JSON is not retried. This is
bounded availability/recovery evidence, not a lossless-transport assertion.

The initial negative HTTP captures exposed repeated resets during the serial
listener's recovery interval; immediate retries did not span its timeout. The
bounded, spaced probe subsequently passed EOF and idle recovery. Serial
admission and this observable interruption remain part of the open SEC-06
availability work; a successful later request does not establish flood
resistance or uninterrupted admission.

These runs used named `security-*` instances and separate NVMe images. No
existing soak guest or its persistent store was stopped or reset. The first
network launch was blocked by sandbox port-binding restrictions; the successful
network run used approved execution outside that sandbox.

Not yet validated: x86-64 guest execution, mutually hostile live ELF mapping
attempts, custom independent trust roots across a complete deployed release,
grant-controller stalled-target/publication/cancellation stress, deployd
slow-body tests, or TLS failure cleanup with allocator instrumentation. No
dependency-advisory scan, hardware attack experiment or cryptographic
implementation proof is claimed.

## Next implementation order

1. Extend scoped-application integration coverage to forced generation reuse,
   controller replacement, allocation failure and many-client fairness. Test
   independent artifact/deployment roots through the complete remote S3/Raft
   release pipeline; direct scoped-launch/grant tests are documented below.
2. Implement protected production bootstrap trust and recipient-key provisioning;
   only then enable production images without fixture fallback. Migrate sibling
   broker/Durga templates to the new signing file-path interface. Keep developer
   fixtures visibly identified and separate from real credentials.
3. Extend admission to the remaining capability, connection/call/observer,
   CQ/weak-only storage, other timer and loader/heap/page-table budgets. Add
   typed launch-policy limits and observable counters. Preserve rollback and
   delayed-release accounting, and test essential-service progress under
   sustained hostile pressure, not only bounded fixture exhaustion.
4. Separate security-time provenance from ordinary clock synchronization, then
   authenticate management and enrolled node traffic with bounded replay state.
5. Attenuate local storage authority and make HTTP admission concurrent and
   bounded. Add advisory/license policy checks and release provenance for the
   locked dependency graph, toolchain and pinned action inputs.

Until those controls exist, retain the audit's operational restrictions: use
only public fixtures, no real credentials under development recipient keys,
trusted network segments and management access, and no mutually distrustful
application workload assumptions.

## Follow-up: signing and explicit development images — 2026-10-04

The first pass was committed as `b3a77f6c`. This follow-up addresses signing
input and makes the remaining production-trust boundary explicit.

`elf-sign`, `deployment-sign`, `release-sign` and `shutdown-sign` take a private
key-file path. Operational signing and recipient decryption use the same
restricted reader. It opens once with Unix no-follow/nonblocking flags, checks
the opened descriptor's type, owner and group/other permission bits, limits
input to 4096 bytes, and zeroizes read, filtered and decoded buffers. Known
public fixture contents are exempt from secrecy-related mode checks; filenames
are not an exemption. Non-Unix real-key use is refused pending ACL support.
Hex decoding now rejects malformed UTF-8 hex without slicing panics.

Key generation uses exclusive creation, creates owner-only private files,
rejects existing output files and does not print private bytes. The signer
rejects real raw hex argv keys without echoing them. It retains warned
compatibility only for the exact publicly known artifact fixture, so existing
broker demos are not silently broken. A caller that already put a real key in
argv has exposed it before rejection; the signer cannot undo that exposure.

In-repository build/deployment/shutdown callers pass file paths and reject
`CLUSTER_SIGN_PRIVATE_KEY` by checking presence, never value—even with `bash -x`.
Use `CLUSTER_SIGN_KEY_FILE` instead. Signer invocations in these callers now
select the repository's pinned toolchain outside the bare-metal Cargo config.
The host-test wrapper runs signer unit tests, and CI lints its test target too.

`CATTEN_TRUST_MODE` defaults to `development`. The scripts and kernel build
script refuse `production`, unknown and empty values. Build output and boot
logs warn that fixture trust must not protect real credentials. This is an
intentional safety gate, **not a production provisioning implementation**.
No production root is loaded and no real recipient secret is embedded by this
change. See [Signing and development trust](../../guides/signing-and-trust.md).

Follow-up validation:

- The full host-test wrapper passed, now including seven signer unit tests.
  New coverage exercises generation, 0600/0400 inputs, rejection of 0644/0640/
  0602 real-key files, symlinks, FIFOs, non-regular/oversized/malformed inputs,
  non-overwrite and rollback, public-fixture-only argv compatibility, and
  file-based deployment/release/shutdown signature verification.
- Shell tests passed for the development default, production/unknown/empty
  rejection, early refusal by both runners and bundle/user builders, and
  non-disclosure of the retired secret-valued variable under `bash -x`.
- Direct kernel Cargo checks deliberately failed for production, unknown and
  empty modes at the build-script gate, before producing a new kernel image.
- AArch64 and x86-64 service bundles built and were signed using file paths.
  Kernel Clippy passed for both architectures; host signer Clippy passed with
  all targets on the pinned toolchain. Formatting, diff checks, shell syntax
  checks and CI YAML parsing passed.
- A separate no-network AArch64 guest passed **17 tests, 0 failed, 0 pending**,
  and emitted the explicit development-trust boot warning. Kernel SHA-256:
  `4b9428cc95d31c4d65abe78f35e321ebd6138acf8e9c41dfd0f2412c71963480`.
  This used only the named `security-signing-20261004` instance and its own
  storage image; existing soak guests/storage were not touched.

Temporary follow-up evidence is in
`/private/tmp/charlotte-security-signing-host-tests.log`,
`/private/tmp/charlotte-security-signing-aarch64-services.log`,
`/private/tmp/charlotte-security-signing-x86-services.log`,
`/private/tmp/charlotte-security-signing-boot-run.log`, and
`/private/tmp/charlotte-security-signing-20261004-serial.log`.
The earlier network/HTTP boot evidence belongs to the first pass, not this
follow-up. This pass did not exercise a production provisioning path, x86-64
guest execution, OS ACL enforcement, a compromised signer workstation, or
sibling broker/Durga real-key packaging.

## Follow-up: adversarial scoped launch and grant IPC — 2026-10-04

The signing/development-trust pass was committed as `462c0c3a`. This follow-up
adds `--security-test` and a real EL0 `security_probe`, using the production
scoped-launch verifier and grant controller. Its diagnostic launch metadata
contains no additional capabilities. Two fresh, independent signing roots are
generated per run; only the public roots and signed fixtures enter the kernel.
The bundled platform services still use explicitly identified development trust.

The probe's ten result bits cover:

| Bit | Required observation |
| --- | --- |
| `0x001` | Both descriptors verify under the fresh deployment root; the alternate names the same signed ELF but has a newer sequence. |
| `0x002` | The valid alternate descriptor cannot replace the application's admitted policy. |
| `0x004` | The admitted CALL-only tcpip grant cannot acquire SEND/CALL rights. |
| `0x008` | An undeclared logical service is denied without returned authority. |
| `0x010` | An explicitly granted application endpoint can be published without ambient naming authority. |
| `0x020` | Returned application connections cannot be re-delegated; explicitly mintable connections pass the corresponding IPC submission control. |
| `0x040` | Missing and available grants coexist, with the available connection completing a real call. |
| `0x080` | Acquisition and calls recover after 384 cancelled requests. |
| `0x100` | Ungranted SEND fails in the kernel, and an ordinary CALL cannot inject a raw frame into tcpip. |
| `0x200` | Publication to a separately hosted, deliberately silent endpoint expires within the helper's budget. |

The kernel also rejects swapped artifact/deployment roots before launch. It
retires and relaunches the primary probe, requires an advancing publication
generation, and rejects the retired descriptor's attestation both immediately
after retirement and while the replacement is live. A second application
generates cancelled missing-service requests concurrently. Every transient
userspace request, returned connection and endpoint remains a typed owner.

This work found another unbounded wait: `grant_client::publish` previously used
`PendingCall::wait()`. It now polls under a five-second total monotonic budget;
expiry drops and cancels the pending call. Publication is not automatically
retried because a lost reply does not establish whether registration happened.
Applications must reconcile that state before choosing a retry.

During harness development, one negative assertion incorrectly expected a
status-bearing error from an ABI that returns a zero capability on submission
failure. A second attempted timeout fixture used same-domain memory-copy IPC,
which the kernel deliberately refuses. The corrected tests use a mintable
positive control and a separate silent domain. Those earlier test failures are
not evidence of a production kernel panic at either boundary.

Validation:

- A four-LP AArch64/TCG guest passed **19 tests, 0 failed, 0 pending**, including
  all ten probe bits in two successive scoped launches. The concurrent actor
  submitted 4,416 cancelled requests in the first successful capture.
  A fresh-root repeat with the stronger live-replacement fencing assertion
  passed all 19 tests again, with 4,468 concurrent cancelled requests. Both
  captures reused the primary probe's ASID, although the fixture does not force
  that allocator choice.
- The host test runner passed, including all seven signer key-file tests.
- Both signed service bundles built. Service Clippy passed on AArch64 and
  x86-64; kernel Clippy passed with the AArch64 security feature and the ordinary
  x86-64 configuration. Formatting, diff and shell syntax checks passed.
- The runner rejects no-network, HVF and isolated-suite combinations before
  building. Fixture generation refuses a nonempty output directory.
- CI's AArch64 boot job now enables this regression verifier; x86-64's guest
  job remains unchanged.

The initial successful kernel SHA-256 was
`eb205b0a46182d2f26a1464485d5bdbfffde9e09508b4fa3be32e0715c030039`.
The repeat's kernel SHA-256 was
`380a55643365733503adf90e057b08253f70c5f854d44af7218af891734422b6`.
Temporary evidence is in `/private/tmp/charlotte-security-grants-run.log`,
`/private/tmp/charlotte-security-grants-20261004-serial.log`, and
`/private/tmp/charlotte-security-probe-*-clippy.log`,
`/private/tmp/charlotte-security-probe-host-tests.log` and
`/private/tmp/charlotte-security-probe-x86-services.log`. Repeated runs overwrite
the guest logs; signed fixtures remain under ignored `target/security-test`.
Only the dedicated guest/storage instance was used; existing soak workloads
were not stopped or modified.

This is bounded two-application coverage, not a proof of starvation freedom or
aggregate resource containment. Forced ASID reuse, grant-controller restart,
allocation-failure rollback and many-client saturation need additional tests.
Independent roots are exercised from direct scoped launch through application
IPC, not through the complete S3 retrieval/Raft release path. Production root
provisioning, authenticated security time, peer and management authentication,
and the remaining open audit findings are still outstanding.

## Follow-up: aggregate memory-object admission — 2026-10-04

The scoped-launch verifier was committed as `657414ec`. This continuation
partially addresses SEC-07 at the memory-object allocation boundary, using
checked, host-testable admission counters and linear kernel charge owners.

Repeated allocations now consume a sponsoring generation's page and backing
object-count budget. Defaults are 64 MiB (clamped on smaller nodes) and 1,024
objects. The node pool is one quarter of usable RAM and 8,192 objects;
ordinary domains can consume at most three quarters of either limit. Only the
kernel and supervisor-designated platform launches can use the remaining
share. Every actual backing allocation also preserves one eighth of usable
frames, checked under the physical allocator lock.

Moves and loans retain the existing sponsor charge. Copying reserves a new
charge against the caller, not the receiver. This prevents an application
from laundering allocation costs into a privileged service by transferring
buffers. A retired generation retains charges for receiver-held and pinned
objects until final physical release. ASID reuse creates a separate account;
late releases cannot debit a successor. Retirement blocks new reservations
before payload teardown, and transfers reject a retiring or replaced
destination. Failed physical frees quarantine the charge instead of returning
possibly fictitious capacity.

Staged frame allocations and source copy pins have owning guards. Fallible
vector reservation happens before allocating backing frames. Failed quota,
frame or staging allocation returns the reservation and pin through Drop;
successful objects retain their charges through mapping shootdown and physical
release. This is not a complete conversion of kernel bookkeeping to fallible
allocation: registry maps and other subsystems still need review.

Review also changed retirement to drain IPC before releasing its memory
attachments, keeping partial vector transfers available for rollback. The
first integration run with that ordering stalled in the existing mapped-loan
server-death test: revocation called the public unmap path and recursively
requested the lifecycle lock held by retirement. IPC revocation now uses a
separately documented IPC-serialized path without that lock acquisition.
A transient revoking state prevents new mappings, pins and writes while the
registry is released for unmap/shootdown; failed unmap restores the previous
loan state. Direct kernel revocation retains lifecycle serialization.
The stalled capture is preserved in
`/private/tmp/charlotte-memory-budget-ipc-retirement-lock-regression.log`.

A later capture passed the mapped-loan test but hit the supervisor's one-shot
quiescence assertion after its earlier exit wait. The node-wide retirement
marker can change between those checks, and review found that the reaper's
local vector was not covered by that marker. The reaper now retains a guard
through deferred reinsertion and final resource release. A retirement epoch
also rejects a complete transition between live/staged table snapshots.
Teardown waits up to five seconds for a stable snapshot; it does not release a
busy domain. The failed assertion capture remains in
`/private/tmp/charlotte-memory-budget-teardown-settle-regression.log`.

Validation:

- The host runner passed, including four new checked-budget/physical-reserve
  tests for atomic rejection, overflow, release underflow, limit changes and
  reserved-pool boundaries, plus a quiescence snapshot test covering changed
  epochs and in-flight, live or staged threads.
- Synchronous target tests passed for separate page/count ceilings, failed
  copy unpinning, failed close, move rollback, copy/lend sponsorship,
  receiver-held memory after sponsor exit, and late DMA/copy unpinning while a
  replacement generation is live. A deterministic retirement-fence test
  rejects new allocation, move, copy and lend into a retiring generation while
  preserving rollback of an earlier move and reconciling its counters.
- A reservation-only kernel test fills the ordinary page pool, rejects further
  ordinary admission without a leaked local charge, admits platform work,
  drops all reservations and checks exact reconciliation. It deliberately
  does not consume the equivalent physical RAM.
- The four-LP AArch64/TCG guest passed **19 tests, 0 failed, 0 pending**. Both
  scoped EL0 launches passed the expanded `0x7ff` result mask: small allocations
  are eventually refused, scalar IPC succeeds while the allocation budget is
  full, and ordinary grant acquisition recovers after the batch is dropped.
  Concurrent cancellation traffic completed 4,508 requests in the final
  capture with the revocation and retirement-snapshot corrections.
  A fresh repeat with the same implementation also passed all 19 tests and
  both `0x7ff` probes, with 4,484 concurrent cancellation requests.
- Signed service bundles built for AArch64 and x86-64. Service Clippy passed
  on both architectures, as did AArch64 security-feature and ordinary x86-64
  kernel Clippy with `-D warnings`. No x86-64 guest run or PDF rebuild was
  performed in this continuation.

That capture's kernel SHA-256 was
`3c9d09b8cf286be4e039af191c0179cb9672965d50e6f46f50b50783d4479b8b`.
The repeat's kernel SHA-256 was
`6fff1338b0a563cc07991a95ff65069deb336f019e1c62dea354a6634bf0c3f8`;
the runner generates independent signing fixtures for each run.
Temporary logs are `/private/tmp/charlotte-memory-budget-run.log`,
`/private/tmp/charlotte-security-memory-20261004-serial.log`, and
`/private/tmp/charlotte-memory-budget-host-tests.log`. Repeat logs are
`/private/tmp/charlotte-memory-budget-repeat-run.log` and
`/private/tmp/charlotte-security-memory-repeat-20261004-serial.log`. Only the
dedicated security-memory guest/storage instances were used; existing soak
workloads and their storage were not modified.

The [budget reference](../../reference/memory-object-budgets.md) specifies the
ownership/sponsorship distinction, errors, current policy and remaining work.
The allocation ABI still reports zero rather than a specific quota reason;
status-bearing memory operations define `RESOURCE_LIMIT` (16). There is no
external budget telemetry record, descriptor field or userspace policy setter
yet. The reserved share is a pool, not a guaranteed entitlement for each
essential service. IPC capabilities, queues, endpoints, completions, timers,
loader/heap/page-table frames and comprehensive kernel metadata remain outside
these counters. SEC-07 is not closed and hostile multi-tenant operation is not
claimed safe.

## Follow-up: completion-timer admission and cancellation — 2026-10-04

The memory-object and retirement corrections were committed as `84b464e2`.
This continuation addresses another SEC-07 lifetime gap: a completed, closed or
replaced operation record previously could leave an anonymous far-future timer
event queued until its deadline. Recycling completion slots was not a bound
on those retained queue nodes.

Both capability-backed and detached completion timers now reserve event
admission before publication. A domain's event ceiling is its completion
capacity, clamped to 1,024; the service loader currently uses capacity 16. The
node ceiling is 8,192 events, with ordinary domains limited to 6,144. The
remaining share is available only to kernel/supervisor-designated platform
domains. A captured platform designation must match the completion
namespace's generation-qualified identity, preventing inherited reserve access
after ASID reuse.

Each queued event owns its reservation. Cancellation removes it eagerly on the
local LP when possible; a busy or remote queue retains the flagged event and
charge until reconciliation. Domain budget owners are reference-counted by
old events rather than indexed only by recyclable ASIDs. Local and node
counter rollback is checked, and final event destruction returns the charge.

Operation records own cancellation registrations. Timer cancellation produces
a terminal cancelled result immediately, enabling an owned hour-long timer to
be dropped without waiting an hour. Read/write cancellation retains its
existing deferred buffer-ownership contract. A detached cancelled result that
cannot enter a full CQ retains its existing submission slot until delivery.
The observer and cancellation owner are installed before enqueue; teardown
winning that interval causes the cancelled event to be discarded.

Timer, thread-exit and endpoint-close observers capture a weak reference to
their original completion object. Transition and CQ publication require the
exact captured object still to be registered under the registry lock, so a
callback cannot complete a replacement with the same ASID and numeric cap.
Relative deadline conversion also saturates rather than truncating a large
tick count or overflowing addition.

Review of that boundary found that x86-64's APIC initial count cannot represent
very distant logical deadlines in one arm. Its absolute-deadline path now
uses bounded, nonzero hardware checkpoints and leaves the logical deadline in
the queue for rechecking on each IRQ. This avoids the queue's prior
out-of-range panic without notifying observers before their logical deadline.
The representable-count decision has host tests; x86-64 runtime validation
remains outstanding.

Validation:

- The host runner passed, including three new boundary tests for checked
  event-count admission, saturating deadlines and nonzero representable
  hardware checkpoints. `charlotte-lifecycle` now runs 14 tests.
- Synchronous guest tests passed for per-namespace rejection, ordinary node
  pool saturation with reserved platform admission, exact charge reconciliation,
  64 hour-long cancellation/close cycles, mixed capability/detached admission,
  maximum-timeout rollback, retained charges during busy-queue cancellation,
  recovery after purge, and exact numeric namespace replacement with a captured
  old completion. Deferred reclamation is simulated with a busy local queue;
  no actual remote-LP cancellation stress is claimed.
- The final four-LP AArch64/TCG security run passed **19 tests, 0 failed,
  0 pending**. Both scoped launches passed the expanded `0xfff` mask, including
  real EL0 timer saturation, bounded owning-batch drop, 64 additional
  cancellation cycles and short-timer recovery. Concurrent cancellation traffic
  retired after 4,508 requests. Earlier integration runs also passed; the
  maximum-timeout capture before the x86-specific checkpoint adjustment had
  4,456 requests and the same 19-test result.
- Both signed service bundles built. Kernel and service Clippy passed for
  AArch64 and x86-64 with `-D warnings`; formatting and diff checks passed.
  No x86-64 guest/hardware run, sustained hostile-pressure soak, PDF rebuild or
  full node-wide completion-metadata accounting is claimed.

The final capture's kernel SHA-256 was
`f5ff10b2782c47ad34db7d91444ef378532e47355fb8aa18a0015cd13c59ed8e`.
Temporary evidence is in `/private/tmp/charlotte-security-timers-verified-run.log`,
`/private/tmp/charlotte-security-timers-verified-20261004-serial.log`,
`/private/tmp/charlotte-security-timers-bounds-run.log`,
`/private/tmp/charlotte-security-timers-host-tests.log`, and the
`/private/tmp/charlotte-security-timers-*-clippy.log`/`*-services.log` files.
Only dedicated isolated guest/storage instances were used; existing soak
workloads and their storage were not modified.

The [timer reference](../../reference/completion-timer-budgets.md) states the
policy, ABI and verification scope. The manual's overly broad assertion that
all capability tables were bounded has been corrected: these controls cover
completion submissions and completion timer events, not the complete
capability namespace. General completion records still lack node-wide
admission; sleeps, wait watchdogs, observer lists, kernel workers, CQ backing
storage, and comprehensive metadata budgets/fallible allocation remain open.
The platform share is a pool, not a per-service progress entitlement. SEC-07
remains partially implemented, and the audit's operational restrictions still
apply.

## Follow-up: endpoint records and queue backing — 2026-10-04

Completion-timer hardening was committed as `e96252e2`. This continuation
extends SEC-07 admission to endpoint records and their actual queue backing.
A generation's ceilings are 64 records and 8,192 slots; the node ceilings are
1,024 records and 32,768 slots. Ordinary domains share at most 768 records
and 24,576 slots. The remaining pool is available to kernel-designated
platform domains, checked against the namespace's exact generation.

Creation reserves a record and rounded backing before fallible queue
allocation and publication. Growth stages a second charged queue while the
old queue remains charged; failure preserves policy and messages. Shrinking
changes admission only and does not return the retained backing charge.
Checked multidimensional counters roll back failed reservations atomically.

Closure drains messages and releases backing. A closed record retained by a
delegated connection stays charged to its original namespace. Internal
revocation of the last connection, including an unobserved returned result,
now reclaims that record too; previously this path could strand it until
namespace teardown. A retained budget owner survives retirement, so late
release cannot credit a replacement using the same numeric ASID. Creation and
growth reject retirement before publication or allocation.

Validation:

- The host runner passed, including atomic multidimensional rejection,
  overflow, underflow and reuse. `charlotte-lifecycle` now runs 15 tests.
- Synchronous guest tests passed for record and queue ceilings, metadata-charge
  rollback when queue admission fails, failed growth preserving queued data,
  successful growth and retained charges after shrink, delegated closed
  records, internal unobserved-result cleanup, retirement rejection, forced
  ASID reuse and exact counter reconciliation. Reservation-only tests filled
  each ordinary and total node dimension independently and checked platform
  reserve and rollback; they did not allocate the full node-sized queues.
- Two isolated four-LP AArch64/TCG security runs passed **19 tests, 0 failed,
  0 pending**. Both scoped launches in each run passed all thirteen bits
  (`0x1fff`), including owned endpoint exhaustion, scalar IPC while full,
  batch-drop recovery and 128 additional create/drop cycles. The final run
  retired concurrent cancellation traffic after 4,472 requests; the first
  run had 3,780 requests.
- Signed AArch64 and x86-64 service bundles built. Both architectures' kernel
  and service Clippy passed with `-D warnings`; formatting and diff checks
  passed. No x86-64 guest execution, allocator-failure injection, sustained
  hostile-pressure soak or PDF rebuild is claimed.

The final capture's kernel SHA-256 was
`ab647c8656192a09e590e51cac6e2a54d840366ffe1bbb2582c7c4b766b60b32`.
Temporary evidence is in `/private/tmp/charlotte-security-endpoints-final-run.log`,
`/private/tmp/charlotte-security-endpoints-final-20261004-serial.log`,
`/private/tmp/charlotte-security-endpoints-host-tests.log`, and the
`/private/tmp/charlotte-security-endpoints-*-clippy.log`/`*-services.log` files.
These runs used dedicated storage and unused forwarded ports; existing soak
workloads and stores were not modified.

The [endpoint reference](../../reference/endpoint-budgets.md) documents policy,
ownership, resize peaks and the current zero-on-failure syscall limitation.
These bounds do not cover the complete capability namespace, connections,
pending calls, reply tokens, attachment vectors, observer lists, general
completion records, all other timers or loader/page-table/heap accounting.
Registry/capability metadata allocation remains infallible, and there are no
typed deployment overrides or userspace admission counters yet. SEC-07 remains
partially implemented; the audit's operational restrictions still apply.

## Follow-up: retained completion records — 2026-10-04

Endpoint/queue admission was committed as `1532ae47`. This continuation adds
a separate record budget to all four completion submission paths: ordinary
capability-backed operations, timers, detached operations and detached timers.
Thread-exit and endpoint-close watches inherit ordinary submission admission.
A namespace can retain at most its configured completion capacity, clamped
to 1,024 records. Node admission is 8,192 records, with an ordinary-domain
pool of 6,144. Generation-qualified kernel designation controls the platform
reserve; the service loader still configures 16 submission slots.

A completion object's charge survives capability close while a waiter or
captured callback holds a strong reference. A detached operation transfers
its charge into the CQ backlog when its result cannot enter the ring. Delivery
or discard returns both its record admission and submission slot. Namespace
replacement and retirement retain the old budget owner while old objects
survive. Retirement now fences all four submission paths, and failed later
timer-event admission rolls back the staged record charge.

Review found a related CQ replacement defect: discarded detached backlog
entries did not return their submission slots. Both heap- and physical-CQ
installation paths now use one replacement helper that reconciles these
slots. Replacement remains kernel-controlled teardown, not result migration.
Completion close also rechecks the exact captured object under the registry
lock before revoking a handle, preventing stale close from deleting a
replacement with the same numeric ASID/capability.

Validation:

- The full host runner passed; existing checked-counter rejection, overflow,
  release and reuse coverage supports this budget's shared arithmetic.
- Synchronous guest tests passed for retained strong objects, record recovery,
  submission rollback, non-timer cancellation, mixed record types, retained
  detached results and delivery, missing CQ rejection, CQ replacement and
  teardown, timer-event admission rollback, ordinary and total pool rejection,
  actual reserved-pool submission after kernel platform designation, retirement
  rejection across all four paths, and forced ASID/capability reuse with a
  captured old close. Pool saturation uses reservations, not allocation of the
  maximum corresponding record footprint.
- Two isolated four-LP AArch64/TCG security runs passed **19 tests, 0 failed,
  0 pending**. Both scoped launches passed all fourteen bits (`0x3fff`), now
  including non-timer close-watch exhaustion, refused timer submission while
  full, successful scalar IPC, endpoint-triggered completion of every watch
  and short-timer recovery. Concurrent cancellation traffic retired after
  4,520 requests in the first run and 4,516 in the final run.
- Both signed service bundles built. AArch64 and x86-64 kernel and service
  Clippy passed with `-D warnings`, along with formatting and diff checks.
  No x86-64 guest execution, allocator-failure injection, sustained hostile
  pressure test or PDF rebuild is claimed.

The final capture's kernel SHA-256 was
`cdf9124544617344685f79841ecec3d72aba022ec16c36907268e01f10cd4ebb`.
Temporary evidence is in `/private/tmp/charlotte-security-records-final-run.log`,
`/private/tmp/charlotte-security-records-final-20261004-serial.log`,
`/private/tmp/charlotte-security-records-host-tests.log`, and the
`/private/tmp/charlotte-security-records-*-clippy.log`/`*-services.log` files.
An unused test import was removed after the captures to pass both architectures'
strict lint checks; this did not change the test logic. Dedicated storage and
ports were used; existing soak guests and stores were not modified.

The [record reference](../../reference/completion-record-budgets.md) distinguishes
submission slots, strong object lifetimes, detached-result retention and CQ
backing. These counts do not charge weak-only Arc/control-block storage: weak
references can retain allocation memory after strong-object fields and the
record charge drop. Observer lists/cancellation, CQ backing, workers, registry
nodes and fallible metadata allocation therefore remain important SEC-07 gaps,
alongside the complete capability namespace and loader/heap/page-table budgets.
Typed deployment overrides, per-principal cross-domain totals and observable
admission counters remain future work. SEC-07 stays partially implemented and
the audit's operational restrictions still apply.

## Follow-up: completion-queue admission and loader rollback — 2026-10-04

Retained completion-record admission was committed as `8aa63b8b`. This
continuation bounds registered CQs and their kernel-owned backing independently:
32 queues/256 KiB per namespace, 2,048 queues/4 MiB per node, and an ordinary
share of 1,536 queues/3 MiB. The remaining pool is available to
generation-qualified, kernel-designated platform domains. Backlog allocation
uses checked rounding from a submission capacity clamped to 1,024. Heap rings
now have explicitly aligned `u64` backing rather than relying on the incidental
alignment of `Vec<u8>` allocations.

CQ setup reserves admission and allocates fallibly before publication. Failed
replacement leaves the original queue, capability table and pending data intact;
successful replacement must fit peak old-plus-new backing and retains the
existing discard semantics. Physical-frame alias rejection prevents one queue
from resetting another's ring. Physical-ring initialization follows all
fallible admission/allocation checks. Physical pages themselves remain owned
by address-space mappings, not charged as CQ heap bytes; old mapped frames can
outlive a replaced registration.

The loader now returns typed CQ preparation errors and owns partial preparation
in one rollback guard. If a later shard CQ fails, it closes installed queues and
tears down the mapped domain. Trusted ambient supervisor loaders designate
platform domains before CQ admission, allowing preparation to use the reserve.
Scoped and syscall preparation remains ordinary; artifact names, roles and
manifest fields do not choose reserve access. The mandatory boot wrapper still
panics on failure, and general ELF/runtime-page allocations remain infallible.

Validation:

- The full host runner passed, including new zero/small/power-of-two/overflow
  tests for checked backlog sizing.
- Synchronous guest tests passed for namespace count and backing-byte limits,
  ordinary and total node exhaustion in each dimension, rollback and recovery,
  invalid capacity, failed queue/namespace replacement preserving data and
  capabilities, aligned backing, physical-frame failures without writes, frame
  alias rejection and same-queue reuse, retirement and numeric ASID reuse.
  A real signed bootstrap image failed after partially installing its five
  CQs with only two ordinary registration slots available. CQ charges returned,
  its ASID was reusable, and trusted platform preparation then succeeded.
  Node saturation uses reservations, not the corresponding maximum allocation.
- The isolated four-LP AArch64/TCG security run passed **19 tests, 0 failed,
  0 pending**. Both scoped launches passed the unchanged fourteen-bit mask
  `0x3fff`; concurrent cancellation traffic retired after 4,500 requests.
- Both signed service bundles built; AArch64 and x86-64 kernel/service Clippy
  passed with `-D warnings`. Formatting and diff checks passed. No x86-64
  guest execution, allocator-failure injection, exhaustive frame-leak test,
  sustained hostile-pressure soak or PDF rebuild is claimed.

The capture's kernel SHA-256 was
`8947d1fb74d82b9955fdf923036827089a729134367f1494232d247cd66cfffb`.
Temporary evidence is in `/private/tmp/charlotte-security-cq-run.log`,
`/private/tmp/charlotte-security-cq-20261004-serial.log`,
`/private/tmp/charlotte-security-cq-host-tests.log` and the
`/private/tmp/charlotte-security-cq-*-clippy.log`/`*-services.log` files.
Dedicated storage and ports were used; existing soak guests/stores were not
modified.

The [CQ reference](../../reference/completion-queue-budgets.md) describes
ownership, replacement peaks, loader rollback and excluded mapped pages.
Observer registrations need cancellable owning handles and bounded storage;
their teardown must avoid entering IPC from a completion destructor under the
completion registry, because IPC already calls into completion handling.
Weak-only Arc allocations, registry/capability metadata, other timers,
worker stacks and general loader/page-table/heap admission remain open, as do
typed deployment overrides, cross-domain principal totals and application
budget counters. SEC-07 remains partial and the deployment restrictions remain.

## Follow-up: owning endpoint-close registrations — 2026-10-04

CQ admission and loader rollback were committed as `5206d8a1`. This continuation
replaces the unbounded lifecycle-watch queue on each endpoint with an owning,
fallible one-shot registration list. It does not change readiness notifications
or the scheduler's existing waiter registration contract.

Each endpoint accepts 128 watches across callers. The submitting completion
namespace is charged up to its configured capacity, clamped to 1,024 entries;
node admission is 8,192 with an ordinary share of 6,144. The submitting
generation's designation determines reserve access, not the source endpoint's
owner. An individually allocated entry owns its charge; no spare vector backing
remains after unlink. The charge is outside the Box, so entry storage and its
weak reference are released before admission returns. List, callback and entry
allocation are fallible through the kernel's newly enabled nightly allocator
API. Complete submission/capability allocation remains infallible elsewhere.

One owner stages the captured completion and registration charge. Failed
connection revalidation, missing endpoint, list rejection or allocation failure
rolls back its unpublished capability with an exact-object identity check.
Cancellation removes the registration and posts a terminal cancelled result
without awaiting source endpoint death. Completion and namespace teardown also
unlink it, including when another kernel owner retains the completion object.
The retained object's separate record charge remains until its strong references
are released. Already-closed endpoint watches finish without retaining an entry.

Token Drop enters only the independent list lock, never IPC or completion
registries. IPC detaches the notification batch without allocation and invokes
callbacks after releasing its registry. Detached entries remain charged until
the batch releases them, even when their tokens are cancelled first. A late
captured callback cannot replace a cancelled terminal result or complete a
different object with reused numeric identifiers. Entry removal is bounded by
128; list/batch destruction is iterative to avoid recursive kernel-stack use.

The [registration reference](../../reference/close-watch-budgets.md) and
[ownership guide](../../guides/resource-ownership.md) explain the developer
contract: ordinary owned watch Drop no longer needs to close the endpoint first.
The scoped probe adds a fifteenth result bit (`0x7fff`), testing a dropped watch
batch and 128 additional watch/drop cycles on a live endpoint, followed by a
normal endpoint-close result and short-timer recovery.

Other observer paths remain open. Completion/CQ waiters, pending-call and
endpoint-readiness waiters, thread-exit observers and watchdogs need registration
failure integrated with scheduler rollback; merely bounding their existing
queues and ignoring failed insertion can strand a parked thread. Their
cancellation, general weak-only Arc/control-block storage, registry metadata,
complete capability admission, other timers and loader/page-table/heap budgets
are still incomplete. No hostile-production containment, per-service progress
guarantee or per-caller endpoint fairness claim is made; SEC-07 remains partial.

Validation:

- The full host test runner passed, including the existing checked-counter
  overflow/rejection/release coverage used by registration admission.
- Synchronous guest tests passed for endpoint/domain/ordinary/node ceilings,
  rollback of a rejected staged completion, 512 cancel/rearm cycles with the
  source still live, subsequent ordinary IPC, normal and already-closed
  notification, detached-entry retention and discard, callback reentrancy,
  a deliberately retained late callback after cancellation, retired admission,
  repeated client teardown while retaining the old completion object, and
  exact ASID/capability reuse with a stale unpublished owner.
- Three isolated four-LP AArch64/TCG security runs passed **19 tests, 0 failed,
  0 pending**, including all fifteen bits (`0x7fff`) in both scoped launches.
  The final run included the retained-object teardown and late-callback tests
  plus the entry layout that frees backing before returning admission.
  Concurrent cancellation traffic retired after 4,512 requests in that run.
- Both signed service bundles built, and AArch64/x86-64 kernel and service
  Clippy passed with `-D warnings`. Formatting and diff checks passed.
  No x86-64 guest execution, allocator-failure injection, exhaustive cross-LP
  race exploration, sustained hostile-pressure soak or PDF rebuild is claimed.

The final capture's kernel SHA-256 was
`2160e7ace57bdbeac1ab8514d5a839898ea0c493a2b33169ddf7af702413d942`.
Temporary evidence is in `/private/tmp/charlotte-security-watches-storage-run.log`,
`/private/tmp/charlotte-security-watches-storage-20261004-serial.log`,
`/private/tmp/charlotte-security-watches-host-tests.log`, and the
`/private/tmp/charlotte-security-watches-*-clippy.log`/`*-services.log` files.
Dedicated storage and ports were used; existing soak guests/stores were not
modified. Reservation-only pool saturation does not allocate the equivalent
maximum registration footprint.

## 2026-10-04 continuation — owned completion/CQ scheduler waiters

The preceding close-watch pass is committed as `1b1dd862`. This continuation
migrates completion/CQ scheduler waiters onto the independent owning list and
supersedes their remaining-work entry above; it does not migrate every observer.

Each source admits 64 live entries. The waiting domain generation admits 1,024,
node admission is 8,192 and the ordinary share is 6,144. Thread construction
captures the original sponsor; retirement rejects new reservations, and
retained entries cannot credit a replacement with the same numeric ASID.
Promotion changes future charges, not the classification of existing ones.
Entry/Waker/completion allocation is fallible on the migrated path; complete
allocator and capability-table admission is still absent.

The scheduler registers before removing a Ready thread or publishing Blocked.
Admission failure therefore preserves queue membership, thread state and
migration constraints. An already-ready source returns without parking and
without inline callbacks under the master table. The Waker owns the token;
Ready admission explicitly cancels it even if another strong Waker survives.
Aborted Blocked threads cancel on actual reaping. Detached batches retain
charges until freed, and source callbacks run outside source/list locks. CQ
notifications drain a reusable list without vector allocation or stale-observer
pruning. Source replacement/destruction still requires quiescent consumers.

Timed completion syscalls no longer panic on park rejection. They return the
new `WAIT_ADMISSION_FAILED` status, and the runtime retains ownership for retry
or cancel/terminal-wait/close on Drop. Untimed completion waiting instead retries
cooperatively until the producer is terminal, including after unrelated wakes;
returning early would let a `ReadOperation` release borrowed storage unsafely.
This fallback protects lifetime, not throughput or progress under overload.
CQ waits keep their old ABI shape and can return without parking on rejection.

The [waiter reference](../../reference/scheduler-waiter-budgets.md), ownership
guide, scheduler invariants, locking table, README and LaTeX sources document
these contracts. Legacy timers, locks, pending-call/endpoint-readiness sources,
raw completion callbacks, thread-exit observers and watchdog storage remain
outside these budgets. Their admission/cancellation, general weak-only backing,
registry and capability metadata, and loader/page-table/heap accounting are
still open. SEC-07 remains partial.

Validation:

- The full host runner passed, including two new owner tests for timed wait
  rejection followed by retry or Drop.
- Synchronous guest tests passed for source/domain/ordinary/node rejection and
  rollback, owned unlink, detached retention/discard, rearming, callback
  reentrancy, future-charge promotion, retirement and exact ASID reuse.
  Node/ordinary pool saturation is counter-only, not maximum-footprint allocation.
- Two completed isolated four-LP AArch64/TCG security runs passed **19 tests,
  0 failed, 0 pending** and both scoped launches' fifteen-bit mask (`0x7fff`).
  The final run also checks never-dispatched Blocked-thread reaping with a
  retained Waker, as well as non-mutating Running/Ready/new rejection, owning
  wake cancellation and 64 completion/CQ timeout cleanup cycles. The existing
  CQ completion, explicit wake, second-queue wake and endpoint-readiness tests
  still pass. Concurrent cancellation traffic retired after 4,492 requests.
- Both service bundles built; AArch64/x86-64 kernel and service Clippy passed
  with `-D warnings`. Formatting and diff checks passed. There was no x86-64
  guest execution, allocator-failure injection, forced real-EL0 timed-wait
  rejection, exhaustive cross-LP exploration, hostile-pressure soak or PDF rebuild.

The final guest capture's kernel SHA-256 is
`1750625ee069912969581ba92144e96467e78c119a81cc419345b2e73d6b1631`.
Evidence is in `/private/tmp/charlotte-security-waiters-reap-final-run.log`,
`/private/tmp/charlotte-security-waiters-reap-final-20261004-serial.log`,
`/private/tmp/charlotte-security-waiters-host-tests.log`, and the
`/private/tmp/charlotte-security-waiters-*-clippy.log`/`*-services.log` files.
Dedicated storage and forwarded ports were used; existing soak guests/stores
were not modified. One attempted follow-up stopped at compilation on a test
helper import before booting, then was corrected and rerun successfully.

## 2026-10-04 continuation — owning IPC receive/reply waiters

The preceding completion/CQ waiter changes are committed as `cf0e98a5`.
Endpoint-readiness and pending-call scheduler waiters now use the same owning
registration contract and share its waiting-generation/node pools. Each source
links at most 64 entries. Detached batches still occupy the domain/node pools,
not the reusable source's linked count. Readiness drains/rearms on messages and
closes on endpoint death; a pending call closes its list on reply or cancellation. Source-ready
checks and insertion share IPC's registry lock. Missing sources reject rather
than silently losing a wake.

Notifications detach under IPC and run after its registry is released. Endpoint
closure combines receiver and queued-call batches by splicing owning entries,
without allocating a callback vector. Detached storage retains its charge until
actually released. Source destruction discards residual entries locally even
if a kernel token retains the source list. Ready admission and reaping use the
already implemented Waker cancellation, including when a watchdog wins.

Endpoint creation prepares the readiness list fallibly before publication.
All six call-submission variants stage their pending-call list before memory
move/copy/lend/vector transfer or delegated connection attachments. Failure at
this new stage therefore leaves those inputs untouched. Later registry and
capability insertion and other allocation paths remain incompletely fallible.
Untimed receive/reply waits retry cooperatively on admission rejection instead
of parking without a wake or treating a live call as completed. A real loan
remains delegated until reply or explicit cancellation revokes it. Kernel timed
reply waiting can return false without consuming the call. No IPC syscall status
or userspace ownership API was added. Two debugger-visible counters record
receive/reply admission retries without granting authority or driving policy.

Repetition caught an additional timed-wait setup defect: one run remained
pending on CQ wait after the other 18 tests passed. Source inspection found a
quantum-preemption window between publishing Blocked and queuing its watchdog.
The shared condition-wait helper, timed CQ wait and timed completion syscall
now use a non-Send local interrupt-mask owner across park/watchdog enqueue/
recheck. Rejection restores the entry IRQ state automatically, and the mask is
dropped before yielding. The condition must be a short non-parking check. This
closes the identified window; the stalled run had no live debugger capture, so
it is not a complete causal trace or exhaustive scheduling proof.

The waiter reference, locking/state-machine rules, ownership/testing guides,
README, endpoint/record references and LaTeX sources were updated. The manual's
memory-loan description now makes clear that a wait timeout alone does not end
the call or revoke the loan. Connection, pending-call and reply-token records,
source-list control blocks, general registry/weak-only metadata, legacy lock/
timer/raw callback registrations, watchdog storage and loader/page-table/heap
admission remain open. SEC-07 stays partial; no hostile-production containment
or per-service progress guarantee is claimed.

Validation:

- The full host test runner passed. Both service bundles built, and AArch64/
  x86-64 kernel and service Clippy passed with `-D warnings`.
- Synchronous guest tests passed for both source ceilings and rollback,
  512 receiver register/drop/rearm cycles, message/reply/closed-source fast
  paths, callback reentrancy, reply-token Drop, pending-call close with loan
  revocation, receiver plus multiple queued-call notification batches, retired
  sponsorship and replacement-ASID accounting.
- Scheduled tests check 64 reply/readiness timeout cleanup cycles, full-source
  rejection with unchanged Running state/constraints, nested IRQ restoration
  and rejection-path restoration, and forced untimed receive/reply recovery.
  Helpers wait for an admission-retry counter before producing. The reply
  helper sees a real delegated read loan before reply; the caller verifies it
  is revoked when the untimed wait returns. These are kernel fixtures, not
  forced-quota real-EL0 syscall tests. The ordinary real-EL0 IPC memory/receive
  tests and both scoped security launches still pass.
- The first attempt failed in a new test fixture because a synthetic IPC
  namespace was used as a loan recipient without an address space. That
  fixture was corrected to use real sender/receiver address spaces. A later
  repetition exposed the timed-wait setup gap described above. Those failed
  attempts are not counted as successful validation.
- Three subsequent isolated four-LP AArch64/TCG runs passed **19 tests,
  0 failed, 0 pending**, including both scoped launches' fifteen-bit mask
  (`0x7fff`). The final run retired concurrent cancellation traffic after
  4,492 requests. The first post-fix run also captured a steady-state debugger
  snapshot with all four LPs in their idle loops and interrupts unmasked;
  that snapshot does not establish the cause of the earlier stall.
- Workspace formatting and diff checks passed.

No allocator-failure injection, exhaustive cross-LP interleaving, x86-64 guest,
hostile-pressure soak or PDF rebuild was performed. Dedicated storage and ports
were used; existing soak guests/stores were not modified.

The final guest capture's kernel SHA-256 is
`a18d20679e8ac309c9b6189c1a289829b3847ccda19e6455c481285800db6596`.
Evidence is in `/private/tmp/charlotte-security-ipc-waiters-repeat2-run.log`,
`/private/tmp/charlotte-security-ipc-waiters-repeat2-20261004-serial.log`,
`/private/tmp/charlotte-security-ipc-waiters-atomic-run.log`,
`/private/tmp/charlotte-security-ipc-waiters-repeat1-run.log`,
`/private/tmp/charlotte-security-ipc-waiters-atomic-20261004-debug-snapshot-lldb.log`,
`/private/tmp/charlotte-security-ipc-waiters-host-tests.log`, and the
`/private/tmp/charlotte-security-ipc-waiters-*-clippy.log`/`*-services.log` files.

## 2026-10-04 continuation — IPC connection/call/reply record admission

The preceding IPC waiter and timed-wait changes are committed as `9741700c`.
This continuation separately admits connection capabilities, retained pending
calls and outstanding reply tokens. Each dimension has a 512-record sponsoring
namespace ceiling, an 8,192-record node ceiling and a 6,144-record ordinary
share. Kernel-designated platform generations can use the shared reserve;
names, supplied roles and descriptor fields cannot obtain it.

Grantors sponsor direct connections and re-delegations. Callers sponsor pending
calls and the server's outstanding reply token together, and pay for connections
returned in their solicited replies. Repeated lookups therefore cannot consume
the serving grantor's connection allowance; unsolicited grants cannot spend
their recipient's allowance.
Connections from several grantors can accumulate at one recipient: the domain
limit is sponsorship, not a per-recipient capability-table ceiling. None of this
changes who may use or close a capability. Completed/observed call records stay
charged until closed; reply-token charges return on reply or cancellation.
Connection charges are owned inside their stored capability record, not in a
parallel bookkeeping table, so internal removals release them too.

Every call path atomically reserves its two records and prepares its fallible
waiter list before transferring attachments. Connection-bearing calls reserve
their connection before copying memory. Connection-bearing replies reserve
before consuming the token, moving memory or revoking a loan. Staged owners
release reservations on error. Kernel quota rejection leaves the token/loan
live; current consuming Rust `ReplyToken::reply_connection*` methods instead
close/cancel the token on error, retaining the borrowed grant source. The new
reference and ownership guide document that distinction rather than promise a
retryable token that the current API does not return.

Review also found a teardown-publication gap: a receive or delegated/returned
connection could create a capability after teardown collected its drain list.
IPC now marks its record account retiring before that snapshot. Receive and
connection-publication paths reject retiring or generation-mismatched namespaces
under IPC, before dequeue or transfer. Captured charges keep their old account
alive after namespace removal. A late remote connection close cannot credit a
replacement with the same ASID.

The [record reference](../../reference/ipc-record-budgets.md), endpoint/waiter
references, locking rules, ownership/testing guides, README and LaTeX sources
document sponsorship, lifetime and error semantics. Registry/capability insertion,
namespace-account allocation, general weak-only retention, remaining legacy
observers/watchdogs, attachment bytes and comprehensive loader/page-table/heap
accounting remain incomplete. There are no signed record-limit overrides or
per-principal aggregates across domains. SEC-07 remains partial and the audit's
deployment restrictions still apply.

Validation:

- The full host runner passed, including the new owned reply test for
  `RESOURCE_LIMIT` cancelling its token without closing its borrowed grant source.
- Synchronous kernel tests passed for actual 512-connection, retained completed-call
  and outstanding-call ceilings; reuse after closure; every call attachment
  variant's rejection before transfer; independent call/reply dimension rejection;
  staged attachment-failure rollback; rejected connection reply with a live loan;
  successful reply revocation; observed/unobserved result cleanup; queued
  connection cancellation; retirement before scalar/vector dequeue or delegated
  publication; and original-generation accounting after forced ASID reuse.
  Ordinary and total node saturation tests are counter-only, not maximum-footprint
  registry allocation.
- The final requester-funded implementation passed two isolated four-LP
  AArch64/TCG runs: **19 tests, 0 failed, 0 pending**, with both scoped launches'
  fifteen-bit mask (`0x7fff`). The EL0 probe now fills connection admission and
  submits 512 outstanding calls to a larger queue using owned batches, then
  verifies cancellation and recovery. The final run retired concurrent
  cancellation traffic after 4,528 requests. Three earlier runs also passed
  19/19 while the retirement fence, stored-entry ownership and sponsorship
  policy were being refined; they are not substitutes for the final-policy runs.
- Both service bundles built. AArch64/x86-64 kernel and service Clippy passed
  with `-D warnings`; workspace formatting and diff checks passed.

The final guest kernel SHA-256 is
`c79ee2a69aa189e5b3a743de2b169cad641961d499ba6f69d1e2fcf7e241fc92`.
Evidence is in `/private/tmp/charlotte-security-ipc-records-final-run.log`,
`/private/tmp/charlotte-security-ipc-records-final-20261004-serial.log`,
`/private/tmp/charlotte-security-ipc-records-requester-run.log`,
`/private/tmp/charlotte-security-ipc-records-host-tests.log`, and the
`/private/tmp/charlotte-security-ipc-records-*-clippy.log`/`*-services.log` files.
No allocator-failure injection, exhaustive cross-LP teardown race exploration,
x86-64 guest execution, hostile-pressure soak or PDF rebuild was performed.
Dedicated storage and forwarded ports were used; existing soak guests/stores
were not modified. These count limits do not guarantee many-client fairness,
per-service progress, or complete kernel metadata containment.

## 2026-10-04 continuation — owned kernel blocking-lock waiters

The preceding IPC record-admission changes are committed as `9319e28e`.
Kernel scheduler-blocking mutex and read/write locks now use owning waiter
registrations instead of unbounded weak-observer queues. Each mutex has one
64-entry linked source; RwLock reader and writer sources each have 64. Entries
share the existing generation-scoped 1,024-domain / 8,192-node waiter pools and
6,144-entry ordinary share with completion, CQ and IPC waiters. This does not
add another independent allowance or let a caller claim platform reserves.

The const-initializable source allocates its list fallibly on first contention.
Uncontended acquisition allocates nothing. Entry admission follows after the
initialization guard is released, and source destruction discards entries even
when tokens retain the list. Detached notification batches retain their charges
until released. The source's list control block remains until destruction and
is not charged to the entry count; general weak-only/control-block metadata
containment is still incomplete.

Admission rejection leaves the thread runnable to yield and retry acquisition;
it cannot return as if it owned the data lock. No source, initialization or
scheduler guard crosses yield. Unlock releases ownership and detaches candidate
batches before callbacks. RwLock detaches both classes before notifying either,
and only the final shared owner triggers notification. An expired writer cannot
suppress waiting readers. These bounded broadcasts are wake hints, not reserved
handoffs: CAS still selects ownership. FIFO order, writer priority, starvation
freedom, priority inheritance and owner-death recovery are not provided.

Review found a preemption window between publishing Blocked and rechecking the
raw lock state. If unlock preceded insertion, a quantum could switch out the
waiter before that recheck with no later unlock to wake it. All three acquisition
paths now hold the existing non-Send local interrupt-mask owner across parking
and recheck, restoring the entry IRQ state before yield. This was identified
from source inspection, not an observed guest stall or an exhaustive scheduling
proof. Rejection also releases the scheduler read guard before yielding.

The blocking family has kernel fixture users but no production callers.
Registry/allocator interrupt-masking spin locks are unchanged; no lock-throughput
improvement is claimed. The README, waiter/locking references, testing guide and
LaTeX sources document this distinction. SEC-07 remains partial: legacy timer,
thread-exit and raw callback observers, watchdog storage, general registry and
weak-only metadata, and comprehensive loader/page-table/heap accounting remain
open. The audit's deployment restrictions continue to apply.

Validation:

- The full host test runner passed. Both service bundles built; AArch64/x86-64
  kernel and service Clippy passed with `-D warnings`.
- Synchronous fixtures passed source ceilings and rejection rollback, a free
  mutex's ready fast path, 512 cancel/rearm cycles, callback reentrancy, expired
  writer plus live reader notification, final-reader release, detached-batch
  accounting, retired sponsorship and source destruction with retained tokens.
- Scheduled fixtures passed 64 timed source-wait cleanup cycles, non-mutating
  admission rejection, forced mutex/reader/writer retry counters, remote-LP
  contention and final-reader wake. Data holders never explicitly yield or park
  with their guard. Timed tests exercise the shared condition-wait helper, not
  a newly introduced timed-lock API. No userspace ABI or probe-mask bit changed.
- The final park/recheck-guard version passed two isolated four-LP AArch64/TCG
  runs: **19 tests, 0 failed, 0 pending**, including both scoped launches'
  fifteen-bit mask (`0x7fff`). The final run retired concurrent cancellation
  traffic after 4,508 requests.
  Two earlier pre-guard runs also passed 19/19; they do not validate the final
  preemption fix.
- Workspace formatting and diff checks passed.

The final post-guard guest kernel SHA-256 is
`15edc69dc87a09544280757e9153eb316c486b33874cf498f07f72784162793a`.
Evidence is in `/private/tmp/charlotte-security-lock-waiters-final-run.log`,
`/private/tmp/charlotte-security-lock-waiters-final-20261004-serial.log`,
`/private/tmp/charlotte-security-lock-waiters-guard-run.log`,
`/private/tmp/charlotte-security-lock-waiters-guard-20261004-serial.log`,
`/private/tmp/charlotte-security-lock-waiters-host-tests.log`, and the
`/private/tmp/charlotte-security-lock-waiters-*-clippy.log`/`*-services.log` files.
No allocator-failure injection, forced quantum at the identified window,
exhaustive cross-LP interleaving, x86-64 guest, hostile-pressure soak or PDF rebuild
was performed. Dedicated storage and ports were used; existing soak guests and
stores were not modified. Entry bounds do not establish fairness, per-service
progress or complete kernel metadata containment.
