# CharlotteOS renewed security audit — 2026-10-05

This follows the [2026-10-03 audit](2026-10-03-security-audit.md) and its
[remediation ledger](2026-10-03-security-remediation.md). It reviews revision
`62bc8f011ff45f66ac4531f534c6d63f98229c19`, after the explicit IPC cancellation
ownership changes. The working tree was clean when review began. No production
code was changed during this audit. Subsequent corrections and validation are
recorded in the [remediation report](2026-10-05-security-remediation.md).

## Assessment

The hardening materially improves the application boundary. The original
kernel-half mapping defect, launch-policy substitution, global userspace
mailboxes, ordinary-client raw frame ingress, and serial grant-controller
lookup stall have concrete corrections. Admission now follows many retained
resources through transfers, cancellation and physical retirement instead of
merely counting public handles. Failed physical cleanup increasingly preserves
backing and authority fences rather than declaring false completion.

Nevertheless, this revision should still be treated as a development system
for trusted applications on isolated management and cluster networks. Two new
High findings concern application-triggered kernel panic and persistent shared
socket exhaustion. Two Medium findings concern foreign-thread subscriptions
and deterministic TCP/IP random state. None requires defeating artifact
signatures; a signed application can be compromised or intentionally hostile.

Production provisioning remains disabled, rather than implemented. Cluster
peer authentication, authenticated security time, authenticated management,
translation-table admission, and parts of bulk teardown remain open. Passing
the existing security suite does not establish containment of mutually
distrustful applications.

## Scope and method

- Read the original findings and the complete remediation ledger's current
  contracts and relevant continuations; cross-checked testing and retirement
  reference documents against source.
- Traced EL0 syscall dispatch, explicit mapping and thread creation, thread
  lifecycle observers, capability admission, memory/frame preparation,
  translation walkers, IPC reply/cancellation and bulk cleanup.
- Reviewed grant attestation and namespace mediation, TCP/IP ownership and
  reclamation, management receive handling, connector TLS/profile boundaries,
  signing/build policy and CI configuration. These were selected boundary
  reviews, not line-by-line coverage of every service or driver.
- Ran the host test runner, rebuilt bundled AArch64 services through the
  documented runner, booted an isolated fresh-storage AArch64 security guest,
  and scanned the committed lockfile with cargo-audit.
- New findings are **confirmed in source**, with conditions and impact
  qualified below. No new attack payload was executed. Existing guest tests
  exercise the established regressions; they do not reproduce SEC-19–22.
- No x86 guest execution, physical-device experiment, concurrent hostile
  syscall stress, parser fuzzing campaign, cryptographic implementation proof,
  or independent audit of sibling projects was performed. x86 source review
  is not x86 execution evidence.

The threat model remains the original audit's: compromised admitted
applications, unauthenticated network clients, hostile L2 peers, compromised
platform services/connectors, malicious devices and compromised build hosts.
Kernel, supervisor, authority mediators and credential-holding connectors are
trusted components. Peer consensus remains non-Byzantine under its documented
deployment assumptions.

High means a substantial containment or shared-availability failure. Medium
means a narrower or conditional authorization/network exposure. “Confirmed”
does not mean an end-to-end exploit was executed. Source line references are
for the reviewed revision and may drift after remediation.

## New findings

| ID | Severity | Finding | Required access or condition |
| --- | --- | --- | --- |
| SEC-19 | High | User thread construction turns a recoverable stack collision into kernel panic | Any running application; collision with the stack slot chosen for a new thread |
| SEC-20 | Medium | Thread-exit subscriptions are not restricted to the caller's domain | Any running application; a live foreign TID; enough occupancies for source exhaustion |
| SEC-21 | High | Remote socket resources outlive dead clients and can exhaust the shared stack | A TCP/IP CALL capability; client abandonment, termination or cancelled creation |
| SEC-22 | Medium | TCP/IP uses public deterministic random state on every launch | Network visibility or spoofing opportunity; protocol-state predictability matters |

