# CharlotteOS renewed security audit — 2026-10-10

This follows the [2026-10-05 renewed audit](2026-10-05-security-audit.md), its
[remediation](2026-10-05-security-remediation.md) and the
[SEC-23–25 follow-up](2026-10-05-security-follow-up.md). It reviews revision
`2221b5bfa0ce6f303cfe4e07f6c8602060adfbfa` (2026-10-10, `main`), after the
60-commit SEC-07/18 ownership and admission campaign. The working tree was
clean when review began. No production code was changed during this audit.
Corrections and validation for the new findings are not implemented here; they
are tracked in the [remediation ledger](../../reference/security-remediation.md).

## Assessment

The ownership campaign is real and largely holds up: the previously corrected
application boundary paths remain corrected, admitted storage now follows most
resources through publication and retirement, and the AArch64 security guest
passes all 19 tests on this revision. Source review of the corrected SEC-01,
SEC-19, SEC-20, SEC-23 and SEC-24 paths found no regression.

The remaining risk has moved outward, to the services and launch gates that
consume untrusted input. Three new High findings are confirmed in source:

- An **unauthenticated L2 frame** can abort the reliable-message service
  (`relmsg`) by advertising a multi-gigabyte reassembly total in the first
  fragment.
- A **hostile signed application** with a TCP/IP socket can deterministically
  exhaust the TCP/IP service's memory-object budget, after which the next
  transmitted packet panics the service on a zero-length buffer. The failure
  path itself converts any transient transmit-allocation failure into an abort.
- The deployment agent's legacy descriptor-less launch path uses a
  **panic-on-failure kernel wrapper**, turning a recoverable launch admission
  failure into a kernel panic.

Four lower findings concern a missing AArch64 syscall emitter that aborts the
deployment agent, platform progress reserve granted to descriptor-less
artifacts, an unbounded discovery peer table keyed by attacker-chosen MAC, and
a legacy name-service lookup that resolves policy-published registrations
without an authorization decision.

None of these requires defeating artifact signatures. They are availability
and policy-boundary failures within the documented threat model, which already
includes a compromised admitted application and a hostile L2 peer. The four
original open findings (SEC-08, 09, 10, 12) and the partial SEC-04/05/06/07/11/18
remain the primary production blockers; the new findings concentrate on the
same services and launch boundaries those gaps expose.

## Scope and method

- Read the [remediation ledger](../../reference/security-remediation.md),
  [cleanup/recovery strategy](../../reference/cleanup-recovery.md), the
  SEC-19–30 reports and the prior audit reassessments; cross-checked current
  contracts against source.
- Traced the EL0 syscall dispatcher, capability publication and detachment,
  mailbox word/endpoint paths, IPC cancellation and endpoint close, memory
  object mapping, thread/stack admission, the ELF/launch gates and the boot
  trust handoff.
- Reviewed the network-facing services (`tcpip`, `socket_state`,
  `charlotte-smoltcp`, `relmsg`, `disco`, `ns`, `raft` transport), cluster
  protocol parsing and framing, and the launch/trust tooling. Four parallel
  review passes covered the network stack, ELF/launch paths,
  cluster/authorization protocols, and the `catten-rt::owned` wrappers; every
  High/Medium claim below was then re-verified directly by the author against
  the exact lines cited.
