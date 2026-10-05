# Security remediation — 2026-10-03

This records implementation passes following the
[security audit](2026-10-03-security-audit.md) of revision
`42183c57ce4c0b32a6010246f6eee1b6262ebb4e`. It is not a declaration that the
audit is closed or that CharlotteOS is ready for hostile production workloads.
Implementation and validation span 2026-10-03–05 local time.

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
| SEC-07 | Partially implemented | Memory-object backing pages/counts, anonymous timer events (completion plus sleep/watchdog), endpoint records/queue backing, retained completion objects/detached results, CQ registrations/kernel backing, endpoint-close/thread-exit/kernel-callback registrations, completion/CQ/IPC/lock/timer/boot-status scheduler waiters and connection/pending-call/reply-token record counts have generation-scoped domain/node admission and platform reserves. Lifecycle watches and kernel callbacks share one account and node pool; worker exit registration precedes execution and retains deferred producer cancellation. Callback registration checks exact operation identity; late watches are fenced against namespace replacement. Every scheduler Observable requires owning registration, without a weak-only fallback. Timer families have separate domain accounts and one shared node pool. Charges survive transfers, deferred cancellation, delegation, retained references or detached notifications as appropriate. Waiter admission precedes parking; owning cancellation handles competing wakes and reaping. Watchdog callback/cancellation/node preparation precedes Blocked; queue insertion allocates nothing and the quantum has independent inline storage. Timed completion admission failure retains its owner; untimed completion/IPC waits preserve borrowed-buffer safety. Sleep rejection waits runnable to the requested deadline; internal timer callbacks have one embedded slot. Timed park/watchdog setup is non-preemptible. IPC call/reply preparation precedes attachment transfer, and retirement fences receive/connection publication before teardown. Aggregate limits for loader/heap/page tables (including physical CQ mappings), the complete capability namespace, arbitrary callback captures, general weak-only/control-block storage and comprehensive kernel metadata remain open. |
| SEC-08 | Open | Authenticate enrolled nodes and control/data peer traffic, add replay protection, and bound discovery state. A trusted L2 segment remains an explicit deployment prerequisite. |
| SEC-09 | Open | Distinguish authenticated security time from observational SNTP/holdover; enforce freshness and uncertainty at security-policy gates. |
| SEC-10 | Open | Authenticated encrypted access to node and cluster management, browser-client provisioning, and access policy remain necessary. |
| SEC-11 | Implemented for scoped applications | Scoped application mapping uses the configured artifact key; grantctl relies on launcher attestation under the configured deployment key. Independent roots are tested through real scoped launch and grant IPC. The complete S3/Raft release pipeline with those roots remains to be tested. Bundled platform-service trust and production provisioning still belong to SEC-04. |
| SEC-12 | Open | Attenuate local object-store authority to object sets/namespaces; retain an explicitly separate administration endpoint. |
| SEC-13 | Implemented in this repository | Signing/decryption commands enforce restricted bounded key files; generation never prints private material; build/deployment/shutdown scripts pass paths and reject the old secret-valued environment variable without expanding it. Only the exact public artifact fixture retains warned argv compatibility. Sibling broker/Durga callers still need file-path migration; host custody, ACL review and agent/HSM signing remain separate work. |
| SEC-14 | Mitigated; audit corrected | Cargo.lock is already tracked. Main build/test runners and CI now enforce --locked; CI actions are commit-pinned, token permissions are read-only, and checkout does not persist credentials. Advisory/license scans and a release dependency inventory remain. |
| SEC-15 | Mitigated | SigV4 prefixed secret, derived keys, HMAC block/pads and inner digest use zeroizing owners. TLS record buffers are wiped after dropping their borrower, including handshake failure. This is not a complete audit of crypto-library state or compiler-created secret copies. |
| SEC-16 | Implemented | grantctl polls bounded concurrent operations with per-sender/generation limits and total deadlines. Non-parking authorized lookup avoids a shared name-service waitlist leak. Acquisition retries and publication waits have total deadlines. A two-application cancellation stress and silent-endpoint publication timeout pass in the guest; many-client fairness and controller-replacement testing remain. |

SEC-07 also includes fixed per-route IRQ readiness storage: repeated or retired
deliveries cannot exhaust a shared wake queue, and deferred route validation/CQ
publication is lifecycle-fenced. This does not close aggregate kernel metadata
accounting or establish interrupt-controller/scheduler progress guarantees.
Mailbox handle records now also have generation-scoped admission, owning
refund and a trusted platform reserve; this is not aggregate namespace or
mailbox queue-backing admission.

Shared namespace accounting now includes all capability kinds and enforces
staged admission for mailbox, completion and memory publication. Exact namespace
tokens fence cancellation/publication; owning prepared memory transfers integrate
source escrow/backing retention or private-copy backing, with atomic mixed-mode
IPC memory publication. Scalar reverse-move and receiver-alias rollback are
removed. Reply tokens track every vector loan, and kernel buffer/DMA operations
enforce loan permissions. Other families
can still exceed shared policy through explicitly named unconverted paths.
Thus the complete capability-namespace admission requirement remains open;
this is not a backward-compatibility promise for those paths.

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

## 2026-10-04 continuation — timer scheduler observers and sleep rejection

The preceding blocking-lock waiter changes are committed as `8d9e2551`.
`TimerEvent` no longer allocates an unbounded weak-observer queue. Scheduler
registrations use the shared owning waiter source, with a 64-entry linked
ceiling and the existing generation-scoped domain/node admission. Sleep's Waker
owns its token; wake, thread reaping or event destruction release the entry.
Detached entries retain their charge until the notification batch releases them.
Cancelled events suppress delivery without falsely crediting retained storage.

Every current non-scheduler timer producer installs one callback: scheduler
quantum/idle wake, completion-backed timer, or timed-wait watchdog. That weak
reference now occupies one embedded slot. It does not allocate a callback list
or consume scheduler waiter-entry admission. Attempting a second internal
registration is a kernel programming error, not silently dropped delivery.
This is an intentionally single-callback internal contract; thread-exit and
raw completion callback lists are unchanged. Signal detaches both forms before
calling either, releasing source guards. Timer-queue processing still holds
its LP-local borrow, so callbacks must not re-enter the queue.