### SEC-19 — Stack mapping failure is an application-triggered kernel panic

**Confirmed in source; High.**

`SPAWN_THREAD` constructs `Thread::new` before trying fallible thread
publication. For a user domain, that constructor calls
`create_user_thread_context(...).expect(...)`. Both architecture constructors
return an error if a newly selected user-stack page is already mapped.

The stack allocator chooses from a fixed, monotonically indexed virtual
region. Ordinary explicit memory-object mappings accept vacant pages in that
same region: the corrected mapping gate checks the user window and complete
range, but does not reserve future stack slots. The ELF validator's runtime
exclusions also do not reserve this stack region. A legitimate user mapping
that collides with a subsequently selected stack page therefore changes a
recoverable allocation/layout failure into kernel panic.

Evidence:

- [syscall/mod.rs:884](../../../crates/catten/src/syscall/mod.rs#L884), especially
  line 913: construction precedes `publish_thread` and its rejection handling.
- [threads/mod.rs:392](../../../crates/catten/src/cpu/scheduler/threads/mod.rs#L392),
  especially line 405: user context failure is unconditionally expected away.
- [ARM thread_context/mod.rs:351](../../../crates/catten/src/cpu/isa/aarch64/lp/thread_context/mod.rs#L351):
  fixed stack slots and mapping-error return at line 434.
- [x86 thread_context.rs:357](../../../crates/catten/src/cpu/isa/x86_64/lp/thread_context.rs#L357):
  the equivalent slot selection and fallible mapping path.
- [memory/object.rs:677](../../../crates/catten/src/memory/object.rs#L677) and
  [loader.rs:220](../../../crates/catten/src/service/loader.rs#L220): mapping
  validation does not establish an exclusive future-stack reservation.
- [system_scheduler/mod.rs:104](../../../crates/catten/src/cpu/scheduler/system_scheduler/mod.rs#L104):
  the thread ceiling is checked during publication, after stack preparation.
- [panic.rs:50](../../../crates/catten/src/panic.rs#L50): kernel panic masks
  interrupts and permanently stops the initiating LP. Other LPs are not
  automatically stopped; abandoned kernel state and loss of that LP can affect
  shared service progress.

No kernel-memory modification or arbitrary code execution is established by
this finding. Physical allocation failure reaches the same constructor panic,
but proving physical exhaustion is unnecessary to establish the collision
path. Even a restrictive published-thread limit is not a pre-construction
defence. The exact selected slot depends on prior system-wide thread creation.

**Correction:** provide fallible user-thread construction and propagate
failure through the syscall without kernel panic. Admit thread/stack resources
before construction and retain them in one preparation owner. Allocate stacks
from an exclusive per-domain virtual reservation that explicit mapping and ELF
layout validation respect. Recycle slots only after their mapping and thread
lifetimes complete; use checked arithmetic and bounded slot metadata.

**Acceptance:** an already occupied next stack slot, physical/table allocation
failure and exhausted thread admission must reject without stopping an LP or
altering the pre-existing mapping. Verify initial-thread launch rollback,
concurrent spawns, slot reuse and both architecture paths. A signed ELF whose
layout conflicts with stack reservations must fail before launch mutation.

### SEC-20 — A numeric TID grants foreign-thread observation authority

**Confirmed authorization omission; Medium.**

The EL0 watch syscall passes the caller's ASID only to completion allocation.
Target resolution receives a TID, optional generation and observer, but no
caller identity. It checks existence and generation; it never checks that the
target belongs to the caller or that observation was delegated.

The ABI permits zero generation, which disables generation matching. An
application can therefore observe the lifetime of another domain's or a
kernel thread by its numeric TID. Generation-aware completion ownership
protects subscription lifetime, but does not authorize the target.

Evidence:

- [syscall/mod.rs:966](../../../crates/catten/src/syscall/mod.rs#L966).
- [completion/mod.rs:1270](../../../crates/catten/src/completion/mod.rs#L1270):
  caller-sponsored completion creation followed by unscoped target lookup.
- [scheduler/mod.rs:514](../../../crates/catten/src/cpu/scheduler/mod.rs#L514):
  only generation matching gates `thread.try_observe_exit`.
- [exit_source.rs:35](../../../crates/catten/src/cpu/scheduler/threads/exit_source.rs#L35)
  and [watch_budget.rs:15](../../../crates/catten/src/completion/watch_budget.rs#L15):
  all registrations share a target-source ceiling of 128.

Besides lifecycle disclosure, unrelated clients can occupy the target's
bounded source slots and deny a legitimate join/watch registration. Per-caller
and node charges do not partition this target ceiling. Exhausting 128 slots
may require multiple domains: the ordinary loader completion capacity also
limits each caller. No single-default-domain saturation or inevitable victim
termination is claimed. This does not grant the ability to kill or control
the watched thread.

**Correction:** restrict the EL0 operation to the caller's exact domain
generation, or require a delegated thread-observation owner. Keep a distinct
trusted kernel adapter for legitimate supervisor cross-domain observation.
Make the authorization decision under the same thread-table serialization as
target validation and registration. Retain owned, cancellable subscriptions.

**Acceptance:** raw watch calls naming foreign user/kernel TIDs, with both zero
and nonzero generation, must reject without consuming their source slots.
Same-domain joins and trusted supervisor observation must still work. Test
TID/domain reuse, target exit during registration and foreign-source pressure.

### SEC-21 — Socket ownership does not reclaim resources after owner death

**Confirmed remote-resource lifetime gap; High.**

TCP/IP records an authenticated `(ASID, generation, principal)` owner and
enforces that identity on socket use. Its sweeper, however, follows socket
protocol state and explicit `OP_CLOSE`, not the client's domain lifetime.
Fresh, unactivated sockets deliberately remain indefinitely. Open UDP sockets
and listening TCP sockets likewise need a close or other terminal transition.

The service has no owner-death subscription or exact-generation liveness
reclamation. Kernel teardown cannot run an exited application's Rust
`OwnedSocket::Drop` or infer cleanup for the service's scalar socket ID. After
owner death, successor generations cannot close the retained socket because
the ownership check correctly refuses them. Retention therefore survives the
client's departure and can accumulate across legitimate relaunches.

Creation also installs the socket before replying and ignores reply failure.
A creation whose call has been cancelled can leave a socket whose ID was
never observed. Client-side RAII cannot close an ID it never received.

Evidence:

- [tcpip.rs:146](../../../crates/catten-services/src/bin/tcpip.rs#L146):
  exact occupancy ownership.
- [tcpip.rs:300](../../../crates/catten-services/src/bin/tcpip.rs#L300):
  quotas and operations compare the complete owner.
- [tcpip.rs:759](../../../crates/catten-services/src/bin/tcpip.rs#L759),
  especially line 811: reaping requires activated/closing protocol state.
- [tcpip.rs:915](../../../crates/catten-services/src/bin/tcpip.rs#L915),
  especially lines 978–993: publication in service state precedes an unchecked
  scalar reply.
- [tcpip.rs:1330](../../../crates/catten-services/src/bin/tcpip.rs#L1330):
  explicit client close is the normal resource-release trigger.
- [lib.rs:635](../../../crates/charlotte-launch/src/lib.rs#L635): default
  shared capacity is 64 and the per-owner ceiling is also 64, with two MiB of
  per-owner buffer allowance. TCP buffers consume 32 KiB per socket. Apart
  from DHCP's dedicated protocol slot, ordinary clients and platform clients
  share application-socket admission.

Consequently one authorized client can fill the currently available default
application pool, and its termination need not restore capacity. New platform
connections/listeners can then be refused until TCP/IP is restarted or another
explicit recovery mechanism is introduced. Existing sockets may continue to
work. Lowering tenant quotas reduces the single-client impact but does not
correct retained dead-generation resources or aggregate relaunch exhaustion.

This is separate from SEC-05's repaired raw-ingress authorization and SEC-06's
bounded handling of an individual HTTP peer. It is related to SEC-07's broader
shared-resource progress requirement.

**Correction:** retain an owning client/session registration that reclaims
every socket when the exact domain/session ends, including forced abort.
Handle failed creation reply by consuming the newly created socket owner.
Provide bounded idle admission for unactivated sockets, separate platform
progress capacity, and quotas that cannot let one tenant occupy the whole
shared application pool. Avoid reclaiming a live successor by numeric ASID.

**Acceptance:** cancel creation before observation, terminate a client holding
fresh/listening/UDP sockets, and relaunch repeatedly. Socket/buffer counts must
recover without TCP/IP restart. Old IDs must remain unusable by successors;
unrelated management/connector clients must still establish sockets during
tenant pressure. Verify explicit close and graceful TCP drain remain correct.

### SEC-22 — Every TCP/IP launch repeats public pseudo-random state

**Confirmed configuration weakness; Medium, conditional network impact.**

TCP/IP initializes smoltcp with the same literal random seed on every launch.
It also starts its locally allocated ephemeral ports at the same value and
increments them deterministically. These are the normal service paths, not
test-only overrides.

Evidence:

- [tcpip.rs:555](../../../crates/catten-services/src/bin/tcpip.rs#L555): fixed
  `Config::random_seed`.
- [tcpip.rs:289](../../../crates/catten-services/src/bin/tcpip.rs#L289) and
  [tcpip.rs:611](../../../crates/catten-services/src/bin/tcpip.rs#L611):
  sequential ephemeral-port allocator and fixed initial port.
- The resolved `smoltcp` **0.13.1** source was inspected in the local Cargo
  cache: `iface/interface/mod.rs` initializes `Rand` from that seed;
  `socket/tcp.rs::random_seq_no` uses `cx.rand().rand_u32()` in non-test builds;
  `rand.rs` implements a deterministic generator. The version/checksum is
  recorded in the committed [Cargo.lock](../../../Cargo.lock).

Given equivalent protocol history, nodes/restarts repeat generator output.
Public initial state and predictable ports weaken the uncertainty normally
available against blind TCP injection or reset attempts, and repeat sequence
state across restart. Successful spoofing still depends on routing, the tuple,
traffic history and receiver checks. No stream hijack or TLS bypass was
demonstrated; TLS and connector authentication must remain independently
enforced. This finding does not concern the entropy-backed TLS RNG wrapper.

**Correction:** obtain fresh entropy before constructing the interface and
choose a randomized ephemeral-port start/selection policy. Define startup
failure behavior when entropy is unavailable. Keep deterministic seeds behind
an explicit test configuration. Evaluate sequence-number generation against
the deployment's off-path attacker model; a randomly seeded noncryptographic
generator should not be presented as a complete injection defence.

**Acceptance:** independent service launches have different seeds and initial
wire sequence/port state under equivalent traffic. Verify the normal build
does not use a fixture constant, test builds remain reproducible, and entropy
failure cannot silently restore the public default.

## Reassessment of earlier findings

These judgments concern the reviewed paths, not a certification that every
original acceptance case has been executed.

| Existing IDs | Renewed assessment |
| --- | --- |
| SEC-01 | The identified shared-root mapping path is corrected: raw ABI range rejection precedes normalization; complete object ranges and architecture walkers also check the user window. ELF layout validation uses that contract. Stack-region ownership is the distinct SEC-19 gap. |
| SEC-02 | The identified policy-substitution path is corrected: grantctl hashes supplied policy, requires controller-only kernel attestation for the authenticated occupancy, and checks the principal and declared rights. |
| SEC-03 | Queues and mailbox authority are domain-local, with bounded records and teardown. No reappearance of the old global application channel was found in the reviewed path. |
| SEC-04 | Development trust is explicit and production/unknown build modes reject. Protected production bootstrap trust and recipient custody remain unavailable. This is fail-closed mitigation, not production provisioning. |
| SEC-05 | Raw TCP/IP ingress requires the exact live designated router generation. Socket bind/listen policy remains broad; SEC-21–22 expose separate stack risks. |
| SEC-06 | Per-connection error handling and total receive budgets are present. Serial admission/flood resistance remains open. Socket pool exhaustion adds another availability dependency. |
| SEC-07 | Many aggregate resource owners and platform reserves are implemented. Translation tables, stacks, kernel heap and general metadata/callback storage remain incompletely admitted. See the concrete table-pressure path below. |
| SEC-08 | Cryptographic node/control/data authentication and replay protection remain open. Trusted L2 remains a prerequisite. |
| SEC-09 | Security-time provenance/freshness remains open. SNTP/holdover and a nonzero/synchronized timestamp are insufficient security-policy evidence. |
| SEC-10 | Management authentication/encryption remains open. Signed write payloads do not authenticate every reader or transport peer. |
| SEC-11 | Scoped applications use configured roots and launch attestation. Independent-root scoped launch passes; the complete remote release pipeline and production platform trust remain separate assurance work. |
| SEC-12 | Local object-store CALL authority remains store-wide. Do not infer tenant isolation from IDs or typed owners. |
| SEC-13 | In-repository signing calls use restricted key paths and reject secret-valued legacy configuration. Host custody and sibling callers remain outside this review. |
| SEC-14 | Main runners enforce the lockfile; CI actions are commit-pinned with read-only permissions and no persisted checkout credentials. This audit adds a clean advisory scan, but CI advisory enforcement, licensing and release provenance remain open. |
| SEC-15 | Reviewed SigV4 temporaries and TLS record buffers have zeroizing owners. Complete crypto-state/compiler-copy erasure is not established. |
| SEC-16 | Bounded concurrently polled grant operations replace the identified blocking lookup. Guest scoped publication/acquisition and cancellation traffic pass. Many-client fairness/controller replacement remain unverified. |
| SEC-17 | Both walkers retain empty private tables through root teardown, correcting dynamic table recycling. Lifetime correctness must be preserved when implementing admission; do not restore premature reclamation. |
| SEC-18 | Owning retirement, root leases and prepared reply/cancellation claims substantially improve the reviewed explicit paths. Bulk cleanup, recipient progress, physical failure recovery and full CPU/DMA quiescence remain partial. |

### SEC-07: retained sparse tables still bypass domain backing ceilings

One memory object can be mapped and unmapped at successively sparse user
addresses. Each previously unused translation subtree may require fresh table
frames. Empty tables intentionally remain linked after unmap. Data-page,
object, heap and image ceilings do not charge those retained table frames.
Thus a small live data allocation can sponsor an increasingly large private
translation hierarchy over time.

[translation.rs:15](../../../crates/catten/src/memory/translation.rs#L15)
enforces a physical progress floor for private tables, rather than a
domain/node translation budget. Both user roots and private intermediate
tables use that policy. The floor limits outright physical depletion; it does
not stop one tenant from denying further private-table/root preparation to
other domains when it is reached. Platform resource pools cannot by themselves
make that separate translation allocator admit another private root.

This is a concrete remaining SEC-07 path, not a new finding ID or a claim that
the retention fix is wrong. Add lifetime-owned table admission, captured domain
identity and platform progress policy before linking each private table. Charge
partial construction and retained empty tables until their actual teardown.
Test sparse-map churn, rejection/retry, concurrent domains and essential-domain
launch under pressure. No physical-pressure attack was executed in this audit.

### SEC-18: explicit ownership is not yet complete bulk quiescence

`PreparedReply` and `PreparedCancellation` retain both roots and loan receipts
across unlocked cleanup. Failure fences delivery/result publication; runtime
pending-call close preserves its borrow owner or uses the fatal-domain boundary
when Drop cannot return ownership. Those protections should be preserved.

However, [ipc/mod.rs:2003](../../../crates/catten/src/ipc/mod.rs#L2003) still
contains serialized endpoint bulk cleanup, and
[ipc/mod.rs:2190](../../../crates/catten/src/ipc/mod.rs#L2190) uses its non-leasing
domain-cleanup adapter. In particular,
[ipc/mod.rs:2313](../../../crates/catten/src/ipc/mod.rs#L2313) still ignores
revocation errors in bulk pending-call cancellation. These paths do not acquire
the new explicit cancellation owner. Whole-domain device teardown also retains
its outer lifecycle boundary. Existing quarantine reduces unsafe reuse; it
does not establish recoverable cleanup, complete progress or physical quiescence.

Finish bulk operation ownership and failure propagation before treating root
drain as proof that all subsystems are physically retired. Do not simply call
the live-leasing adapter while already holding lifecycle/IPC. Include true
multi-LP close/reply/cancel stress, real fatal-domain cleanup, x86 recipient
progress/failure and device reset/quiescence in acceptance work.

## Validation performed in this audit

| Check | Result and limits |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed, including the 18 direct slot-owner tests, six scratch tests, 33 runtime tests, protocol/authorization suites, signing-policy checks and cluster-sign self-test. |
| Bundled AArch64 services | Rebuilt and signed through `scripts/build-catten-services.sh` as invoked by the guest runner; no production service source changes. |
| Fresh AArch64 security guest | **19 passed, 0 failed, 0 pending**. Both scoped probes reported `0x7fff`; publication generations advanced to 1 and 2. Concurrent cancellation traffic retired after **4,400 requests**. |
| Committed dependency graph | `cargo-audit 0.22.2`, **177 dependencies**, **0 reported vulnerabilities**, no informational warnings. Advisory database had 1,290 advisories at commit `ef6173cbc5c50ec8166f9a5b28f07834144373ee`, updated `2026-10-03T10:14:03+02:00`. This is a database-relative result, not proof of dependency safety. |

The first QEMU launch failed before boot because the sandbox could not bind the
forwarding port. The approved isolated rerun succeeded; this was not a guest
security failure. No existing guest or store was stopped or reset.

Successful guest command:

```sh
CATTEN_HTTP_HOST_PORT=18185 CATTEN_DEPLOY_HOST_PORT=17485 \
  scripts/run-aarch64.sh --security-test \
  --instance renewed-audit-20261005-approved --fresh-storage --timeout 100
```

Kernel SHA-256:
`763cb1b8ddd7a1d5b77e37462ca8ffe5b86deb16d4656fcb811bc7fec24f49a1`.
Lockfile SHA-256:
`90db6ee790336456e6a4f9c40d1951476b0b91f30e690ce8c6bbb8c7f44460cc`.

Local logs are `/private/tmp/charlotte-renewed-security-audit-guest-approved.log`,
`/tmp/charlotte-renewed-audit-20261005-approved-serial.log`, and
`/private/tmp/charlotte-renewed-security-audit-dependencies.json`.
The [dependency scan result](2026-10-05-security-audit-dependencies.json) is
retained alongside this report. Temporary guest logs may be removed later.
No new x86 build/Clippy result, x86 guest result, hardware attack result or TLC
run is claimed by this report.

The normal guest suite passes despite SEC-19–22 because it does not include
those negative cases. Its deterministic cleanup fixtures also do not establish
all newly split paths' behavior under simultaneous production scheduling.

## Recommended order

1. Correct SEC-19's fallible thread/stack admission and layout ownership. Kernel
   panic must not be a normal response to an application-controlled collision.
2. Reclaim remote socket resources on exact owner/session death and failed
   publication (SEC-21), with real shared-stack progress reserves.
3. Restrict foreign-thread watch authority (SEC-20), and replace deterministic
   TCP/IP initialization (SEC-22).
4. Complete table/stack/metadata admission (SEC-07) and bulk physical-retirement
   ownership/quiescence (SEC-18), with concurrent and x86 execution evidence.
5. Complete protected production trust, authenticated security time,
   management access and node traffic before changing the original isolated,
   trusted-workload deployment assumptions.

The revised confidence comes from specific source corrections and executed
regressions. It should not be expanded into a production or multi-tenant
security claim while the remaining boundaries are open.
