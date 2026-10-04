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
| SEC-07 | Partially implemented | Memory-object backing pages and counts have generation-scoped sponsorship budgets, RAM-derived node admission and platform/physical progress reserves. Completion-backed timer events have domain/node admission, reserved platform progress and charges retained through deferred cancellation. Aggregate limits for loader/heap/page-table memory, the complete capability namespace, endpoints/queues, general completion records, other timer paths and kernel metadata remain open. |
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
3. Extend admission to the remaining capability, endpoint/queue, general
   completion, other timer and loader/heap/page-table budgets. Add
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