Sleep previously expected parking to succeed. With fallible timer registration,
that would turn admission pressure into a kernel panic. The unchanged void sleep
ABI now discards the unqueued event on rejection, restores the entry IRQ state
and yields cooperatively while runnable until a rebased counter deadline.
It does not report success early or leave a Blocked thread without a timer.
Normal admission still rebases after parking and queues the event before IRQ
restoration. The diagnostic fallback counter grants no authority and drives no
policy. This is a safety fallback, not efficient idle waiting or guaranteed
progress under overload.

This continuation bounds observer-entry retention, **not all timer storage**.
Sleep/watchdog queue nodes and cancellation/control blocks still lack complete
event admission; queue growth and several event allocations remain infallible.
An aborted sleep can leave an empty source control block until the deadline,
although its linked waiter is cancelled. Completion-backed events keep their
existing separate event charge. General metadata, thread-exit/raw callback
registrations and loader/page-table/heap accounting remain open. SEC-07 stays
partial and the deployment restrictions continue to apply.

The README, reference/locking/state-machine documentation, testing guide and
LaTeX sources distinguish observer admission from event-storage admission.

Validation:

- The full host runner passed; AArch64 bundled services built through the guest
  runner. Strict AArch64/x86-64 kernel Clippy passed with `-D warnings`. Services
  are unchanged; both service bundles and their strict Clippy passed in the
  preceding blocking-lock pass.
- Synchronous fixtures passed source bounds/rollback, 512 register/drop/rearm
  cycles, callback reentrancy, expired internal callbacks, cancelled-event
  suppression, destruction with retained tokens and retired sponsorship.
  Source/domain/node counters reconcile at the synchronous test boundary.
- Scheduled fixtures passed 64 normal sleeps and 64 competing-watchdog timeout
  cleanup cycles. Full-source rejection leaves Running state and constraints
  unchanged. The normal sleep implementation, supplied a pre-filled timer
  source, increments its fallback counter and does not return before the
  requested interval; event destruction releases the test sponsor even with
  retained tokens. This is a kernel source-pressure fixture, not a real EL0
  domain-saturation or allocator-failure test.
- Two isolated four-LP AArch64/TCG runs passed **19 tests, 0 failed, 0 pending**,
  including the new fixtures and both scoped launches' fifteen-bit mask
  (`0x7fff`). The final run retired concurrent cancellation traffic after
  4,540 requests. No new syscall or probe-mask bit was added.
- Workspace formatting and diff checks passed.

The final guest kernel SHA-256 is
`2089d9f75002f01845749eaa1bc8fcc588493c24654ebe220b8f96e14319e472`.
Evidence is in `/private/tmp/charlotte-security-timer-waiters-run.log`,
`/private/tmp/charlotte-security-timer-waiters-20261004-serial.log`,
`/private/tmp/charlotte-security-timer-waiters-repeat-run.log`,
`/private/tmp/charlotte-security-timer-waiters-repeat-20261004-serial.log`,
`/private/tmp/charlotte-security-timer-waiters-host-tests.log`, and the
`/private/tmp/charlotte-security-timer-waiters-*-kernel-clippy.log` files.
No allocator-failure injection, maximum-footprint queue test, exhaustive cross-LP
interleaving, x86-64 guest, hostile-pressure soak or PDF rebuild was performed.
Dedicated storage and ports were used; existing soak guests/stores were untouched.

## 2026-10-04 continuation — sleep/watchdog events and prepared timer nodes

The preceding timer-observer changes are committed as `9f6ed6d8`.
All anonymous timer producers now prepare an admitted owning queue node:
capability/detached completion timers, scheduler sleeps and all three timed-wait
watchdog paths. Sleep/watchdog sponsorship has a 1,024-event per-generation
ceiling. It shares the existing 8,192-node / 6,144-ordinary event pool with
completion-backed timers, whose namespace limit remains the smaller of its
completion capacity and 1,024. These are independent domain accounts, not a
combined per-domain allowance or a fresh independent node pool.

Thread construction captures its scheduler timer sponsor from the generation
ledger. Reservation takes only domain then node counters; no memory/ASID table
lookup occurs during those counter operations. Retirement rejects new events;
promotion affects future charges only. Old events retain their original account
through teardown/reuse. Review also removed eager construction of unused budget
accounts on every existing ledger lookup by making account initialization lazy.
Initial sponsor/control-block allocation itself is still infallible.

Anonymous queue storage is now a sorted owning linked list, not a growing
`VecDeque`. Fixed-size nodes and cancellation state are allocated fallibly before
parking or record publication. Timed waits also allocate their callback owner
before parking. Insertion transfers the prepared node without allocating or
holding registry guards. Cancellation/removal and list destruction are iterative;
no high-water queue backing persists after node removal. The node/event retains
its charge while prepared, queued and notifying callbacks. Queue insertion,
purge scans and diagnostics remain linear in the number of events.

Each LP's scheduler quantum/idle wake occupies one embedded queue slot outside
anonymous admission. Its deadline participates in earliest-event selection,
without an external armed flag or a separately charged heap node. Anonymous
pressure cannot consume this scheduler storage. A cancelled prepared event is
discarded at enqueue. Shared cancellation state records the actual publishing
LP, so relocation between preparation and enqueue cannot leave the handle's
queue owner stale. Remote/busy cancellation still flags rather than waiting on
or recursively entering a queue; the charge remains until physical removal.

Failed event/callback/node preparation occurs before Blocked. Sleep waits
cooperatively while runnable to a rebased deadline. Generic timed waits return
their condition and timed CQ waits return `false`. Timed completion syscalls
return existing `WAIT_ADMISSION_FAILED` (3), leaving the capability/operation
live. Completion timer submissions return existing `SubmitError::WouldBlock`
before record publication when cancellation/node staging fails. Local IRQ masks
still span park, enqueue and lost-wake recheck, and end before yield.

The [event reference](../../reference/scheduler-timer-budgets.md), waiter and
completion-timer references, locking/state-machine descriptions,
testing guide, README and LaTeX sources document the independent lifetimes.
This is fixed event-count containment, not comprehensive heap-byte admission.
General sponsor/control-block/weak-only metadata, completion and thread-exit
callback lists, capability namespaces and loader/page-table/heap accounting
remain incomplete. Aborted sleepers can retain charged nodes until their
original deadline; the node ceiling bounds that retention but does not ensure
service progress. SEC-07 remains partial and deployment restrictions still apply.

Validation:

- The host runner passed. AArch64 services built through each guest runner;
  strict AArch64 and x86-64 kernel Clippy passed with `-D warnings`. Service
  source is unchanged; its dual-architecture build/lint evidence remains from
  the preceding passes.