- Ran the workspace host suites, the AArch64 security guest, formatting, both
  kernel Clippy configurations and a fresh advisory scan. Details and artifact
  hashes are in [Validation performed](#validation-performed).
- No x86 guest execution, physical-device experiment, sustained multi-LP
  hostile stress, destructive kernel fix, or end-to-end exploit of the new
  findings was performed. The new findings are source-confirmed, with
  reachability and impact qualified below.

The threat model is unchanged: compromised admitted applications,
unauthenticated network clients, a hostile L2 peer, compromised platform
services/connectors, malicious devices and a compromised build host. Kernel,
supervisor, name service, grant controller, deployment agent and
credential-holding connectors are trusted. Peer consensus remains
non-Byzantine under its documented deployment assumptions.

Severity follows the prior audits: High is a substantial containment or
shared-availability failure; Medium is a narrower or conditional
authorization/network exposure or a policy-boundary gap; Low is defence in
depth. “Confirmed” means the relevant path was established in source; it does
not mean an end-to-end exploit was executed.

## New findings

| ID | Severity | Finding | Required access or condition |
| --- | --- | --- | --- |
| SEC-31 | High | Unauthenticated L2 frame aborts `relmsg` through an oversized reassembly total | Any peer on the cluster L2 segment |
| SEC-32 | High | Transmit-allocation failure panics TCP/IP; a client can deterministically exhaust the service object budget | TCP/IP socket capability (signed application) or allocation pressure |
| SEC-33 | High | Descriptor-less artifact launch panics the kernel on a recoverable launch failure | Deployment-agent launch plus admitted resource pressure |
| SEC-34 | Medium | Missing AArch64 `svc #77` emitter aborts the deployment agent | Operational-connector deployment on AArch64 |
| SEC-35 | Medium | Descriptor-less artifacts receive platform progress reserve | Legacy deployment-agent launch path |
| SEC-36 | Medium | Discovery peer table is unbounded and keyed by an attacker-chosen MAC | Any L2 peer sending discovery frames |
| SEC-37 | Medium | Legacy name-service lookup resolves policy-published registrations without an authorization decision | Signed application with a name-service connection |
| SEC-38 | Low | `SPAWN_UPGRADE` snapshot length is not bounded by the artifact ceiling | Live-upgrade manager with an oversized memory object |

### SEC-31 — One remote frame can abort the reliable-message service

**Confirmed remote availability defect; High.**

The reliable-message service stores the fragment's advertised
`total_message_len` in per-peer reassembly state before any ceiling check, then
calls the assembler unconditionally. The assembler allocates the advertised
total with an infallible allocation:

- [relmsg.rs:386](../../../crates/catten-services/src/bin/relmsg.rs#L386):
  a new `Reassembly` is created with `total: total_message_len` when the
  fragment sequence is expected.
- [relmsg.rs:395](../../../crates/catten-services/src/bin/relmsg.rs#L395):
  `within_ceiling` is computed afterwards and only gates fragment buffering.
- [relmsg.rs:423](../../../crates/catten-services/src/bin/relmsg.rs#L423):
  `try_assemble` runs even when `within_ceiling` is false.
- [relmsg.rs:116](../../../crates/catten-services/src/bin/relmsg.rs#L116):
  `Vec::with_capacity(total)` before any fragment-presence check.

The protocol permits the advertised total: a `SYN|FRAG|MORE` frame with
`offset=0`, `payload_len=1` and `total_message_len=0xFFFF_FFFF` satisfies
`end <= total` and `MORE == (end < total)`
([charlotte-protocol-msg/src/lib.rs:344](../../../crates/charlotte-protocol-msg/src/lib.rs#L344)).
The frame router dispatches non-IP frames to the registered EtherType route
without a destination-MAC check
([frouter.rs:1104](../../../crates/catten-services/src/bin/frouter.rs#L1104),
[1106–1145](../../../crates/catten-services/src/bin/frouter.rs#L1106)); the
victim MAC is discoverable from discovery traffic. `relmsg::MAX_MSG` is 1 MiB
([lib.rs:1402](../../../crates/catten-services/src/lib.rs#L1402)), so the
advertised total is far beyond any feasible service heap.

**Impact:** the first such frame aborts `relmsg`; the DNS/Raft transport over
`relmsg` and all reliable messaging lose their service until it is restarted.
The attack is unauthenticated, repeatable, and needs no prior session or
valid peer state.

**Correction:** bounds-check `total_message_len` before creating or updating
reassembly state; only run the assembler under `within_ceiling`; use fallible
reservation (`try_reserve`) and drop the fragment on rejection.

**Acceptance:** an over-ceiling fragment must leave the service running,
retain no reassembly state, and not pre-allocate memory. A framed
`total=0xFFFF_FFFF` probe and an exact-`MAX_MSG` success case must both pass.

### SEC-32 — Transmit failure panics TCP/IP and is client-triggerable

**Confirmed remote/client availability defect; High.**

The smoltcp TX token returns an **empty buffer** whenever the transmit scratch
allocation or mapping fails, but smoltcp requires the closure to receive
exactly `len` bytes:

- [charlotte-smoltcp/src/lib.rs:215](../../../crates/charlotte-smoltcp/src/lib.rs#L215):
  `len > 4096`, `memory_alloc` failure and `memory_map` failure all call
  `f(&mut empty)`.
- smoltcp 0.13.1 (`Cargo.lock`) `dispatch_ethernet` calls
  `tx_token.consume(tx_len, …)`, wraps the buffer in
  `EthernetFrame::new_unchecked` and calls `set_src_addr`, which indexes the
  zero-length slice and panics. `emit_ip` writes further into the same buffer.
- The transmit path allocates the scratch object from the service's own
  per-domain object budget:
  [tcpip receive path](../../../crates/catten-services/src/bin/tcpip.rs#L769)
  allocates one object per delivered chunk and
  [moves it to the client](../../../crates/catten-services/src/bin/tcpip.rs#L794).
  The creating generation sponsors the allocation for its entire lifetime
  ([memory/budget.rs](../../../crates/catten/src/memory/budget.rs#L1)); the
  per-domain object count ceiling is 1024
  ([budget.rs:24](../../../crates/catten/src/memory/budget.rs#L24)).

A signed application holding a socket can call `OP_RECV` repeatedly, retain
the returned one-page memory objects without closing them, and exhaust
TCP/IP's 1024-object budget (about 4 MiB of pages). Every subsequent
`memory_alloc` in the service fails, including the next smoltcp transmit
(ACK, retransmit, DHCP or response), which then panics. The documented
cross-domain-charge limitation
([smoltcp-adapter.md:162](../../reference/smoltcp-adapter.md#L162)) explains
the retained charge; it does not justify converting a dropped packet into a
service abort.

**Impact:** any admitted application can take down the node's TCP/IP service
and all network-backed platform services until restart. Independently,
ordinary physical/budget exhaustion reaches the same panic, so the failure
path is unsafe even without a hostile client.

**Correction:** fail closed at the `Device` level, not inside `TxToken`:
reserve and map the transmit buffer in `transmit()` and return `None` (packet
drop) on failure, or keep a preallocated device-owned TX buffer. Account for
moved receive backing against a bound the receiver cannot retain without
limit, or cap retained receive objects per owner. Add a test that injects
allocation rejection into the TX path.

**Acceptance:** transmit allocation failure must drop the packet and leave the
service alive; a client exhausting its receive retention must not starve the
service's TX path; a regression test must exercise both.

### SEC-33 — Descriptor-less artifact launch panics the kernel

**Confirmed application-triggered kernel panic path; High (requires the
deployment agent's legacy launch).**

`SPAWN_ARTIFACT` consumes a loaded domain through `start_domain`, the
mandatory-launch wrapper that panics on any admission failure:

- [syscall/mod.rs:2238](../../../crates/catten/src/syscall/mod.rs#L2238)
  calls `start_domain(loaded)`.
- [supervisor.rs:394–404](../../../crates/catten/src/service/supervisor.rs#L394)
  implements `start_domain_with_limits` as
  `.expect("mandatory service launch failed")` over the fallible
  `try_start_domain_with_limits`.
- The fallible path fails on thread/stack admission, profile limits or
  physical backing availability
  ([supervisor.rs:331](../../../crates/catten/src/service/supervisor.rs#L331)).

The deployment agent's legacy branch launches descriptor-less records through
this syscall ([agent.rs:518](../../../crates/catten-services/src/bin/agent.rs#L518),
[agent.rs:526](../../../crates/catten-services/src/bin/agent.rs#L526)). Any
running application can create memory/thread pressure; a legacy deployment
performed under that pressure then stops an LP instead of returning an error.
The scoped `SpawnArtifactScoped` path already uses the fallible wrapper and is
not affected.

**Impact:** kernel panic (with `panic = "abort"`, the initiating LP halts and
the panic handler masks interrupts) from a recoverable, caller-visible launch
failure. This is the SEC-19 failure class in the deployment boundary.

**Correction:** use `try_start_domain_with_limits` in `sys_spawn_artifact`
and tear down/close the loaded domain on rejection; reserve the "mandatory"
panic wrapper for boot-time trusted launches only.

**Acceptance:** injected thread-admission or limits rejection during
`SPAWN_ARTIFACT` must return an error and leave the pre-existing domain and
LP intact.

### SEC-34 — AArch64 has no emitter for `SpawnOperationalConnector`

**Confirmed userspace availability/config defect; Medium.**

The wrapper for syscall 77 exists and the kernel implements it, but the
manual AArch64 `svc3` emitter table has no arm for 77, so the wrapper panics:

- [catten-syscall/src/lib.rs:1966–1979](../../../crates/catten-syscall/src/lib.rs#L1966)
  calls `svc3(SyscallNumber::SpawnOperationalConnector, …)`.
- The `svc3` match
  ([lib.rs:396–451](../../../crates/catten-syscall/src/lib.rs#L396)) falls
  through to `panic!("syscall {:?} has no svc3 emitter", imm)`; `svc #77`
  appears nowhere in the crate.
- The deployment agent reaches it through
  [catten-rt owned ipc](../../../crates/catten-rt/src/owned/ipc.rs#L113) and
  [agent.rs:646](../../../crates/catten-services/src/bin/agent.rs#L646), and
  is built for AArch64.

**Impact:** every operational-connector deployment on AArch64 aborts the
deployment agent instead of returning `ArtifactLaunchError`. The failure is
fail-closed (no connector is launched) but denies the documented deployment
path and aborts a trusted platform service. This is the same emitter-table
class caught for syscall 85 during SEC-23–25 remediation
([follow-up](2026-10-05-security-follow-up.md)); the table is manual and has
regressed again.

**Correction:** add the `77` arm and add an automated check that every
`SyscallNumber` passed to `svc3`/`svc4` has an emitter (or generate the stubs
from `define_syscall_numbers!`).

**Acceptance:** an AArch64 connector deployment must reach the kernel gate and
return a result rather than aborting the agent.

### SEC-35 — Descriptor-less artifacts receive platform progress reserve

**Confirmed resource-policy defect; Medium.**

The launch path classifies a domain as platform when its authority has no
launch-descriptor digest:

- [supervisor.rs:337–343](../../../crates/catten/src/service/supervisor.rs#L337):
  `if … launch_descriptor_digest.is_none() { mark_platform(…) }`, with the
  comment “Only the supervisor's ambient platform launch path receives
  progress reserve.”
- The digest is only installed by scoped descriptor launches
  ([memory/mod.rs:161](../../../crates/catten/src/memory/mod.rs#L161)).
- `SPAWN_ARTIFACT`
  ([syscall/mod.rs:2165–2252](../../../crates/catten/src/syscall/mod.rs#L2165))
  launches through the same path with no descriptor, so the deployed artifact
  inherits the platform classification.

**Impact:** an untrusted signed artifact deployed through the legacy path can
consume the platform progress reserve intended for essential services
(SEC-07 R7-4), weakening progress guarantees under pressure. The artifact is
still signed, so this is a policy/accounting misclassification rather than an
authority bypass.

**Correction:** derive platform classification from trusted launch provenance,
not from the absence of a descriptor; legacy agent launches should be
ordinary.

**Acceptance:** a descriptor-less artifact launch must not change the
domain's platform classification or reserve eligibility.

### SEC-36 — Unbounded discovery peer table keyed by attacker-chosen MAC

**Confirmed unauthenticated exhaustion defect; Medium.**

Discovery inserts a peer record for every accepted frame, keyed by the
*source MAC in the payload*, without a volume bound and without comparing it
to the frame's real Ethernet source:

- [disco.rs:502](../../../crates/catten-services/src/bin/disco.rs#L502)
  calls `learn_peer` for every decoded frame whose advertised cluster id
  matches; only the `FLAG_PROBE` response path is conditional.
- [disco.rs:446–469](../../../crates/catten-services/src/bin/disco.rs#L446)
  inserts unconditionally into a `BTreeMap` holding three `Vec`s per entry;
  no `MAX_PEERS` equivalent is used.
- The key comes from payload bytes
  ([charlotte-protocol-disco/src/lib.rs:338](../../../crates/charlotte-protocol-disco/src/lib.rs#L338),
  shown as `frame[18..24]`), which the sender controls; the cluster id is
  broadcast in every probe.
- The discovered raft ids propagate into the raft transport peer map
  ([raft.rs:778–791](../../../crates/catten-services/src/bin/raft.rs#L778),
  [relmsg_transport.rs:206](../../../crates/catten-services/src/relmsg_transport.rs#L206)).

**Impact:** an L2 peer can grow `disco` (and the DNS/Raft transport maps)
until heap exhaustion aborts the service, without spoofing the real Ethernet
source. This is a concrete SEC-08/SEC-07 exhaustion vector: discovery has no
peer authentication by design, but it needs a volume bound and MAC/ID
consistency check.

**Correction:** cap peer entries and per-tick churn, reject records whose
advertised MAC is inconsistent with the frame source where possible, and
bound transport peer maps.

**Acceptance:** a sustained stream of distinct-MAC discovery frames must not
grow service memory without limit or abort the service.

### SEC-37 — Legacy lookup resolves policy-published registrations

**Confirmed authorization-boundary gap; Medium
(compatibility behavior documented in the architecture).**

Authorized registrations are stored with `access_key = 0`, and the legacy
lookup path treats zero as “no key required”, returning a `SEND|CALL`
connection without any policy decision or audit record:

- [ns.rs:908–933](../../../crates/catten-services/src/bin/ns.rs#L908)
  publishes through `OP_REGISTER_AUTHORIZED` with `access_key` 0.
- [ns.rs:617–640](../../../crates/catten-services/src/bin/ns.rs#L617)
  (`lookup_or_defer`) denies only when `access_key != 0` and mismatched, then
  [reply_connection_or_error](../../../crates/catten-services/src/bin/ns.rs#L229)
  delegates `SEND|CALL`.
- [ns.rs:786](../../../crates/catten-services/src/bin/ns.rs#L786) routes the
  public `OP_LOOKUP` opcode here with caller key 0.

The [authorization policy](../../architecture/authorization-policy.md)
explicitly states that legacy opcodes are not an authorization boundary and
that security-sensitive clients must use the authorized opcodes. The defect
is therefore a known compatibility surface, but it still means any signed
application that knows a protected service's name can obtain a connection to
it without passing `OP_LOOKUP_AUTHORIZED`, and the service has no way to
distinguish that connection from a policy-approved one.

**Correction:** refuse to resolve `authorized` registrations through legacy
opcodes (or route them through `authorize_now`), so policy-published services
are reachable only through the mediated path; keep a trusted kernel/service
adapter for legitimate internal callers.

**Acceptance:** a legacy lookup naming a policy-published service must be
denied and audited unless the caller passes the authorized path.

### SEC-38 — `SPAWN_UPGRADE` snapshot has no artifact-ceiling bound

**Confirmed bounded-resource defect; Low (manager-only).**

`SPAWN_UPGRADE` snapshots a caller-sized ELF through the kernel heap without
the artifact ceiling that `SPAWN_ARTIFACT` enforces, and `snapshot_bytes`
allocates infallibly:

- [syscall/mod.rs:2065–2069](../../../crates/catten/src/syscall/mod.rs#L2065)
  takes `size` from the caller and calls `snapshot_bytes`.
- [object.rs:547](../../../crates/catten/src/memory/object.rs#L547) does
  `Vec::with_capacity(len)` while holding the object registry lock; the only
  bound is the source object's page capacity.
- `MAX_ARTIFACT_ELF_SIZE` is 4 MiB
  ([charlotte-launch/src/lib.rs:57](../../../crates/charlotte-launch/src/lib.rs#L57));
  no equivalent check exists on this path.

**Impact:** the trusted live-upgrade manager can force multi-megabyte kernel
heap allocations (up to the 64 MiB per-domain object ceiling) that can abort
the kernel under heap pressure. No unprivileged caller reaches it today.

**Correction:** enforce the artifact size ceiling before snapshotting and use
fallible allocation, retaining the object on rejection.

**Acceptance:** an oversized upgrade object must be rejected without a kernel
allocation of its advertised size.

## Reassessment of earlier findings

Judgments marked “re-verified” come from direct source review at this
revision; the others retain the prior records and were not re-executed here.
This is not a certification that every acceptance case has been exercised.

| Existing IDs | Renewed assessment |
| --- | --- |
| SEC-01 | Corrected; **re-verified**. `object::map` checks `valid_application_pages` before mapping ([object.rs:767](../../../crates/catten/src/memory/object.rs#L767)) and `map_locked` rechecks the complete object range under the registry ([object.rs:847](../../../crates/catten/src/memory/object.rs#L847)); the shared contract excludes the stack arena ([user_address.rs:23](../../../crates/charlotte-launch/src/user_address.rs#L23)). |
| SEC-02 | Corrected in the documented scope; prior evidence retained. Scoped grants are checked at the controller ([tests pass](#validation-performed)); not re-reviewed line by line. |
| SEC-03 | Corrected; **re-verified at the dispatch boundary**. Mailbox endpoints are per-ASID namespaces with exact root/identity capture ([mailbox_publication.rs:178](../../../crates/catten/src/syscall/mailbox_publication.rs#L178)), and capability lookups check owner, kind and live state ([capability.rs:607](../../../crates/catten/src/capability.rs#L607)). |
| SEC-04 | Partial/open as recorded. The boot path still installs the explicit development fixture ([launch.rs:956](../../../crates/catten/src/service/launch.rs#L956)); production admission has no adapter and remains disabled. Signed-policy tooling and the one-shot handoff exist; protected provisioning/custody do not. |
| SEC-05 | Partial; **re-verified**. Raw ingress remains gated on the designated router, but socket `bind`/`listen` still accepts arbitrary local IPv4 addresses and ports ([tcpip.rs:255](../../../crates/catten-services/src/bin/tcpip.rs#L255)), so a signed app can claim a service VIP. |
| SEC-06 | Partial. The TCP/IP reactor is bounded to 16 requests and one ring per cycle ([lib.rs:58](../../../crates/charlotte-smoltcp/src/lib.rs#L58)); SEC-32 shows an abort path in the same service; flood fairness for management remains open. |
| SEC-07 | Partial. The admission-owner inventory is broad, but SEC-32 and SEC-36 identify concrete unbounded/incorrectly-lifetimed allocations; R7-1–R7-5 are not met. |
| SEC-08 | Open; **re-verified** as the precondition for SEC-36. L2 peer identity is still spoofable, and discovery has no volume bound. |
| SEC-09 | Open. Time remains SNTP-derived, and the operational pickup compares expiry against a caller-supplied `now_unix_seconds` ([syscall/mod.rs:2451](../../../crates/catten/src/syscall/mod.rs#L2451)); no authenticated security time exists. |
| SEC-10 | Open. No management client authentication was added. |
| SEC-11 | Partial; **re-verified**. Scoped launches use the installed trust, but the legacy `SPAWN_ARTIFACT`/`SPAWN_UPGRADE` paths still verify against the compiled development key ([syscall/mod.rs:2205](../../../crates/catten/src/syscall/mod.rs#L2205), [supervisor.rs:1400](../../../crates/catten/src/service/supervisor.rs#L1400)); SEC-35 compounds the classification gap. |
| SEC-12 | Open. Legacy object-store authority remains store-wide. |
| SEC-13 | Corrected in the documented scope; prior evidence retained. |
| SEC-14 | Corrected in scope; advisory scan clean this pass (177 dependencies, zero advisories). CI advisory enforcement and release provenance remain separate. |
| SEC-15 | Corrected in scope; prior evidence retained; complete crypto-state erasure is not established. |
| SEC-16 | Corrected in scope; prior evidence retained. |
| SEC-17 | Corrected in scope; prior evidence retained. |
| SEC-18 | Partial as recorded. C01–C18 ownership landed with execution evidence; G1–G7 and the historical user-stack lease timeouts remain open. |
| SEC-19 | Corrected; **re-verified**. `SPAWN_THREAD` uses fallible construction/publication and propagates failure ([syscall/mod.rs:961](../../../crates/catten/src/syscall/mod.rs#L961)); stack slots are an exclusive arena excluded from application mappings. |
| SEC-20 | Corrected; **re-verified**. The EL0 watch uses `observe_own_thread_exit` ([syscall/mod.rs:1018](../../../crates/catten/src/syscall/mod.rs#L1018)), which captures the caller root and authorizes under the thread table ([completion/mod.rs:1295](../../../crates/catten/src/completion/mod.rs#L1295)). |
| SEC-21 | Corrected in the documented scope; **re-verified** designation/ownership checks. SEC-32 identifies a separate retained-backing lifetime gap in the same service. |
| SEC-22 | Corrected; entropy-gated startup and independent port key remain in place. |
| SEC-23 | Corrected; **re-verified**. Only a ring-3 saved `CS` authorizes domain containment; kernel faults still panic ([exceptions/mod.rs:80](../../../crates/catten/src/cpu/isa/x86_64/interrupts/fixed/exceptions/mod.rs#L80)). |
| SEC-24 | Corrected; **re-verified for AArch64**. The trampoline clears all GPRs, SIMD registers and FP control state before `eret` ([ops.rs:504](../../../crates/catten/src/cpu/isa/aarch64/lp/ops.rs#L504)); the x86 FX/FS-GS ownership is per the prior report. |
| SEC-25 | Corrected in the documented scope; bounded reactor and sampled monotonic time are present. |
| SEC-26–30 | Corrected per the prior reports; not re-reviewed in this pass. |

## Validation performed

| Check | Result and limits |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed. All workspace host suites plus the standalone id-table, retirement-list and scratch fixtures reported 336 tests and zero failures; signing-policy, boot-result, secure-boot-result and cluster-sign self-tests passed. |
| Fresh AArch64 security guest | **19 passed, 0 failed, 0 pending**, `passed_bitmap=0x2003ffff`; the scoped probe reported `checks=0xffff`; publication generations reached 2; concurrent cancellation traffic retired after **4872 requests**. |
| AArch64 kernel Clippy | Passed with `-D warnings` (default features, `CATTEN_AARCH64_SERVICE_BUNDLE` set). |
| x86_64 kernel Clippy | Passed with `-D warnings` (`--no-default-features --features acpi`). |
| `cargo fmt --all -- --check` | Passed. |
| Committed dependency graph | `cargo-audit 0.22.2`: **177 dependencies, 0 vulnerabilities**. Advisory database: 1,296 advisories, commit `7eebec69c352c7191b1f13eb95dd510eeca5d1de`, updated `2026-10-09T10:12:02+02:00`. Machine-readable result: [dependency scan](2026-10-10-security-audit-dependencies.json). Database-relative, not proof of dependency safety. |

Successful guest command (embedded bundles reused from a previously built
bundle because only review, not code, changed):

```sh
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18819 CATTEN_DEPLOY_HOST_PORT=18119 \
  scripts/run-aarch64.sh --security-test \
  --instance renewed-audit-20261010 --fresh-storage --timeout 180
```

Validated artifact SHA-256 values:

- AArch64 security-guest kernel:
  `ac41ad98e6a40505b27b3c7ff3eca3c324f0fa920f4c47f33ff9acb91505fadc`
- `Cargo.lock`:
  `6237dbf75d0e29bb7fe1c0dba4301d12d5a298d626fdc89bd05e5d766c50aa76`

The passing guest does not exercise SEC-31/32/33/34/36: the security suite
covers scoped grants, independent roots, cancellation, raw-frame denial and
TCP/IP progress under socket pressure, but sends no adversarial reassembly
header, does not exhaust the service object budget, and does not use the
descriptor-less launch or discovery paths. SEC-31, SEC-32, SEC-33 and SEC-36
are source-confirmed with clear call paths and line-level evidence, not
reproduced crashes. The historical intermittent Intel/AMD user-stack lease
timeouts remain causally unresolved; this audit adds no new evidence on them.

## Recommended order

1. **SEC-32** first: restore fail-safe transmit behavior in `charlotte-smoltcp`
   and bound retained receive backing. A single admitted client must not be
   able to abort the node's network stack.
2. **SEC-31**: validate the reassembly total before state creation/allocation
   so one unauthenticated frame cannot kill `relmsg`.
3. **SEC-33 and SEC-34**: make the deployment-agent launch paths fallible and
   restore the AArch64 emitter, so deployment failures are errors rather than
   kernel panics or agent aborts. **SEC-35** belongs with this change.
4. **SEC-36 and SEC-37**: bound discovery state and stop legacy lookup from
   resolving policy-published services without a decision.
5. Complete the standing production blockers: SEC-07/18 admission and
   quiescence criteria, then SEC-04/08/09/10/12 (protected trust,
   authenticated peer/management channels, security time, object-scope
   attenuation) before any change to the isolated, trusted-workload
   deployment assumptions.

## Limits

This is a boundary review with targeted execution evidence, not a line-by-line
proof of every subsystem, a cryptographic implementation audit, or a
production certification. Reassessments marked “prior evidence retained” rest
on the version-controlled prior reports rather than fresh execution. No x86
guest, physical hardware, DMA-device fault injection, mutating adversarial
test against SEC-31/32/33 or concurrent multi-domain pressure run was
performed. The new findings do not establish memory corruption or privilege
escalation; their impact is service availability, misclassification and
authorization reachability. Source line references refer to the reviewed
revision and may drift after remediation.
