# CharlotteOS security audit — 2026-10-03

Follow-up: [implemented remediations, validation, and remaining work](2026-10-03-security-remediation.md).
Findings below describe the reviewed revision, not the hardened working tree.

## Verdict

CharlotteOS has meaningful security mechanisms, particularly kernel-derived
IPC identity, generation-aware capabilities, signed deployment admission,
connector-only credentials, and DMA pinning. However, the current build is
not ready to host mutually distrustful applications or to expose its management
and cluster networks to untrusted parties.

The most urgent implementation finding is that an application can request an
explicit memory-object mapping in the kernel half of the address space. On
AArch64, this reaches the shared kernel translation-table root. Other important
findings concern grants that are not bound to the actual launched application
version, globally accessible userspace mailboxes, raw-frame injection through
the ordinary TCP/IP endpoint, and denial of service against serial management
services.

The default image additionally uses public development signing keys and a
publicly known recipient private key. This is an intentional fixture, but it is
also the normal operational boot configuration. Real credentials must not be
sealed to that recipient, and the image must not be mistaken for a production
trust configuration.

This report distinguishes implementation defects from deployment assumptions
and production hardening gaps. A documented limitation is still a security risk
when the deployment exceeds the assumptions under which it is safe.

## Scope, evidence, and limitations

- Repository: `charlotte-os`, branch `main`.
- Reviewed revision: `42183c57ce4c0b32a6010246f6eee1b6262ebb4e`.
- Method: static source review, tracing calls across kernel, runtime, protocol,
  service, launch, and signing-tool boundaries; cross-checking architecture
  Markdown, LaTeX source, existing investigations, CI, and selected formal-model
  assumptions.
- No implementation changes, exploit execution, guest boots, builds, test-suite
  runs, dependency-advisory scan, or hardware experiments were performed during
  this audit. The findings below are source-supported, not reproduced attacks.
- The sibling broker, Durga, and Sitas repositories were not independently
  audited. Their interfaces increase the importance of the application
  isolation findings, but this is not an audit of those implementations.
- This is a broad boundary review, not a line-by-line proof of every unsafe
  block, a cryptographic implementation audit, or a security certification.
  Existing tests and TLA+ models are evidence of intended invariants, not proof
  that this exact revision passed them during this review.

Source links and line numbers refer to the reviewed revision. Reports are
historical records; later fixes should cite the finding IDs and their validation
evidence rather than silently removing findings.

### Threat model

The review considers an unauthenticated IP client, a hostile machine on the
cluster's L2 segment, a compromised but legitimately signed application, a
compromised connector or privileged platform service, a malicious DMA-capable
device, and compromise of a build or operations workstation.

Signatures establish approval and provenance; they do not make application
behaviour harmless. An admitted application must remain unable to alter kernel
translations, communicate outside its delegated authority, or exhaust the
node's essential resources. A cluster node is a more privileged actor: the
current consensus design assumes cooperating, non-Byzantine members.

Trusted components include the kernel and boot chain, name service, grant
controller, deployment agent/controller, time service for security decisions,
device managers/drivers, and connectors holding credentials. Compromise of these
components has greater consequences than compromise of an ordinary application.
The hypervisor, firmware, and key-provisioning environment are also part of the
deployment trust base; in-OS artifact verification does not authenticate them.

### Severity and confidence

Critical means a fundamental isolation failure or a configuration that defeats
production signing/confidentiality. High means a significant authority bypass,
cross-domain interference, or substantial availability/security-policy impact.
Medium denotes narrower or conditional exposure and operational hardening.
Low denotes defence in depth.

“Confirmed” means that the relevant missing check or behaviour was established
in source. It does not mean that an end-to-end exploit was executed. Conditional
impact and unverified consequences are stated explicitly.

## Findings at a glance

| ID | Severity | Finding | Required access / condition |
| --- | --- | --- | --- |
| SEC-01 | Critical | Explicit user mappings can reach shared kernel page tables | Any running application; MMIO variant requires a device capability |
| SEC-02 | High | Grant policy is not bound to the caller's launched descriptor | Application plus a valid descriptor for another version of its logical artifact |
| SEC-03 | High | Userspace LP mailboxes are globally accessible across domains | Any running application; victim must use this mailbox ABI |
| SEC-04 | Critical, conditional | Normal boot retains public development signing and decryption trust | Default image used with real deployment authority or secrets |
| SEC-05 | High | Ordinary TCP/IP clients can inject raw receive frames | A TCP/IP CALL capability |
| SEC-06 | High | Remote clients can stop or monopolize HTTP management services | Network access to the relevant listener |
| SEC-07 | High | Per-object limits do not enforce aggregate domain resource budgets | Any running application |
| SEC-08 | High, conditional | Cluster and DSR peer identity rests on spoofable L2 information | An untrusted participant on the cluster segment |
| SEC-09 | High | Unauthenticated or stale UTC is used as security-policy time | Time-source/path manipulation, or persisted/stale clock state |
| SEC-10 | Medium | Management transport and observability lack client authentication | Network access or an on-path position |
| SEC-11 | Medium | Configured trust does not propagate to all launch/grant gates | Deployment using non-development artifact/deployment keys |
| SEC-12 | Medium, conditional | Local object-store authority is store-wide, not object-scoped | A local object-store capability |
| SEC-13 | Medium | Signing-tool invocation exposes private key material in argv | Access to process arguments, command tracing, or recorded invocations |
| SEC-14 | Medium | Builds do not enforce the committed lockfile; CI action references are mutable | Dependency/action supply-chain compromise or unreviewed updates |
| SEC-15 | Low | S3 signing leaves non-zeroizing secret-derived temporaries | Subsequent access to connector memory; defence in depth |
| SEC-16 | High | An unavailable grant target can stall the whole grant controller | Application with a signed grant for an unavailable service |

## Detailed findings

### SEC-01 — Explicit user mappings can modify shared kernel translations

**Confirmed isolation defect; Critical.**