- Synchronous fixtures allocate 1,024 real nodes and check ordering, capacity
  reuse, iterative filtering/destruction and the independent inline quantum.
  Ordinary/node saturation with shared completion reservations is counter-only
  at the full 8,192-event node ceiling. Promotion, retirement and exact numeric
  ASID reuse preserve original-account charges. A deterministic node-allocation
  failure checks rollback; it does not exhaust the physical allocator.
- Scheduled fixtures temporarily substitute only the executing kernel thread's
  sponsor to force event rejection. Generic/CQ waits reject before parking.
  A synthetic timed-completion syscall returns status 3 with a still-pending
  capability; the fixture explicitly cancels it before closing. Running state
  and constraints remain unchanged. Runnable sleep preserves its interval.
  Busy-local cancellation retains its charge until purge; cancellation before
  enqueue discards its prepared charge; simulated relocation updates the shared
  owner LP. Sixty-four normal sleeps plus 64 watchdog waits reconcile charges.
  These are kernel/synthetic-ABI fixtures, not real-EL0 domain saturation or
  actual remote-LP purge tests.
- The first run passed 19/19 before the synthetic-ABI fixture was added. That
  added fixture initially panicked by trying to close a pending capability
  without cancelling it (`CapError::NotComplete`). The fixture was corrected;
  the failing run is not counted as successful validation. The corrected run
  passed 19/19 before the final lazy-initialization/pre-enqueue checks.
- The final bounded implementation passed two isolated four-LP AArch64/TCG runs:
  **19 tests, 0 failed, 0 pending**, including both scoped launches' fifteen-bit
  mask (`0x7fff`). The final run retired concurrent cancellation traffic after
  4,508 requests.
- Workspace formatting and diff checks passed.

The final-policy guest kernel SHA-256 is
`139e77cf7a99038134f2ba09d683b355798ebbc4108d8614abf33a0ea4774135`.
Evidence is in `/private/tmp/charlotte-security-timer-events-repeat-run.log`,
`/private/tmp/charlotte-security-timer-events-repeat-20261004-serial.log`,
`/private/tmp/charlotte-security-timer-events-bounded-run.log`,
`/private/tmp/charlotte-security-timer-events-bounded-20261004-serial.log`,
`/private/tmp/charlotte-security-timer-events-run.log`,
`/private/tmp/charlotte-security-timer-events-final-20261004-serial.log`
(failed fixture), `/private/tmp/charlotte-security-timer-events-fixed-run.log`,
`/private/tmp/charlotte-security-timer-events-host-tests.log`, and the
`/private/tmp/charlotte-security-timer-events-*-kernel-clippy.log` files.
No physical-allocator exhaustion, exhaustive cross-LP teardown/IRQ exploration,
x86-64 guest, hostile-pressure soak or PDF rebuild was performed. Dedicated
storage and forwarded ports were used; existing soak guests/stores were untouched.

## 2026-10-04 continuation — owned thread-exit subscriptions

The preceding scheduler-event/prepared-node changes are committed as `d3ee64b5`.
This continuation replaces the target thread's unbounded weak-callback vector
with fallible one-shot registrations. Each thread links at most 128 entries.
The submitting generation sponsors its entry in the **existing** endpoint-watch
account: configured completion capacity clamped to 1,024, sharing the same
8,192-node / 6,144-ordinary pool. These are not independent allowances per event
type, and source owners do not lend callers platform privileges.

External exit watches own callback and cancellation token in their completion.
Cancellation unlinks and finishes locally without killing or joining the target.
Lookup, generation checking and insertion remain serialized with retirement;
absent/stale targets complete immediately, whereas allocation/admission failure
returns submission backpressure and rolls back the staged completion. Reaping
detaches notifications before callback invocation, outside source/list/master
table guards and before stack deallocation. Charges follow detached entries,
not retained empty tokens; teardown/destruction remain iterative.

Review also found that a worker could execute and exit before its completion's
weak callback was registered, and that the old path ignored registration errors.
Worker setup now binds the callback and its owning token to the fresh thread
before publication/admission, with no recycled-TID lookup. A worker completion
retains its subscription after cancellation until actual producer exit; it is
not treated as a locally removable join. Terminal completion or namespace
teardown releases both kinds of subscription. The latter revokes notification
ownership, rather than proving that a running worker stopped.

Installation now fences exact completion identity under the namespace guard.
This also covers endpoint-close watches: teardown/replacement before a late
installation cannot leave a registration on an obsolete completion retained by
another kernel owner. Cancellation or completion before owner installation is
handled without retaining a late token on a terminal record.

The [event-watch reference](../../reference/close-watch-budgets.md), ownership
guide, runtime API comments, scheduler-waiter reference and LaTeX completion
chapter document these contracts. The public syscall ABI is unchanged. Lazy
thread-list/control-block allocation can persist after the last entry is
removed; entry-count admission does not charge that empty metadata. Kernel
stack/thread-table construction, general weak-only Arc storage, raw completion
callback lists, capability namespaces and loader/page-table/heap accounting
remain unfinished. SEC-07 stays partial and deployment restrictions apply.

Validation:

- The host runner passed. Bundled AArch64 services built through the guest
  runner; strict AArch64 and x86-64 kernel Clippy passed with `-D warnings`.
  Service implementation is unchanged; the runtime change is documentation.
- Synchronous fixtures link 128 real target entries and verify source rejection
  rolls back its staged record/charge, then run 512 cancellation/rearm cycles.
  They cover normal/stale-generation exit, callback reentry into scheduler and
  completion registries, retained tokens/completions, shared-account exhaustion,
  cancellation before owner installation, worker deferred cancellation and
  exact numeric ASID/capability reuse during rejected late installation.
  Failed worker setup must neither run its entry point nor invoke its callback.
- Scheduled kernel tests cancel/rearm 128 watches on a still-live target,
  observe its eventual exit, run 32 immediate-return workers and cancel a held
  worker before permitting it to finish. Its completion stays pending and
  charged until actual exit. These forced conditions do not add an EL0 probe
  bit or simulate physical allocator exhaustion. Shared node/ordinary saturation
  remains covered by the preceding event-watch counter fixtures.
- The first guest fixture incorrectly assumed that reopening a synthetic
  completion namespace reset its capability numbering and panicked on that
  assertion. It was replaced with actual address-space retirement/reuse. This
  failed fixture run is excluded from successful validation.
