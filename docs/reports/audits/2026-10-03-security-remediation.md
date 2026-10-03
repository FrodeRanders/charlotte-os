# Security remediation — 2026-10-03

This records the first implementation pass following the
[security audit](2026-10-03-security-audit.md) of revision
`42183c57ce4c0b32a6010246f6eee1b6262ebb4e`. It is not a declaration that the
audit is closed or that CharlotteOS is ready for hostile production workloads.
Implementation and validation span 2026-10-03–04 local time.

## Finding ledger

“Implemented” means the identified code defect has a correction in this working
tree. Validation scope is listed separately; it does not imply that every
acceptance test proposed by the audit has run. “Mitigated” leaves part of the
finding open. “Open” means no correction was implemented in this pass.

| Finding | State | Correction or outstanding requirement |
| --- | --- | --- |
| SEC-01 | Implemented | Raw explicit mapping addresses and complete ranges are checked before address normalization or page-table mutation; both architectures reject user-accessible mappings outside the user window. |
| SEC-02 | Implemented | Kernel records the exact admitted descriptor digest against the domain generation; grantctl obtains attestation instead of accepting another signed policy for the same artifact name. |
| SEC-03 | Implemented | Legacy and capability mailbox queues are domain-local and discarded on domain teardown. |
| SEC-04 | Open | Explicit development/production modes, independently provisioned role keys, fixture rejection, protected boot and recipient-key provisioning remain necessary. |
| SEC-05 | Implemented | tcpip rejects raw frame ingress unless the authenticated sender is the exact live, kernel-designated frouter. Separate socket/VIP binding policy remains future hardening. |
| SEC-06 | Mitigated | HTTP EOF, peer-raced accept and transport failures close one connection, not the server; listener-setup resource failures retry with backoff. httpd has a five-second request wait and bounded send retries; deployd has five-second header and thirty-second total receive budgets. Serial admission remains vulnerable to sustained connection floods. |
| SEC-07 | Open | Aggregate memory, capability, endpoint, completion and queued-work accounting needs a common kernel admission/budget mechanism. Per-object bounds and these service limits do not replace it. |
| SEC-08 | Open | Authenticate enrolled nodes and control/data peer traffic, add replay protection, and bound discovery state. A trusted L2 segment remains an explicit deployment prerequisite. |
| SEC-09 | Open | Distinguish authenticated security time from observational SNTP/holdover; enforce freshness and uncertainty at security-policy gates. |
| SEC-10 | Open | Authenticated encrypted access to node and cluster management, browser-client provisioning, and access policy remain necessary. |
| SEC-11 | Implemented for scoped applications | Scoped application mapping now uses the configured artifact key. grantctl relies on launcher attestation, which used the configured deployment key. Bundled platform-service trust and production provisioning still belong to SEC-04. Custom-root end-to-end deployment remains to be tested. |
| SEC-12 | Open | Attenuate local object-store authority to object sets/namespaces; retain an explicitly separate administration endpoint. |
| SEC-13 | Open | Migrate all signing commands and callers to restricted key files/descriptors or a signer; do not put real private keys in positional arguments or traced shell variables. |
| SEC-14 | Mitigated; audit corrected | Cargo.lock is already tracked. Main build/test runners and CI now enforce --locked; CI actions are commit-pinned, token permissions are read-only, and checkout does not persist credentials. Advisory/license scans and a release dependency inventory remain. |
| SEC-15 | Mitigated | SigV4 prefixed secret, derived keys, HMAC block/pads and inner digest use zeroizing owners. TLS record buffers are wiped after dropping their borrower, including handshake failure. This is not a complete audit of crypto-library state or compiler-created secret copies. |
| SEC-16 | Implemented | grantctl polls bounded concurrent operations, with per-sender/generation limits and total deadlines; its non-parking authorized lookup cannot leave cancelled grant requests in the shared name-service waitlist. The application helper retries within a total deadline. End-to-end stalled-target/flood testing remains. |

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

1. Add adversarial scoped-application integration tests for descriptor
   substitution, concurrent unavailable/available grant requests, cancellation,
   generation reuse, and unauthorized raw-frame calls. Exercise independently
   generated artifact/deployment roots end to end.
2. Make production trust explicit and fail closed without provisioning; migrate
   signing-key input away from argv. Keep developer fixtures available only as
   visibly identified development infrastructure.
3. Introduce aggregate kernel resource reservations/accounting with rollback
   and release on all cancellation/transfer/teardown paths. Reserve essential
   service capacity and test exhaustion without kernel panic or starvation.
4. Separate security-time provenance from ordinary clock synchronization, then
   authenticate management and enrolled node traffic with bounded replay state.
5. Attenuate local storage authority and make HTTP admission concurrent and
   bounded. Add advisory/license policy checks and release provenance for the
   locked dependency graph, toolchain and pinned action inputs.

Until those controls exist, retain the audit's operational restrictions: use
only public fixtures, no real credentials under development recipient keys,
trusted network segments and management access, and no mutually distrustful
application workload assumptions.