The syscall constructs a `VAddr` from a caller-controlled integer and passes it
to the memory-object mapper. The mapper validates page alignment, capability
ownership, mapping rights, and lending/pinning state, but does not validate that
the complete mapping range is in the application's permissible virtual space.

Evidence:

- [syscall/mod.rs:1222](../../../crates/catten/src/syscall/mod.rs#L1222):
  `sys_memory_map` accepts the requested base without a user-range check.
- [memory/object.rs:568](../../../crates/catten/src/memory/object.rs#L568):
  `map` checks alignment, not address-space partition or range overflow.
- [memory/object.rs:649](../../../crates/catten/src/memory/object.rs#L649):
  `map_locked` installs pages at `base + index * PAGE_SIZE`.
- [AArch64 paging/mod.rs:219](../../../crates/catten/src/cpu/isa/aarch64/memory/paging/mod.rs#L219):
  user address spaces inherit the current kernel `ttbr1_el1`.
- [AArch64 walker.rs:83](../../../crates/catten/src/cpu/isa/aarch64/memory/paging/walker.rs#L83):
  a higher-half address selects TTBR1; the mapping path subsequently edits that
  hierarchy.
- [device/mod.rs:590](../../../crates/catten/src/device/mod.rs#L590):
  explicit MMIO mapping has the same alignment-only address gate.
- [vaddr.rs:143](../../../crates/catten/src/cpu/isa/common/memory/address/vaddr.rs#L143):
  integer conversion normalizes/sign-extends addresses instead of rejecting an
  invalid raw input.

An application owns the physical frames it presents, but that does not entitle
it to install them in shared kernel virtual space. An unused higher-half slot
can be populated through this path. The existing “already mapped” check limits
direct replacement of an existing leaf; it does not make modification of a
shared kernel hierarchy safe. No arbitrary kernel-memory read/write or code
execution exploit was demonstrated in this audit.

The x86-64 constructor also shares higher-half subordinate tables
([paging/mod.rs:83](../../../crates/catten/src/cpu/isa/x86_64/memory/paging/mod.rs#L83)).
The exact consequence depends on existing parent mappings and permissions, but
the architecture-neutral syscall must reject such requests before either
architecture's walker is invoked. Unchecked end arithmetic is an additional
hazard, especially with overflow checking enabled.

The ELF gate needs the same review: `validate_user_elf` uses a fixed `1 << 48`
ceiling, while segment mapping later normalizes addresses through `VAddr::from`
([loader.rs:191](../../../crates/catten/src/service/loader.rs#L191),
[loader.rs:294](../../../crates/catten/src/service/loader.rs#L294)). On a
sign-extending 48-bit configuration this is not equivalent to validating the
canonical lower user half. This related path requires a signed artifact; the
memory-object syscall requires no new signed artifact.

**Correction:** introduce an architecture-correct, checked user-range type at
the ABI boundary. Validate the original raw start, checked byte length/end,
canonical form, null-page exclusion, and user/kernel partition before changing
any registry or page table. Reuse it for explicit memory/MMIO maps and ELF
segments. Prevent userspace mapping code from selecting shared kernel roots
even if a higher-level check is accidentally omitted. Do not merely mask or
canonicalize a rejected address.

**Acceptance:** negative tests for kernel-half addresses, noncanonical aliases,
near-maximum wrapping ranges, null, existing runtime pages, and crossing the
user boundary; verify shared roots and neighbouring mappings remain unchanged
on both architectures. Include allocation/mapping rollback and TLB behaviour.

### SEC-02 — Grants are transferable between versions of a logical artifact

**Confirmed authorization gap; High.**

[grantctl.rs:50](../../../crates/catten-services/src/bin/grantctl.rs#L50)
verifies the descriptor supplied in the request and compares its artifact-name
principal with the kernel-authenticated caller principal. It then checks the
grants in that supplied descriptor. It does not compare the descriptor with the
one actually admitted for the caller's address-space generation or compare the
descriptor's artifact digest with the running image.

The launch path does install an immutable descriptor into the application
([supervisor.rs:768](../../../crates/catten/src/service/supervisor.rs#L768)).
That helps cooperative clients, but `grantctl` accepts independently constructed
request bytes; an application is not forced to use its installed descriptor.
The controller's revision ledger is keyed by principal, starts empty, and is
volatile ([grantctl.rs:82](../../../crates/catten-services/src/bin/grantctl.rs#L82),
[grantctl.rs:246](../../../crates/catten-services/src/bin/grantctl.rs#L246)).

A compromised version can present a valid descriptor for another version of
the same name with broader grants. A newer approved-but-not-launched descriptor
is particularly clear: the controller accepts its higher sequence even though
the caller is still the older image. Older descriptors can also be accepted
before a higher sequence has been observed or after controller restart. No
signature forgery or principal collision is required.

**Correction:** make the trusted launcher install the exact descriptor digest,
policy, and deployment-key identity against the caller's immutable occupancy
`(ASID, generation)`. Resolve grants against that record. A request may carry
a descriptor for convenience, but it must match the launched policy exactly.
Policy updates should be explicit authenticated operations, not side effects of
an application presenting a higher signed sequence. Bind publication as well
as acquisition, and handle retired generations and controller restart.

**Acceptance:** version A cannot acquire or publish using broader version B's
descriptor, even if B is correctly signed, newer, previously deployed, or has
the same artifact name. Test controller restart and ASID reuse.

### SEC-03 — Userspace LP mailboxes bypass domain communication authority

**Confirmed cross-domain channel; High when this ABI is used.**

The kernel-global `USER_MAILBOX` contains one scalar queue per logical
processor, not per application. Legacy send/receive syscalls discard the caller
ASID after checking that it is a user caller. The capability API lets any caller
open a send endpoint for any valid LP and a receive endpoint for its current LP.
Those capabilities still target the same global queues.

Evidence: [syscall/mod.rs:956](../../../crates/catten/src/syscall/mod.rs#L956),
[syscall/mod.rs:1039](../../../crates/catten/src/syscall/mod.rs#L1039), and
[syscall/mod.rs:1063](../../../crates/catten/src/syscall/mod.rs#L1063).

A domain scheduled on the same LP can consume another domain's mailbox words.
Any domain can inject words or fill a queue. Queue entries do not carry
authenticated sender identity. Minting an owner-checked handle does not repair
the absence of an authorization decision when opening the shared queue.

This concerns the userspace syscall mailbox, not every typed kernel/Sitas
mailbox. The audit did not establish which external applications currently
depend on it, or demonstrate exploitation of a pointer-bearing mailbox word.

**Correction:** scope shard mailboxes to an address-space generation and LP;
provide separately delegated endpoints for deliberate inter-domain channels.
Remove, restrict, or compatibility-gate the ambient legacy ABI. Teardown must
not leave old words available to a recycled occupancy.

**Acceptance:** two domains on one LP cannot steal or inject each other's
messages; generation reuse does not expose old messages; cross-domain use
requires explicit delegation and authenticated policy.

### SEC-04 — Normal operational boot uses publicly known development trust

**Confirmed default configuration; Critical if used for real authority/secrets.**

The steady-state boot path invokes `launch_deployment_plane(&ns, b"charlotte")`
([launch.rs:1056](../../../crates/catten/src/service/launch.rs#L1056)). That helper
installs development admission trust and the embedded development recipient
private key ([launch.rs:774](../../../crates/catten/src/service/launch.rs#L774)).
The corresponding signing/recipient fixtures are tracked under
`tools/cluster-sign/`. Normal ELF signing also falls back to the development key
([sign-service-elfs.sh:22](../../../scripts/sign-service-elfs.sh#L22)).

These are deliberately public fixtures, not accidentally leaked production
secrets. Nevertheless, anyone possessing the repository can produce signatures
accepted by that default trust and decrypt operational envelopes addressed to
that development recipient. Network access and ordinary admission constraints
still affect exploitability; cryptographic proof under a public private key
does not establish a trusted operator.

Alternative provisioning functions exist, including
`launch_deployment_plane_with_operational_key`, but the normal boot path does
not select an independently provisioned production trust configuration. SEC-11
also obstructs using custom keys consistently.

**Correction:** provide an explicit development-image mode and a production
mode that fails closed without provisioned trust. Reject known fixture keys in
production and clearly identify fixture images in boot/management output.
Provision distinct artifact, deployment, operations, and recipient keys; define
key custody, recovery, rotation, and revocation. Authenticate the boot image and
protect recipient material in the platform threat model.

**Acceptance:** a production image refuses development-signed input and
development-recipient configuration, and custom independently generated keys
work end to end. Do not upload real connector credentials under the fixture
recipient while awaiting this work.

### SEC-05 — A socket capability also permits raw-frame injection

**Confirmed service-authority gap; High.**

The TCP/IP reactor obtains authenticated sender identity and uses it to own
sockets ([tcpip.rs:893](../../../crates/catten-services/src/bin/tcpip.rs#L893)).
However, its `OP_FRAME` branch accepts any valid frame attachment and pushes it
into the shared receive device without checking that the caller is the trusted
frame router ([tcpip.rs:1358](../../../crates/catten-services/src/bin/tcpip.rs#L1358)).

A normal socket client with CALL authority can therefore impersonate the
stack's network input. This bypasses per-socket ownership: injected traffic is
processed against the whole shared stack. It can influence ARP/IP/TCP/UDP and
DHCP processing, subject to their normal packet checks. Full TCP stream
hijacking was not demonstrated; it is not necessary to establish that the
capability exposes unintended raw ingress authority.

The same endpoint also exposes broad bind/listen operations. Socket ownership
does not, by itself, authorize a caller to claim another service's VIP/port.
That policy should be addressed when separating client and platform endpoints.

**Correction:** use a private frame-ingress endpoint delegated only to the
router, or authenticate its exact kernel identity/generation in this branch.
Give applications a separate socket endpoint with explicit bind/listen policy.
Do not use a caller-provided role/name as the authority check.

**Acceptance:** an application with a valid socket capability cannot submit
`OP_FRAME`; genuine router delivery works and router generation replacement is
handled explicitly. Test negative VIP/port claims as well as ordinary sockets.

### SEC-06 — Remote peers can stop or monopolize HTTP services

**Confirmed error/deadline handling defects; High.**

The node/cluster keyhole handles one connection at a time. While awaiting the
first request chunk it retries timeouts indefinitely. EOF calls `fail(0xe00e)`;
other receive errors also call `fail`. `fail` terminates the service thread.
See [httpd.rs:1468](../../../crates/catten-services/src/bin/httpd.rs#L1468) and
[httpd.rs:225](../../../crates/catten-services/src/bin/httpd.rs#L225).
The socket helper decodes a zero-length completion as EOF
([lib.rs:1081](../../../crates/catten-services/src/lib.rs#L1081)).

An accepted idle connection can monopolize the keyhole; closing before sending
a request can lead to thread exit rather than connection-local cleanup. The
boot launch reviewed here starts `httpd` directly, without a demonstrated
automatic restart policy ([launch.rs:734](../../../crates/catten/src/service/launch.rs#L734)).
A deployed supervisor could reduce outage duration, but would not correct the
remote failure trigger.

The deployment HTTP adapter is also serial. Its bounded per-receive wait is
restarted indefinitely, without a total header/body deadline or minimum
progress requirement ([deployd.rs:197](../../../crates/catten-services/src/bin/deployd.rs#L197),
[deployd.rs:513](../../../crates/catten-services/src/bin/deployd.rs#L513)). A client
can hold the admission listener with an idle or incomplete request. Request
size caps do not prevent this, and body signatures are verified too late to
protect listener availability.

**Correction:** EOF/reset/malformed input must close one connection and resume
listening. Add monotonic total request deadlines, bounded concurrent clients,
header/body limits, progress policy, and bounded reply time. Preserve lifecycle
responsiveness. Treat expected network errors as routine events, not domain
termination.

**Acceptance:** connect-and-close, idle client, slow partial body, reset during
reply, and malformed request do not stop service or starve healthy clients.
Verify the deployment endpoint remains usable while hostile clients are open.

### SEC-07 — Aggregate resource exhaustion remains possible

**Confirmed missing budget enforcement; High.**

Memory objects have a 64 MiB per-allocation bound, explicitly not a per-domain
total quota ([object.rs:34](../../../crates/catten/src/memory/object.rs#L34)).
`allocate` validates length and occupancy, then obtains shared frames; it does
not consult an aggregate owner budget
([object.rs:226](../../../crates/catten/src/memory/object.rs#L226)). Applications
can repeat allocations independently of their signed heap window or stack
limits. Capability registries also grow through heap-backed maps. IPC endpoint
capacity is bounded per endpoint, not proof of a bound on all endpoints/caps
owned by one domain ([ipc/mod.rs:338](../../../crates/catten/src/ipc/mod.rs#L338)).

Socket quotas are a genuine improvement, but constrain only socket allocations
in the TCP/IP service. A malicious domain can still exhaust shared frames or
kernel bookkeeping and impair other domains and essential services. Physical
allocation failure has rollback; this is not a claim that every such failure
necessarily panics the kernel. Infallible heap allocation and other allocation
callers need their own pressure audit.

**Correction:** enforce generation-scoped aggregate budgets for physical pages,
kernel metadata, endpoints, pending calls, timers, and capabilities, with global
reserves for control-plane/kernel progress. Define accounting on copy, lend,
move, DMA pin, cancellation, and delayed teardown. Telemetry and placement
feedback must supplement, not replace, hard admission enforcement.

**Acceptance:** a hostile domain allocating many small and large objects cannot
consume another domain's reserved capacity or stop management progress; all
charges reconcile after failures, transfer, abort, and generation reuse.

### SEC-08 — Cluster and DSR identities depend on a trusted L2 segment

**Confirmed architectural boundary; High outside that deployment assumption.**

Discovery learns peers from matching cluster bytes and advertised identities,
keyed by source MAC ([disco.rs:445](../../../crates/catten-services/src/bin/disco.rs#L445),
[disco.rs:496](../../../crates/catten-services/src/bin/disco.rs#L496)). Raft
transport checks the asserted candidate/leader against the learned source MAC
([relmsg_transport.rs:428](../../../crates/catten-services/src/relmsg_transport.rs#L428)).
DSR encapsulation checks a forwarder's source MAC against the current membership
snapshot ([frouter.rs:326](../../../crates/catten-services/src/bin/frouter.rs#L326)).
These are useful consistency checks, not cryptographic peer authentication.

The networking manual correctly states the trusted-L2 requirement, but the
cluster-vision chapter describes membership as supplying “authenticated
Ethernet routes”
([19-cluster-vision.tex:642](../../manual-v2/chapters/19-cluster-vision.tex#L642)).
That wording should be narrowed to membership-associated/MAC-checked routes
until peer authentication exists. Kernel-authenticated local IPC and
cryptographically authenticated remote nodes are different guarantees.

A hostile L2 participant can spoof a known peer's MAC and send plausible
protocol messages, interfere with elections/replication, misrepresent discovery
or telemetry, or inject encapsulated ingress. Signed artifact/admission records
remain additional checks; this finding does not establish that every forged
Raft message can install unsigned software. Raft does not provide Byzantine
resilience merely because membership is replicated.

Discovery's peer map also has no explicit admission count in `learn_peer`;
many fabricated MAC identities can consume service memory until expiry. The
reliable-message service's peer/reassembly bounds do not bound this separate
discovery map.

**Correction:** retain an isolated cluster fabric as an immediate requirement.
For a broader LAN deployment, authenticate node enrollment and control traffic
with provisioned node identity, freshness/replay protection, and revocation.
Authenticate DSR forwarding or confine it to an enforceably trusted network.
Bound discovery candidates separately from admitted membership. Treat member
telemetry as claims subject to policy and plausibility checks.

**Acceptance:** spoofed MAC, unknown enrollment, replayed control frames,
fabricated telemetry, and discovery floods cannot impersonate admitted peers
or exhaust the control plane. Define behaviour for a genuinely compromised
admitted node rather than claiming ordinary Raft solves it.

### SEC-09 — Security-policy UTC is unauthenticated and can be stale

**Confirmed trust/freshness gap; High.**

The time client validates SNTP version/mode, stratum, nonzero timestamps, and
the echoed originate token, but performs no cryptographic source authentication
([time.rs:553](../../../crates/catten-services/src/bin/time.rs#L553)). The token
is derived from local time/counter data, not a cryptographically random nonce
([time.rs:354](../../../crates/catten-services/src/bin/time.rs#L354)). An on-path
attacker can observe and answer the request regardless of nonce quality.

Accepted samples directly replace the UTC anchor; damping applies to oscillator
drift, not an authentication or bound on the new absolute time
([time.rs:165](../../../crates/catten-services/src/bin/time.rs#L165)). The
non-regression guard prevents exposed time moving backwards, but a large
forward jump can leave time incorrect and effectively held until the true
clock catches up.

There is a second issue independent of network spoofing: `OP_UNIX_SECONDS`
returns any available model, including persisted holdover, without returning
state, uncertainty, or synchronization age
([time.rs:848](../../../crates/catten-services/src/bin/time.rs#L848)). Both
`dns::trusted_unix_seconds` and the agent's similarly named helper consume that
scalar ([dns.rs:1261](../../../crates/catten-services/src/bin/dns.rs#L1261),
[agent.rs:480](../../../crates/catten-services/src/bin/agent.rs#L480)). Connector
TLS startup checks synchronization state, but that state means a successful
SNTP exchange, not authenticated UTC.

Consequences include incorrect deployment/shutdown/ingress validity-window
decisions and TLS certificate-validity checks. Accepting an expired signed
operation requires the relevant replay/sequence checks to allow it too; time
manipulation does not itself forge a signature. Future jumps can also reject
legitimate operations and produce long-lived denial of service.

**Correction:** define a security-time policy with authenticated provenance,
freshness, uncertainty, permissible correction, and bootstrap rules. Use the
full snapshot for admission and fail closed if its security-time criteria are
not met. Keep ordinary display/holdover time available without calling it
trusted admission time. NTS provides cryptographic NTP protection, while its
certificate bootstrap and residual delay attacks still need explicit handling
([RFC 8915](https://www.rfc-editor.org/rfc/rfc8915.html)).

The final operational launch syscall additionally consumes
`pickup.now_unix_seconds` supplied by the privileged agent
([syscall/mod.rs:2339](../../../crates/catten/src/syscall/mod.rs#L2339)). This is
not an ordinary-application bypass—the agent is explicitly trusted—but kernel
re-verification does not independently establish current UTC. Document that
TCB dependency or obtain security time independently at the final gate.

**Acceptance:** forged replies, forward/backward jumps, prolonged holdover,
boot with a stale calibration, unsynchronized state, and excessive uncertainty
cannot incorrectly authorize time-bounded operations. Use monotonic time for
timeouts even when UTC is under correction.

### SEC-10 — Management endpoints lack authenticated transport

**Confirmed production gap; Medium on a protected management network.**

The keyhole HTTP endpoint and deployment adapter serve plaintext traffic
without browser/client authentication. The existing
[cluster-observability design](../../architecture/cluster-observability.md)
correctly acknowledges this and plans mTLS. The deployment adapter has signed
mutation bodies, which is stronger than an unrestricted management API; the
absence of TLS does not erase those signature checks.

Nevertheless, any reachable client can read exposed topology, resource,
placement, and management status, and an on-path attacker can observe or alter
HTTP requests/responses and availability. Operational envelopes protect their
encrypted profile bodies, not every surrounding metadata field or status page.
Protecting only the leader VIP would leave per-node keyholes exposed.

**Correction:** protect both node and cluster management with an authenticated
TLS endpoint, browser/client certificate setup, distinct read/admin policy,
certificate rotation/revocation, and audit identity. Until then, enforce
network-level access control; a VIP is a routing identity, not access control.
Keep signature verification of sensitive operations even after adding TLS.
Require TLS for production connector profiles rather than relying on operators
to avoid permitted plaintext test profiles.

**Acceptance:** unauthorized clients cannot read management data, authorized
readers cannot mutate policy, and certificate expiry/revocation and leader
changes preserve the access policy. Verify direct node access as well as VIP
access.

### SEC-11 — Custom trust keys stop at hardcoded downstream gates

**Confirmed configuration/availability defect; Medium.**

Scoped deployment verifies descriptors/artifacts against configured keys
([supervisor.rs:779](../../../crates/catten/src/service/supervisor.rs#L779)).
However, its subsequent `loader::try_load_domain` call verifies every image
against the constant development `CLUSTER_PUBLIC_KEY`
([loader.rs:367](../../../crates/catten/src/service/loader.rs#L367),
[loader.rs:462](../../../crates/catten/src/service/loader.rs#L462)). `grantctl`
also verifies descriptors against that constant
([grantctl.rs:64](../../../crates/catten-services/src/bin/grantctl.rs#L64)).

A correctly configured independent artifact/deployment key therefore does not
work end to end. This is principally false rejection and an obstacle to
removing fixture trust, not evidence that arbitrary signatures bypass the
configured earlier gate. Bundled platform artifacts and deployed application
artifacts may reasonably have different roots; that distinction must be
explicit rather than an accidental second development-key check.

**Correction:** carry verified trust context into loading, retaining independent
verification under the right key, and provision grant policy with its proper
deployment key. Avoid “fixing” this by silently accepting the union of all old
and new keys without policy. Review the automatic administrative roles granted
to `ArtifactClass::Administration`
([loader.rs:468](../../../crates/catten/src/service/loader.rs#L468)): decide
which signer and deployment policy are permitted to authorize those roles.

**Acceptance:** distinct platform/artifact/deployment/operations keys work in
the intended paths; using any key in the wrong role fails, and fixture keys are
not implicitly retained as fallback roots.

### SEC-12 — Local object-store capabilities cover the entire store

**Confirmed coarse authority; Medium, potentially High if given to tenants.**

The local object store dispatches read/write/delete/create-at operations by
numeric object ID without owner, namespace, or operation-specific policy
([objstore.rs:1267](../../../crates/catten-services/src/bin/objstore.rs#L1267)).
A caller holding its connection can access objects belonging to other clients.
Object magic strings help inspection; they are not authorization checks.

This differs from the S3 connector's configured bucket/prefix/rights boundary.
The audit did not establish that an ordinary deployed application receives
`objstore` by default. Restricting that capability to trusted infrastructure is
a legitimate current defence. If generic application storage is exposed through
it, callers could read or modify infrastructure state, persisted calibration,
or other application data, subject to any higher-level integrity checks.

**Correction:** document the existing endpoint as privileged store-wide
authority. Introduce object/namespace capabilities or mediated application
storage endpoints, with separate read/write/admin rights and quotas. Preserve
signatures/digests for stored artifacts and define integrity protection for
security-relevant persistent state. Do not grant raw block/store access as a
convenient application storage API.

**Acceptance:** a tenant cannot read/write/delete another tenant's or platform's
objects, including through guessed IDs and `CREATE_AT`; trusted recovery tooling
retains an explicitly separate administrative capability.

### SEC-13 — Signing helpers put private keys on the command line

**Confirmed key-handling weakness; Medium.**

[sign-service-elfs.sh:72](../../../scripts/sign-service-elfs.sh#L72) passes
`PRIVATE_KEY` as a positional argument to `elf-sign`.
[main.rs:732](../../../tools/cluster-sign/src/main.rs#L732) reads it from argv.
The run script similarly passes signing secrets in deployment/shutdown
invocations. Using `CLUSTER_SIGN_PRIVATE_KEY` avoids a tracked production key
file, but does not avoid argv exposure.

Process-argument inspection, traced commands, recorded invocations, or shell
history for manual commands can retain the key. Exact visibility depends on
host permissions. Some operational commands already use key-file interfaces,
so there is an existing direction to follow.

**Correction:** use restricted key files, inherited descriptors, or a signing
agent/hardware-backed signer, without raw secrets in argv. Do not echo keys;
zeroize temporary decoded storage and audit diagnostic/error paths. Protect
plaintext connector-profile inputs and generated files on the workstation too.

**Acceptance:** process arguments and normal logs contain key references, never
key bytes, for all signing/sealing commands. Verify file creation permissions,
overwrite behaviour, and failure cleanup.

### SEC-14 — Builds do not enforce the committed lockfile or immutable actions

**Confirmed supply-chain hardening gap; Medium.**

The workspace `Cargo.lock` **is tracked**. The initial audit incorrectly inferred
otherwise from the root `.gitignore` entry; `git ls-files Cargo.lock` confirms
that ignore rules do not remove this already tracked file. Several dependencies
use open-ended lower bounds ([catten/Cargo.toml](../../../crates/catten/Cargo.toml)),
which are not independently a vulnerability when the lockfile is enforced.
Build and CI commands omit `--locked`, so an inconsistent manifest/lockfile can
be silently re-resolved instead of rejected. CI also uses
mutable action tags and `dtolnay/rust-toolchain@master`
([ci.yml](../../../.github/workflows/ci.yml)); the Rust toolchain itself is
pinned, the Sitas checkout is revision-pinned, and the TLA+ jar is checksum-checked.
Those existing protections should be retained.

No vulnerable crate version was established: there was no advisory scan and no
claim that a particular dependency is compromised. The problem is that the
reviewed dependency/action graph is not reliably the graph used on rebuild.

**Correction:** retain the tracked workspace lockfile, use `--locked` in release/CI
builds, constrain compatibility deliberately, pin third-party actions to full
commit IDs, minimize workflow token permissions, and introduce advisory/license
checks plus a release dependency inventory. Cargo documents how the lockfile
captures exact resolution
([Cargo Book](https://doc.rust-lang.org/cargo/guide/cargo-toml-vs-cargo-lock.html));
GitHub recommends full-SHA action pinning
([secure-use reference](https://docs.github.com/en/actions/reference/security/secure-use)).

**Acceptance:** a clean build of the same commit uses the same resolved graph;
dependency and action changes produce explicit reviewable diffs. Scan that exact
graph before making any “no known vulnerabilities” claim.

### SEC-15 — S3 signing temporaries retain secret-derived material

**Confirmed defence-in-depth gap; Low.**

The connector stores its long-lived S3 secret in `Zeroizing<Vec<u8>>`
([s3.rs:99](../../../crates/catten-services/src/bin/s3.rs#L99)), but the SigV4
helper creates an ordinary vector containing `AWS4 || secret` and ordinary
arrays for the derived date/region/service/signing keys
([sigv4.rs:143](../../../crates/charlotte-s3/src/sigv4.rs#L143)). Their release
does not guarantee erasure. TLS buffer reclamation likewise frees buffers
without an explicit wipe
([tls_client.rs:227](../../../crates/catten-services/src/tls_client.rs#L227)).

This is not a demonstrated cross-domain secret read: newly allocated
memory-object frames are zeroed, and normal page isolation still applies.
Connector compromise, same-domain heap reuse, or crash/debug access changes
the exposure. Zeroization limits remanence; it does not defeat an attacker who
already controls a connector while its credentials are live.

**Correction:** zeroize secret-bearing SigV4 temporaries and review cryptographic
state/plaintext buffers at teardown. Include SCRAM intermediates, TLS client
identity parsing, HPKE scratch, and host-tool copies in that lifecycle review.

**Acceptance:** inventory each secret copy and its lifetime, keep it out of
diagnostics, and verify erasure/cleanup on both success and failure where the
chosen libraries permit it. Do not promise complete erasure of all compiler or
CPU copies.

### SEC-16 — Deferred lookup blocks the whole grant controller

**Confirmed shared-service availability defect; High.**

After accepting a grant request, the controller synchronously waits for
`OP_LOOKUP_FOR_GRANT`
([grantctl.rs:207](../../../crates/catten-services/src/bin/grantctl.rs#L207)).
The name service parks that request when the target service is missing
([ns.rs:1086](../../../crates/catten-services/src/bin/ns.rs#L1086)). The controller
has a single blocking receive/handle loop and no lookup timeout
([grantctl.rs:240](../../../crates/catten-services/src/bin/grantctl.rs#L240)).

One application with a valid grant to an unavailable service can therefore
prevent unrelated applications from acquiring or publishing capabilities.
Ordinary dependency startup can trigger the same behaviour accidentally; if
publication needed to satisfy the lookup must pass through this controller,
the serialization can produce a dependency deadlock. Waitlist bounds in the
name service do not make the blocked controller concurrent.

**Correction:** retain each pending acquisition in an owning operation struct,
poll it without blocking the admission reactor, enforce per-client/global
pending limits and deadlines, and cancel when the requester disappears.
Publication and lifecycle handling must remain responsive. This matches the
repository's owned-resource guidance rather than adding manual cleanup ladders.

**Acceptance:** a missing service for application A does not block application
B's lookup/publication. Test late service arrival, timeout, caller cancellation,
controller shutdown, and returned-capability cleanup.

## Mechanisms that should be preserved

The audit found substantial positive work; these properties should survive the
corrections rather than being bypassed for convenience.

- **Kernel identity, not claimed identity.** IPC enqueue obtains generation,
  principal, and roles from kernel domain authority
  ([ipc/mod.rs:1115](../../../crates/catten/src/ipc/mod.rs#L1115)). Ordinary
  callers cannot choose those fields by constructing a message.
- **Capability namespaces and kinds.** Owner/kind checking in the unified
  [capability table](../../../crates/catten/src/capability.rs) prevents integer
  handle guessing from automatically conferring another domain's authority.
  Handles need not be random if lookup and delegation are correct.
- **Scoped launch authority.** Artifact spawn, connector spawn, retirement, and
  whole-node shutdown are restricted to the exact designated agent occupancy,
  not whoever registers the name `agent`
  ([syscall/mod.rs:2040](../../../crates/catten/src/syscall/mod.rs#L2040)).
- **Re-verification at operational launch.** The kernel checks configured
  signatures, artifact digest/name, exact release membership, envelope binding,
  cluster/profile/key identity, and expiry; HPKE plaintext is transient and
  zeroizing in that path
  ([syscall/mod.rs:2283](../../../crates/catten/src/syscall/mod.rs#L2283)). The
  time/provisioning dependencies identified above qualify this protection, not
  its existence.
- **Transactional launch preparation.** The profile launch transaction owns
  resources and rolls back failures before the initial thread starts
  ([supervisor.rs:716](../../../crates/catten/src/service/supervisor.rs#L716)).
- **No ambient name service for scoped applications.** Deployment launch gives
  the application the grant controller connection and a read-only descriptor.
  Acquired service connections are attenuated to the requested client rights;
  re-delegation authority stays with trusted mediation
  ([ns.rs:396](../../../crates/catten-services/src/bin/ns.rs#L396)). SEC-02 and
  SEC-16 are gaps in that mediation, not reasons to remove it.
- **ELF and data permissions.** The untrusted ELF validator rejects writable
  executable segments, overlap, malformed file bounds, and non-executable
  entry points. Memory objects map as NX data/rodata. The virtual-range flaw in
  SEC-01 must be repaired without weakening these checks.
- **Connector separation.** S3 fixes endpoint/bucket/prefix/rights at launch,
  rejects literal dot traversal segments, and owns streamed operations by
  caller generation. Kafka has role-specific endpoints, operation-owner
  checks, connector-only zeroizing SCRAM/mTLS credentials, and TLS-required
  authentication profile validation. Application APIs do not return credentials.
- **TLS fails closed on handshake/entropy failures.** The shared wrapper
  supplies a CA verifier and server name and requires usable entropy
  ([tls_client.rs:120](../../../crates/catten-services/src/tls_client.rs#L120)).
  Reviewed TLS-enabled connector branches return failure rather than silently
  falling back to plaintext. A caller-selected plaintext profile remains a
  separate policy issue.
- **Socket ownership and capacity.** The TCP/IP service uses authenticated
  occupancy ownership, checks socket/buffer quotas, and checks ownership on
  socket operations. This fixes a different problem from raw ingress authority.
- **DMA lifetime checks.** DMA mapping requires a delegated device domain and
  memory rights; pinning checks direction and conflicting mappings/lends, and
  delays freeing pinned objects
  ([object.rs:1489](../../../crates/catten/src/memory/object.rs#L1489)).
  [grant_dma_domain](../../../crates/catten/src/device/mod.rs#L491) requires a
  working requester-domain creation rather than treating a CPU physical address
  as an unrestricted DMA grant.
- **Bounded relmsg resources and catalog replay rules.** Reliable messaging
  has peer, queue, reassembly, and outbound bounds; catalog updates retain
  monotonic sequence/conflicting-digest checks. These improve robustness but
  do not authenticate L2 peers or bind grants to launch occupancies.

HPKE encryption is not a substitute for replay, admission, or key-custody
policy. Its application embedding must provide those properties; the existing
signed bindings and catalog sequences are therefore important
([RFC 9180, application non-goals](https://www.rfc-editor.org/rfc/rfc9180.html#section-9.7)).

## Coverage and residual risks

| Boundary | Reviewed evidence | Remaining assurance work |
| --- | --- | --- |
| EL0 → kernel | Syscall mapping, identity, memory/capability ownership, mailbox and launch gates | Hostile syscall testing; all unsafe memory/user-pointer paths; concurrency and fault injection |
| Application → service | Grant controller, namespace mediation, socket ownership, S3/Kafka ownership and policy | Exact launch-policy binding; opcode-specific authority; shared-service fairness and cancellation |
| Operator → cluster | Signed descriptors/releases/operations, HTTP adapter, kernel operational launch | Production provisioning; mTLS; rotation/revocation; admission-time freshness; audit retention |
| Node → node | Discovery, MAC-bound Raft transport, relmsg bounds, DSR forwarder checks | Cryptographic enrollment/traffic; replay resistance; compromised-member policy; authenticated telemetry |
| External managed service → connector | TLS wrapper, entropy sourcing, S3 signing and Kafka auth/profile gates | Certificate/hostname negative tests; exact dependency audit; parser fuzzing and long-lived connection behaviour |
| Device → memory | Requester-domain grant, direction/ownership pinning, IOMMU dispatch, teardown references | Real topology isolation groups, aliases/ACS/ATS/PASID, interrupt remapping, reset and fault storms on hardware |
| Persistent state → restart | Object-store dispatch, calibration, catalog sequences and signed artifacts | Object-scoped storage authority; malformed media/snapshot testing; durable anti-rollback and recovery |
| Build/workstation → image | Signing invocation, default trust, dependency manifests and CI | Verified boot, signer custody, immutable graph, provenance, secret-free logs and reproducibility |

### Hardware and boot assumptions

The previous [DMA security investigation](../investigations/2026-08-30-dma-security.md)
remains relevant. IOMMU translation constrains device-visible addresses; it does
not authenticate device output or prove the safety of every PCIe topology.
Requester aliases, peer-to-peer routing, multifunction/bridge isolation,
firmware-reserved mappings, interrupts, stale translations after reset, and
fault storms require platform-specific assurance. This audit did not repeat
hardware experiments or establish that those residual concerns were resolved.

The [EL2 capability-root document](../../architecture/el2-capability-root.md)
is a research design, not an implemented protection that can be assumed here.
Similarly, artifact signatures inside Charlotte do not demonstrate an
authenticated firmware/bootloader/kernel chain. A hostile hypervisor can also
observe guest memory and host-provided entropy under the present deployment
model; virtio RNG is useful entropy provisioning, not protection from its host.

### Keys, administrative roles, and recovery

The project needs an operational definition of who may sign an artifact as
`Administration`, who approves its placement/grants, and who may rotate each
trust role. Separate fields for artifact/deployment/operations keys are useful,
but separation of duties must also be enforced at admission and provisioning.
Document emergency revoke/redeploy, retirement of already issued capabilities,
loss of quorum, clock-source failure, recipient-key compromise, and restoring
storage without rolling back security sequences. No complete production
runbook for these cases was validated by this review.

The kernel launch gate trusts the designated agent's supplied time and pickup
selection. That is an explicit TCB relationship, not a guarantee that the
kernel independently consulted the current committed desired state. If the
design goal is containment of a compromised agent, the final gate needs a
trusted committed-state/current-generation reference as well as signatures.

### Parsers and memory safety

The reviewed ELF and connector decoders contain length/shape checks, but this
audit did not fuzz every protocol. Priority inputs include ELF program headers,
IPC vector attachments, HTTP Content-Length ambiguity and partial bodies,
S3 chunked/ranged responses, Kafka nested length/count fields, Raft protobuf and
snapshot restoration, relmsg fragment bookkeeping, and malformed object-store
media. A validly signed malformed artifact is still a robustness input: signing
does not justify kernel assertions that can crash the entire node.

RAII and owned capabilities reduce leak/double-close risks, but cannot establish
authorization, memory-range validity, service fairness, or hardware quiescence
by themselves. Driver and legacy reactor raw-syscall regions deserve continuing
review against the ownership guide. Timing/speculative-execution side channels,
constant-time cryptographic behaviour, and cryptographic-library internals were
not evaluated.

## Remediation order

1. **Repair the application isolation boundary first:** SEC-01, then SEC-02,
   SEC-03, and SEC-05. Add negative tests at the raw ABI level; testing only the
   cooperative Rust helpers would miss these problems.
2. **Keep essential services responsive:** SEC-06 and SEC-16; then enforce the
   aggregate budgets in SEC-07. Cancellation and cleanup must obey the typed
   ownership rules.
3. **Make non-fixture trust usable:** SEC-04 and SEC-11 together, followed by
   signing-tool key handling (SEC-13). Do not temporarily retain public fixture
   keys as an undocumented production fallback.
4. **Secure the cluster's environment:** authenticated security time (SEC-09),
   management access (SEC-10), and node/control traffic (SEC-08), with explicit
   isolation requirements until those mechanisms exist.
5. **Reduce operational blast radius:** storage attenuation (SEC-12), immutable
   dependency/CI inputs (SEC-14), secret-copy cleanup (SEC-15), then hardware and
   recovery validation from the coverage table.

While fixes are pending: run only trusted applications on an isolated cluster
and management network, retain development-only credentials, do not expose
keyholes/deployd broadly, and do not present the build as a multi-tenant security
boundary. These restrictions reduce exposure; they do not fix SEC-01.

## Validation plan for a follow-up hardening effort

No row below was executed as part of this audit.

| Test family | Required negative cases and success criterion |
| --- | --- |
| Mapping isolation | Kernel-half/noncanonical/wrapping ranges; raw syscalls; ELF boundary addresses; shared tables unchanged after rejection |
| Grant provenance | Same-name different digests/sequences; undeployed future version; rollback descriptor; controller restart and ASID reuse; only launched policy accepted |
| Mailbox isolation | Two domains sharing an LP; injection/drain attempts; retired-generation messages; explicit delegation required |
| Network authority | Socket caller submitting OP_FRAME; unauthorized VIP binds; socket-ID guessing; valid router delivery; forged DHCP/ARP input from unauthorized IPC rejected |
| Management availability | EOF, reset, idle/slow request, oversize/ambiguous headers, simultaneous healthy clients, shutdown during receive; no domain exit or unbounded monopoly |
| Grant availability | Missing service, late publication, abandoned caller, exhausted pending quota; unrelated grants and shutdown continue |
| Pressure | Many objects/endpoints/timers/caps plus socket churn; per-owner rejection before global reserves are consumed; charge rollback verified |
| Trust separation | Four independent role keys, wrong-role signatures, development keys, invalid release membership, altered ciphertext, replay/conflicting sequence; all wrong inputs rejected |
| Security time | Stale/unsynchronized/uncertain clock, authenticated-source loss, spoof/jump/delay, reboot holdover; expiry uses policy-qualified UTC |
| TLS/authentication | Wrong host/CA, expired/not-yet-valid cert, bad SCRAM proof, missing client identity, entropy failure, rotation; no plaintext downgrade |
| Storage/media | Tenant object-ID guessing, unauthorized create-at/delete, corrupted calibration/catalog/snapshot and restart; no cross-tenant access or unauthorized recovery |
| DMA/lifecycle | Malicious/stale DMA, pin/map/drop races, reset/revoke, incomplete quiescence, requester aliases, interrupt flood; frames not reused while reachable |
| Build/signing | Locked clean rebuild, pinned actions, advisory scan of resolved graph, process/log inspection with fixture keys; no private key in argv/logs |

Formal modelling should complement those tests. Extend authorization/launch
models with the actual occupancy's descriptor digest, so a signed policy for a
different version cannot be treated as interchangeable. Model mailbox channel
ownership and kernel/user mapping partitions explicitly. Resource-reserve and
pending-acquisition models can check that a tenant cannot prevent control-plane
progress. Existing cluster/DSR models should continue to state whether peers
are authenticated and non-Byzantine; model safety within that assumption must
not be reported as protection against MAC spoofing.

## Relationship to earlier reports

This audit does not re-label every finding in the July/August reports as still
open. Generation checks, launch rollback, relmsg bounds, socket ownership, and
signed operational admission have evolved. The findings above refer to the
current reviewed source, including problems in newer paths and remaining trust
assumptions. Consult the [reports index](../README.md) for the historical
audits and the DMA investigation; use current source and new validation evidence
to close these finding IDs.