- The corrected guest passed **19 tests, 0 failed, 0 pending**. The final version,
  with worker setup rollback checks, passed a second isolated four-LP AArch64/TCG
  run with the same result. Both scoped launches retained `0x7fff`; the final
  run retired concurrent cancellation traffic after 4,524 requests.
- Workspace formatting and diff checks passed. Markdown and LaTeX source were
  updated; the PDF was not rebuilt.

The final guest kernel SHA-256 is
`8113f7b0e0f94202462af68113e33dd07d563e5666ae5b739aa32a71ec05aad7`.
Evidence is in `/private/tmp/charlotte-security-exit-watch-repeat-run.log`,
`/private/tmp/charlotte-security-exit-watch-repeat-20261004-serial.log`,
`/private/tmp/charlotte-security-exit-watch-fixed-run.log`,
`/private/tmp/charlotte-security-exit-watch-fixed-20261004-serial.log`,
`/private/tmp/charlotte-security-exit-watch-run.log` and
`/private/tmp/charlotte-security-exit-watch-20261004-serial.log` (failed fixture),
`/private/tmp/charlotte-security-exit-watch-host-tests.log`, and the
`/private/tmp/charlotte-security-exit-watch-*-kernel-clippy.log` files.
No exhaustive cross-LP teardown/IRQ exploration, hostile-pressure soak,
physical allocator-OOM injection or x86-64 guest run was performed. Dedicated
storage and forwarded ports were used; existing soak guests/stores were untouched.

## 2026-10-04 continuation — owning kernel callbacks and mandatory waiter registration

The thread-exit changes are committed as `19d3d650`. This continuation removes
the completion object's unbounded non-scheduler weak-callback queue. The kernel
`observe` API now takes a strong callback and returns a `CompletionObservation`
owner; Drop removes only its subscription. The operation, producer and buffered
data retain their original lifecycle. There were no production callers of the
old helper to migrate; the new fixtures exercise the actual registration path.
No new userspace syscall or wire format is introduced.

Each completion accepts 128 callback entries, sharing the **existing** lifecycle
watch pool: completion capacity clamped to 1,024 per namespace, 8,192 per node
and 6,144 ordinary. This is not a new allowance per callback type. List/entry
allocation is fallible and lazy; entry storage has no retained vector capacity.
Retirement rejects admission, and rejection releases staged charges without
changing the operation. The exact-object `observe_registered` variant rejects a
replacement even with identical numeric ASID/capability values; the convenience
helper captures an object and uses that same checked path.

Terminal checking and insertion share the completion state lock. A registration
either precedes terminal publication or invokes immediately after releasing
registry/source guards, including already-observed operations. The latter needs
no linked entry or entry charge. Notification follows CQ publication, frees each
entry before callback invocation, and uses iterative detached storage. Namespace
teardown, unpublished-submission rollback and source destruction discard entries
even with retained operation/token references; they do not fabricate completion.
Cancellation may race an already captured callback. Caller-owned callback capture
memory and retained empty control blocks are not charged by these entry counts.

Source review found two remaining scheduler uses of the legacy observer path:
steady-state publication's growing vector and the result reporter's unbounded
queue, which separately pruned dead weak references after timeouts. Both now use
bounded `WaiterSource` registration and detached notification. Publication no
longer invokes callbacks under its observer guard. The `Observable` trait now
requires `try_register_waiter`; its weak-only default and legacy token are gone.
Timer internal callbacks retain a separate single embedded slot, now an inherent
method rather than part of the scheduler registration trait. Every current
scheduler source implements the owning method explicitly.

Contributor instructions, the [kernel callback reference](../../reference/completion-callback-budgets.md),
related ownership/budget/testing references, README and LaTeX sources record the
two registration contracts. Generic callback payloads, weak-only/control-block
backing, broader kernel work/metadata storage, capability namespaces and
loader/page-table/heap accounting remain open. SEC-07 is still partial; no
hostile-production containment or per-service progress guarantee is claimed.

Validation:

- Host tests passed. Guest runners built bundled AArch64 services; strict
  AArch64 and x86-64 kernel Clippy passed with `-D warnings`. Service and runtime
  implementations are unchanged in this continuation.
- Synchronous fixtures allocate 128 real callback entries, test source/shared
  account rejection, full-batch notification despite retained tokens, 512
  cancel/rearm cycles, late/reentrant delivery, preserved buffers, pending
  producer cancellation, rollback with retained objects, retirement and exact
  numeric namespace reuse. Captured-object registration against the replacement
  returns `UnknownCap`, without charging or changing it.
- A shared-list fixture injects entry allocation failure after charge admission,
  checks zero linked entries/charges/weak references and proves successful retry.
  This is component-level entry allocation injection, not physical allocator
  exhaustion or failure injection at every submission/list allocation point.
- Both actual status sources reject the 65th entry, run 512 cancel/rearm cycles
  and test detached/reentrant notification. Scheduled fixtures temporarily swap
  only the executing kernel test thread's sponsor to measure 64 publication and
  64 results timeout cleanups independently of other verifiers. The original
  sponsor is restored through an owner. Thirty-two real immediate-return workers
  check one callback per operation while registration races terminal delivery;
  this does not exhaustively explore every interleaving.
- Two preliminary guests passed 19/19 before the final allocation/full-batch/
  captured-object/worker-race fixtures. The **final implementation** passed two
  further isolated four-LP AArch64/TCG runs: **19 tests, 0 failed, 0 pending**,
  with both scoped launches retaining `0x7fff`. The final run retired concurrent
  cancellation traffic after 4,520 requests. No failed guest run occurred in
  this continuation.
- Workspace formatting and diff checks passed. The PDF was not rebuilt.

The final guest kernel SHA-256 is
`86e1931e7c867d77116f0ed277a1b0aca45fe929bdc3a9d9d5346ee47004aefa`;
the first final-fixture run used
`b55881e6061f92318d8e9c6ae6e71a3680325ccef05a74ba20c9aabaee28b3b2`.
Evidence is in `/private/tmp/charlotte-security-callback-verified-run.log`,
`/private/tmp/charlotte-security-callback-verified-20261004-serial.log`,
`/private/tmp/charlotte-security-callback-final-run.log`,
`/private/tmp/charlotte-security-callback-final-20261004-serial.log`,
`/private/tmp/charlotte-security-callback-repeat-run.log`,
`/private/tmp/charlotte-security-callback-repeat-20261004-serial.log`,
`/private/tmp/charlotte-security-callback-run.log`,
`/private/tmp/charlotte-security-callback-20261004-serial.log`,
`/private/tmp/charlotte-security-callback-host-tests.log` and the
`/private/tmp/charlotte-security-callback-*-kernel-clippy.log` files.
No physical allocator exhaustion, exhaustive IRQ/teardown exploration,
x86-64 guest or hostile-pressure soak was performed. Dedicated storage and
forwarded ports were used; existing soak guests/stores were untouched.

## 2026-10-04 continuation — per-route IRQ readiness and exact CQ wake preparation

The owning callback/waiter changes are committed as `5e7b0a10`. Reviewing
remaining kernel work queues found that the deferred-work manager still has an
uninhabited task enum and no submit/worker callers; its unbounded placeholder
is not an active remotely driven queue. It was not presented as a new fixed
security defect. Any future usable task path still needs bounded admission.

The active device IRQ handoff did have a defect: every delivery pushed into a
shared bounded FIFO, with ignored push failure and a comment asserting that a
full queue already contained an equivalent wake. Repeated deliveries or stale
generations can occupy that capacity without preserving another route's wake.
The replacement gives every routing slot one static 64-bit coalescing mailbox:
288 on AArch64, 476 on x86-64. It allocates nothing at runtime and has no shared
capacity admission/drop path. One bounded sweep prevents a producer from
indefinitely refilling the current drain pass. This introduces a fixed scan
cost; no throughput improvement was measured or claimed.

Claims retain the generation watermark. Bind/retirement advances it so a late
old publisher cannot overwrite new pending readiness or resurrect a retired
generation. Binding preserves a retirement identity and fails with device
status 16 before 63-bit exhaustion; retirement cannot wrap into a live identity.
Individual close already held the device guard through unroute; whole-domain
teardown now does so too, before another grant can reclaim the source.

The old drain also checked generation separately from destination lookup and
CQ publication, leaving a route-replacement window. It now validates and calls
`completion::prepare_wake` under the device-management guard. Preparation bumps
work generation and detaches the exact queue's waiters under one completion
registry guard; notification runs only after all subsystem guards are released.
Ordinary explicit CQ wakes use that same preparation path, avoiding a second
numeric lookup after publication. Captured original waiters can still notify
after retirement, as detached notification already permits; replacement
waiters/work generation are not selected by that captured batch.

The [IRQ wake reference](../../reference/interrupt-wake-storage.md), architecture,
locking/scheduler references, testing guide and LaTeX driver chapter describe
these guarantees. Incorrect current documentation claiming full-queue drops
were safe was removed. The existing single-route TLA+ conformance entry is now
explicit about its abstraction: it does not prove the mailbox algorithm,
capacity, exhaustion or controller MMIO. No new formal model was checked.

Validation:

- Host tests passed, including six new tests of the **production mailbox
  primitive**: independent-slot flooding, retirement/rebind and delayed old
  publication, publication after claim, generation exhaustion, concurrent stale
  publishers and competing consumers. These are not exhaustive weak-memory
  interleaving exploration.
- Strict AArch64 and x86-64 kernel Clippy passed with `-D warnings`, both before
  and after adding the final IRQ callback fixture. The guest runners built the
  bundled AArch64 services; service/runtime implementations are unchanged.
- The kernel device fixture floods real delivery beyond twice the previous
  queue capacity, retires/rebinds the source, checks stale-readiness rejection
  and a fresh wake/ack. Other LPs may drain during this integration fixture;
  independent-slot coalescing is checked deterministically by the host test.
- A prepared-CQ fixture detaches a wake, retires/reopens the exact numeric
  namespace, registers a replacement waiter and verifies that the captured
  batch changes neither its work generation nor its notification count. The
  final IRQ fixture invokes a callback through actual deferred dispatch that
  reenters device/completion registries. It waits for a potentially competing
  LP's in-flight claim rather than assuming its local drain wins.
- Two preliminary isolated four-LP AArch64/TCG security guests passed **19/19**.
  The final implementation, including the reentrant IRQ callback fixture,
  passed a further guest: **19 tests, 0 failed, 0 pending**, with both scoped
  launches retaining `0x7fff`. Concurrent cancellation traffic retired after
  4,420 requests. No failed guest run occurred in this continuation.
- Workspace formatting and diff checks passed. The PDF was not rebuilt.

The final guest kernel SHA-256 is
`38a155332b759ad64aa102f4ede14ff8ba087b990d5275ecb38a3f13baa8e688`.
Evidence is in `/private/tmp/charlotte-security-irq-mailbox-host-tests.log`,
`/private/tmp/charlotte-security-irq-mailbox-run.log`,
`/private/tmp/charlotte-security-irq-mailbox-repeat-run.log`,
`/private/tmp/charlotte-security-irq-mailbox-final-run.log`,
the corresponding `charlotte-security-irq-mailbox-*-20261004-serial.log` files,
and `/private/tmp/charlotte-security-irq-mailbox-*-kernel-clippy.log`.
No physical allocator exhaustion, x86-64 guest, hardware IRQ stress,
exhaustive cross-LP masking/rearming or hostile-pressure soak was performed.
Dedicated storage and forwarded ports were used; existing soak guests/stores
were untouched. SEC-07 remains partial: capability namespace/loader/heap/page
tables, arbitrary callback captures and broader kernel metadata still need
admission/accounting work; the other open audit findings remain open.

## 2026-10-04 continuation — bounded mailbox capability records

The IRQ readiness changes are committed as `b56c94e4`. Review of aggregate
capability admission found that destination reservation and rollback escrow
are needed before adding a shared ceiling across IPC attachment moves and
other families. This continuation addresses the still-unbounded mailbox
handle family and documents the remaining aggregate design, rather than
claiming the complete capability namespace is now budgeted.

Mailbox sender/receiver entries own a record charge: **512 per domain
namespace, 8,192 per node, 6,144 ordinary**. The remaining 2,048 form one shared
platform reserve. Policy is captured against the exact kernel-designated
address-space generation, not a name, role or caller field. Each record keeps
its captured classification; later platform designation affects future opens.
Node counters are statically initialized behind the IRQ-safe mutex, avoiding
a new first-use `LazyLock` spin dependency in the syscall path.

Admission precedes identity minting and payload publication. The new
`capability::try_allocate` returns serial exhaustion without mutating existing
authority; a failed mailbox mint drops its staged charge. Other capability
families retain the legacy infallible wrapper. Open failure remains the ABI's
zero-capability result. Full-width sender LP validation now rejects high bits
before narrowing. Existing per-LP receiver lookup consumes no new slot and
returns the same capability, not independent ownership. Closing an entry
revokes its authority before dropping the owning record charge.

Mailbox open holds `ADDRESS_SPACE_LIFECYCLE → USER_MAILBOX_CAPS` through
validation and publication, serializing it with production retirement and
reuse. A retirement snapshot alone would leave a publication window. These
rare metadata opens are now serialized across domains; ordinary send/receive
does not acquire this lifecycle guard. Teardown retires the namespace's budget
before releasing entries, and retained reservations credit only their
original account. A captured predecessor cannot recreate a retired namespace
or mint into a replacement using the same numeric ASID.

The [mailbox capability reference](../../reference/mailbox-capability-budgets.md),
ownership/testing guides, locking reference and LaTeX programming-model chapter
document the limits and receiver-ownership rule. The reference also specifies
the next aggregate contracts: pre-transfer destination reservation, preserved
source escrow for rollback, atomic vector admission and captured namespace
identity in staged owners. The TLA+ conformance table distinguishes successful
serial minting from these unmodeled record/admission lifetimes; no new formal
model was checked.

Validation:

- Host regression tests passed. No host crate or userspace service/runtime
  implementation changed in this continuation; these new fixtures are kernel
  tests, not a new real-EL0 mailbox quota probe. The scoped mask stays `0x7fff`.
- The actual syscall dispatcher is used to fill 512 mixed sender/receiver
  handles, reject an additional sender, reuse a receiver at capacity, close
  and refill a slot, then run 1,024 open/close cycles. Rejection consumes no
  serial. High-bit-invalid sender LPs are tested both with spare capacity and
  at the ceiling.
- Serial-exhaustion injection preserves an existing handle, rejects another
  open and refunds staged admission. A retained reservation spans exact
  numeric namespace/capability reuse; its destruction changes only the old
  account. Counter fixtures exercise ordinary/node ceilings, reserved platform
  progress, failed ordinary-reservation rollback and recovery, using isolated
  production counter code, not thousands of registry allocations.
- Real address-space handles test rejection after retirement, after mailbox
  registry removal and after numeric ASID reuse. A stale captured open leaves
  the replacement's record/authority unchanged.
- Strict AArch64 and x86-64 kernel Clippy passed with `-D warnings`, including
  the final statically initialized counter and spare-capacity LP fixture.
  Both guest runners rebuilt bundled AArch64 services through the normal runner.
- One preliminary isolated four-LP AArch64/TCG guest passed **19/19**. The
  final implementation passed another: **19 tests, 0 failed, 0 pending**,
  including both scoped launches with `0x7fff`; concurrent cancellation traffic
  retired after 4,492 requests. No failed guest occurred in this continuation.
- Workspace formatting and diff checks passed. The PDF was not rebuilt.

The final guest kernel SHA-256 is
`2272510b864ea3aa65c7d813f6ac1a5ad22b61ad088a69c122156a127957d24b`.
Evidence is in `/private/tmp/charlotte-security-mailbox-caps-host-tests.log`,
`/private/tmp/charlotte-security-mailbox-caps-run.log`,
`/private/tmp/charlotte-security-mailbox-caps-20261004-serial.log`,
`/private/tmp/charlotte-security-mailbox-caps-final-run.log`,
`/private/tmp/charlotte-security-mailbox-caps-final-20261004-serial.log` and
`/private/tmp/charlotte-security-mailbox-caps-*-kernel-clippy.log`.
Dedicated storage and forwarded ports were used; existing soak guests/stores
were untouched. No x86-64 guest, physical allocator failure injection,
many-client quota fairness test or exhaustive concurrent-retirement exploration
was performed. SEC-07 remains partial: these record counts do not charge the
legacy/capability mailbox queue backing, empty namespace/control-block memory,
BTreeMap allocator bytes, aggregate capabilities, loader/page-table/heap
backing or arbitrary callback captures. The other open findings remain open.

## Continuation: shared capability accounting and staged admission — 2026-10-04–05

The preceding mailbox-record continuation was committed as `6d458d7`. This
continuation introduces one capability domain/node account shared across all
six object kinds. Its defaults are 4,096 records per namespace, 65,536 per node
and 49,152 ordinary records. Staged, live and source-escrow entries retain one
owning charge each. Mailbox opens and capability-backed completion submissions
(ordinary, timer, event-watch and worker preparation) now reject shared pressure
as well as family pressure. A mailbox reports zero; completion reports existing
`WouldBlock` backpressure. Detached operations have no capability entry and
continue to use their existing retained-record budget.

User domain registration prepares its budget control block before allocating
an ASID and publishes the exact generation-bearing namespace. Teardown retires
shared admission before draining any family. Platform designation updates the
matching namespace after releasing the memory ledger and address-space table;
outstanding record charges retain their original class. Shared allocation does
not take the address-space table while owning `CAPABILITIES`. Completion uses
its registry's captured generation under that registry's guard. Mailbox borrows
its already-owned lifecycle guard instead of reacquiring it.

`Reservation` keeps unpublished authority hidden and cancels it on Drop.
`MoveEscrow` hides the source but preserves its quota slot for capacity-independent
restoration or committed revocation. Both capture the exact budget object and
check it before modifying an entry; old tokens cannot remove or revive a
replacement with the same ASID/capability number. Their entry charge is released
at namespace teardown even if a token retains the old control block. Cancelled
staged serials remain consumed; rejected admission consumes no serial.

There is no backward-compatibility requirement. The old generic
`capability::allocate` and scalar `restore` API names were removed. The remaining
payload paths use explicitly named `allocate_unmigrated`/`restore_unmigrated`
helpers, which account records but bypass shared policy. This is a temporary
transaction-cutover boundary, not an API retained for old callers. It avoids
inserting a newly fallible quota check after those paths already mutate
ownership. New contributors must not add callers. IPC, memory, device and
observer payload paths still require conversion and removal of both helpers.
In particular, production memory/IPC moves do not yet use the new source-escrow
primitive, and it refuses restoration after retirement. Owning payload rollback
must explicitly resolve that case before migration. Neither an atomic vector
reservation API nor pre-dequeue reply-cap admission has been implemented here.

The reference, contributor instructions, ownership/testing and locking guides,
mailbox reference, TLA+ conformance table and LaTeX programming-model chapter
state this partial enforcement. Count admission does not charge BTreeMap bytes,
empty namespace/control blocks, loader/heap/page-table backing or arbitrary
captures. `Arc` preparation is fallible; BTreeMap allocation still is not.
Unconverted paths can exceed the policy and consume platform headroom, so this
does not close SEC-07 or establish allocator exhaustion safety. No new formal
model was checked, and the PDF was not rebuilt.

Validation corrections discovered during guest runs:

- The first run rejected a platform progress fixture that had inserted its
  address space directly into the global table. All real-domain self-tests now
  use production `register_user_address_space`, including namespace, limits and
  accounting setup. Three unused copies of the current kernel address space in
  the pseudo-domain adversarial test were removed, rather than preserved as
  legacy test scaffolding.
- A second run reached the new shared-admission fixtures and rejected their
  attempt to close a still-in-flight completion. The fixture now explicitly
  completes the operation before closing it. Both failed runs produced a kernel
  assertion and no authoritative passing verdict; they are not counted as
  successful validation.

Final validation:

- Host regression/signing tests passed. No host-library or userspace service
  implementation changed in this continuation; the new admission tests run
  inside the kernel, not as host tests or new EL0 quota probes.
- Kernel fixtures filled 4,096 actual mixed-kind namespace entries, rejected
  further reservation, restored escrow at the ceiling, committed revocation,
  cancelled a full staged batch and checked exact-state numeric replacements.
  Explicit unconverted over-limit allocations remained counted. Isolated
  production node counters tested limits/headroom without filling the live pool.
- Actual mailbox dispatch, completion and timer submission paths rejected
  shared pressure with room in family pools, refunded family staging, consumed
  no serial on rejection and recovered after slot release. Rejected timer
  staging left zero timer events. Real-domain teardown/reuse rejected old tokens
  and captured predecessor generations without changing the successor.
- Final strict AArch64/security and x86-64 kernel Clippy passed with
  `-D warnings`; workspace formatting and diff checks passed.
- The corrected isolated four-LP AArch64/TCG guest passed **19 tests, 0 failed,
  0 pending**, including both scoped launches with unchanged `0x7fff` checks;
  cancellation stress retired after 4,396 requests. The runner rebuilt bundled
  services through the normal build path and used dedicated storage/ports.

The passing kernel SHA-256 is
`bf88ec40dce102fda62b9e9ddc476ee8b31aa79a73cf77e9c7bfb6986391ac96`.
Evidence is in `/private/tmp/charlotte-security-capability-admission-host-tests.log`,
`/private/tmp/charlotte-security-capability-admission-*-clippy.log`,
`/private/tmp/charlotte-security-capability-admission-verified-run.log` and
`/private/tmp/charlotte-capability-admission-verified-20261005-serial.log`.
The two failed captures are retained in the initial and `final-run` log files;
only `verified-run` has a passing authoritative verdict. Existing soak guests
and storage were untouched. No x86-64 guest, physical allocation-failure
injection, many-client quota fairness or exhaustive retirement interleaving
test was performed. SEC-07 and the other open findings remain open.

## Continuation: bounded memory authority and owning move preparation — 2026-10-05

Shared accounting and the mailbox/completion cutover were committed as
`0d65610b`. This continuation converts every memory-object destination to
bounded shared admission: allocation, copy, move, read-only move and read/write
loan. Admission precedes frame allocation or source authority mutation. A copy
rejection drops its source pin and staged backing; rejected loans leave their
source access/lend state unchanged.

`PreparedMove` owns the reserved destination, the source's charged escrow slot
and a backing-retention pin. Preparation hides source authority but leaves the
payload source-owned. Cancellation restores the original handle without fresh
quota and releases destination admission and the pin. `commit_moves` validates
the entire batch under the memory registry, then publishes all authorities
atomically under the capability registry before infallible payload updates.
Source/target namespace identity is captured before taking the memory registry;
no lifecycle acquisition is added under memory or IPC serialization.

Retirement can precede payload drain. Cancellation may restore an existing
source slot in that same retiring namespace for cleanup, but cannot admit new
authority or revive a removed/replaced namespace. Source teardown removes the
escrowed payload authority while the owning pin retains its frames; late Drop
releases the old sponsorship charge without debiting a successor generation.
The scalar `rollback_move_to` and `restore_unmigrated` APIs and their cleanup
ladders are removed rather than kept as compatibility paths.

IPC move vectors use prepared owners and commit their move batch before enqueue
under the same IPC queue reservation. Reply memory is prepared before loan
revocation. Quota rejection preserves both loan and reply token; a successful
revocation followed by late publication rejection clears the token's loan
record, so retry/cancellation does not try to revoke it twice. The multi-state
kernel upgrade prototype now returns errors and owns its loaded replacement
plus prepared moves, closing the staged domain on failure. That prototype has
no caller; the actual userspace upgrade syscall remains single-state. Markdown
and LaTeX upgrade descriptions now distinguish the two paths.

Validation:

- Host regression/signing tests passed. The new tests are kernel fixtures,
  not host-library tests or a new real-EL0 quota mask.
- Strict AArch64/security and x86-64 kernel Clippy passed with `-D warnings`;
  workspace formatting and diff checks passed.
- Real-domain fixtures filled spare namespace slots, checked memory admission
  rejection and refunds, cancellation with a full source namespace, a successful
  two-object move batch, and all-or-none rejection after target retirement.
  Paused source retirement restored original slots for cleanup. Source and
  target teardown/reuse explicitly reused both ASID and numeric capability,
  preserving successor authority/budgets and retaining old frames only until
  cancellation.
- Actual kernel IPC calls rejected the second vector move with one destination
  slot remaining, restored both source handles at the source ceiling, refunded
  staging and queued no message. A memory reply rejected at the caller ceiling
  preserved its loan and token, then succeeded when one slot was freed. Existing
  vector/reply and ordinary EL0 integration regressions also passed.
- The final isolated four-LP AArch64/TCG guest passed **19 tests, 0 failed,
  0 pending**, including both scoped launches with unchanged `0x7fff` checks.
  Concurrent cancellation retired after 4,416 requests. The runner rebuilt
  bundled services and used dedicated storage/ports; existing soak guests and
  storage were untouched.

The final passing kernel SHA-256 is
`17e3583fb0ec63ecae968029d6669b2dd374a8d347aeeaab46b37555f0a34048`.
Evidence: `/private/tmp/charlotte-security-memory-moves-host-tests.log`,
`/private/tmp/charlotte-security-memory-moves-*-clippy.log`,
`/private/tmp/charlotte-security-memory-moves-final-run.log` and
`/private/tmp/charlotte-memory-moves-final-20261005-serial.log`.
Initial compilation found incorrect fixture imports/private-constant access;
those were corrected before the passing guests. An earlier passing guest
preceded the final IPC pressure regressions and is not the final verdict.

Remaining scope: IPC/device/system-observer capability minting still uses the
counted but unbounded allocator. Copy/loan vector aliases still become live
before complete preparation; a receiver can guess such handles and change
their state, so their existing rollback infallibility assumption needs removal
alongside hidden alias staging. This continuation closes the move path, not
that mixed-mode publication gap. Complete aggregate admission, BTreeMap and
physical-allocation failure handling, loader/page-table/heap budgets and the
other open findings remain. No exhaustive concurrent-retirement exploration,
end-to-end multi-state upgrade test, x86-64 guest or new formal proof was run.
The TLA+ conformance notes were updated; models and the PDF were not rebuilt.
SEC-07 remains partial.

## Continuation: hidden mixed-mode memory preparation — 2026-10-05

The memory-admission and owning-move batch was committed as `50cf60e7`.
This continuation extends that owner to every memory transfer mode. The
kernel API is now `PreparedTransfer`/`commit_transfers`; `SourceEscrow` reflects
its use for loans as well as moves. No compatibility wrappers remain for the
old move-only API or vector alias-rollback ladder.

Copies hold private charged frames outside the receiver's payload registry.
Loans retain source escrow/backing pins but install no borrower state until
commit. Every destination remains non-authoritative during preparation. A
receiver guessing a staged handle cannot map, write, close, query physical
backing or pin it. Cancellation therefore needs no operation on mutable
receiver state and removes the prior assertion that vector alias rollback must
be infallible. Mixed batches validate every lifetime before publishing all
memory authority under one capability-registry hold; payload updates follow
under the same memory guard.

Allocation/copy backing admission now checks the sponsor generation captured
before preparation. A delayed reservation cannot accidentally charge an ASID
successor. Retained copied or pinned backing still debits its original sponsor
until physical release. Read-loan preparation permits pre-existing read-only
mappings/borrowers; write loans require exclusive unmapped backing. Existing
read-only mappings can still read during source-capability escrow.

Inspection also found that `vector_call` previously created read/write loans
without retaining them in its reply token. Tokens now own a bounded vector of
all loan pairs, allocated before publication. Reply, queued/delivered caller
cancellation, reply-token close and queued endpoint close revoke every loan.
Queued cancellation additionally releases copies/moves; already-delivered
ownership remains with the receiver. Each successful reply revocation is
removed immediately, so a later failure does not cause duplicate revocation.
Committed mapped-loan unmap failure is still a fallible operation; private
preparation cancellation is not that path.

Kernel `snapshot_bytes`, `write_bytes` and DMA pinning now respect live loan
permissions, not only capability bits. An owner cannot write through read loans
or read/write/DMA through another domain's exclusive write loan. The designated
write borrower retains its granted access. Preparation pins continue to fence
tracked writes while retaining backing.

Validation also exposed a runner race: its prefix-only completion check stopped
QEMU in the middle of `SELFTEST COMPLETE`, leaving a truncated result. The
shared AArch64/x86-64 stop condition now requires a fully terminated record.
Final validation uses the same parser and requires a successful verdict; the
poll loop caches completion during long holds. Five host parser tests cover
every partial prefix, LF/CRLF, a subsequent partial line, complete failed/pending
verdicts, malformed/truncated results and panic rejection. This is a capture
correctness repair, not a new kernel containment claim.

Validation:

- Host regression/signing and boot-result tests passed. Strict AArch64/security
  and x86-64 kernel Clippy passed with `-D warnings`; workspace formatting,
  shell syntax and diff checks passed.
- Deterministic real-domain fixtures paused copy/read-loan/write-loan
  preparation and attempted guessed-handle access. They checked copy snapshot
  independence/refunds, cancellation at the source ceiling, a source read-only
  mapping, all-or-none mixed retirement and exact source/destination
  ASID/numeric-handle reuse without affecting successor budgets or authority.
  Stale captured backing reservations were rejected.
- Actual kernel vector calls checked failure after each transfer kind with a
  full source namespace, denied copy backing after a prepared loan, and
  four-mode delivery. Mapped read/write loans were revoked on reply, queued and
  delivered cancellation, reply-token close and queued endpoint close. Queued
  copied/moved backing was reclaimed; delivered copy/move ownership survived.
  Owner snapshot/write/DMA paths rejected access forbidden by active loans.
- The initial guest passed before the final DMA assertions. The next reached
  the new regression markers but was stopped mid-verdict by the runner race;
  it is **not** counted as a passing authoritative capture. After repairing
  the runner, the verified four-LP AArch64/TCG guest passed **19 tests, 0 failed,
  0 pending**, including both scoped launches with unchanged `0x7fff` checks.
  Cancellation stress retired after 4,388 requests. Bundled services were
  rebuilt through the normal runner, using isolated storage/ports. Existing
  soak guests and storage were untouched.

The verified kernel SHA-256 is
`5c28e42f33d2893f0c40743dbf300d5f109634d1a16063678bb44153021c5d4d`.
Evidence: `/private/tmp/charlotte-security-memory-staging-host-tests.log`,
`/private/tmp/charlotte-security-memory-staging-*-clippy.log`,
`/private/tmp/charlotte-security-memory-staging-verified-run.log` and
`/private/tmp/charlotte-memory-staging-verified-20261005-serial.log`.
The truncated capture is retained as `memory-staging-final`/`final-run` for
comparison. Preliminary compilation corrected a renamed-helper shadow and the
captured generation type; these were not passing validation runs.

SEC-07 remains partial: fresh IPC/device/system-observer capability publication
still uses the counted but unbounded allocator, including receive-side reply
authority. General allocator bytes, BTreeMap/physical-allocation failure paths,
loader/page-table/heap budgets and other open audit findings remain. The new
tests are kernel fixtures, not new real-EL0 quota probes or exhaustive concurrent
retirement/unmap-failure exploration. No x86-64 guest, end-to-end multi-state
upgrade test or new formal proof ran. TLA+ conformance and LaTeX sources were
updated; models and the PDF were not rebuilt.
