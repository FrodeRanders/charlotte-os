# Renewed security audit remediation — 2026-10-05

This records corrections to SEC-19–22 in the
[renewed audit](2026-10-05-security-audit.md), based on revision
`62bc8f011ff45f66ac4531f534c6d63f98229c19`. The audit remains a historical
record of the reviewed code. The implementation and validation recorded here
accompany this report; the earlier audit's residual findings retain their
separate scope.

## Disposition

| Finding | Disposition | Result |
| --- | --- | --- |
| SEC-19 | Remediated | Fallible construction/publication, pre-construction stack admission, exclusive per-domain arena and retained retirement ownership |
| SEC-20 | Remediated | EL0 watches authorize the exact caller domain under target serialization; trusted kernel observation remains separate |
| SEC-21 | Remediated | Exact-owner liveness reclamation, failed-delivery rollback, unused-socket expiry and platform capacity reserve |
| SEC-22 | Remediated | Entropy-gated startup, fresh smoltcp seed and independently keyed ephemeral-port selection |

## SEC-19: fallible thread preparation and exclusive stack ownership

`Thread::try_new` returns errors for user-context preparation, context-box
allocation and generation exhaustion. Thread-table publication prepares its
growing metadata fallibly and returns rejected payload ownership to the
caller. Scheduler serialization is released before stack destructors or exit
callbacks can run on publication failure. A host fixture verifies rejection
returns the owner without invoking its destructor. `SPAWN_THREAD` propagates failure as
`(u64::MAX, 0)`, and `ThreadHandle::spawn` reports `ThreadError::SpawnFailed`.
Invalid user entry ranges and LP numbers reject before construction. The
scheduler's initial admission error path checks the captured thread generation
before removing an unadmitted thread, so it cannot remove a recycled successor.

Both architectures use the root's inline 64-bit stack-slot bitmap rather
than a global ever-increasing index. Reservation precedes backing allocation
and counts preparing/retiring stacks against the domain's thread limit.
An exact `AddressSpaceOperation` remains with the slot, preventing root/ASID
reuse throughout preparation and physical retirement. `PreparingStackPage`
owns both provisional slot admission and a zeroed physical page; allocation
respects the existing physical progress floor. Abandonment during mapping
quarantines backing and admission rather than recycling reachable state.

Explicit memory-object and MMIO mappings, and pre-launch ELF validation,
exclude every stack slot and guard. Successful stack cleanup detaches leaves,
releases table guards, invalidates translations and releases frames before
returning the slot. Rejected cleanup retains the slot/root lease.
Fallible profile, deployment, observer and replacement launch paths retain an
unstarted namespace transaction through thread preparation and roll it back
on failure. Trusted mandatory fixtures retain a panic wrapper.

Tests exercise forced collision and preservation of the original page,
pre-construction quota rejection without backing consumption, injected
initial allocation rejection, all 64 simultaneous reservations, repeated slot
reuse, failed initial launch rollback, and stale ASID rejection. The EL0
scoped probe verifies a signed one-thread limit rejects another spawn while
its main thread continues.

See [user thread admission](../../reference/user-thread-admission.md).

## SEC-20: caller-scoped exit observation

Each thread retains its publication domain handle. The EL0 completion adapter
admits the caller's exact root before subscription, then compares the target's
captured domain under the master thread-table lock. This authorization check
precedes thread-generation classification, so neither a zero generation nor a
mismatched generation can qualify a foreign live thread. Rejection drops
provisional completion/watch admission and does not occupy target-source slots.
Absent same-domain join targets still complete immediately. Kernel-only
supervisor/worker adapters preserve trusted cross-domain observation and the
existing cancellation semantics.

Kernel regressions repeatedly attempt foreign user/kernel watches with zero,
matching and wrong generations; verify zero foreign source occupancy; retain
same-domain joins and trusted watches; and check a recycled TID. The EL0
scoped probe also attempts to observe the live foreign name-service thread.
Existing exit/reuse/cancellation race fixtures remain enabled.

See [close-watch budgets](../../reference/close-watch-budgets.md).

## SEC-21: sockets follow remote owner lifetime

The trusted TCP/IP launcher designates the service's exact root before
scheduler admission. A new scalar `SocketOwnerStatus` query is available only
to that designation. The kernel checks authenticated ASID, generation and
principal, holds an exact root lease, checks the abort/publication gate and
live thread table, and returns authoritative platform classification. Ordinary
EL0 callers receive denial. Application names, roles and manifests confer no
query or platform-reserve authority.

The progressing TCP/IP reactor removes all sockets whose exact owner is dead,
closing, aborting or replaced, regardless of protocol state. This includes
fresh sockets, TCP listeners and bound/open UDP sockets. Socket creation and
ID delivery use one registry publication helper: rejected or cancelled reply
immediately removes the record and smoltcp handle. A successfully delivered
but unobserved, unactivated socket expires after five seconds. Activated
sockets survive that idle rule; owner death still releases them.

Deferred receives now retain owned `ReplyToken` values and use owned receive
memory/mappings. Removal releases retained reply authority without an integer
cleanup ladder. Graceful TCP close retains its independent five-second grace.
Sixteen application slots, or the whole capacity of a smaller table, are
reserved for kernel-classified platform domains. Ordinary clients cannot
consume that reserve even if a launch policy raises their per-owner ceiling.
The ordinary default per-owner count is reduced from 64 to 16.

Host tests exercise the actual production registry against fixed smoltcp
storage: 1,024 cancelled TCP creations, subsequent slot reuse, fresh/listening
TCP and open UDP owner-death reclamation, exact-generation successor isolation,
buffer-count recovery and unused-socket expiry. Reserve tests cover capacities
0–1,024. The guest verifies that ordinary callers cannot use the privileged
query and that normal TCP/IP clients continue to operate.

Remote cleanup requires a progressing service reactor. The socket buffers
remain charged to TCP/IP's own heap; cross-domain kernel byte sponsorship is
still separate work. New TCP/IP instances must use the trusted designated
launcher; arbitrary generic service launches do not receive this authority.
No remote socket/session migration across a TCP/IP service replacement is
introduced.

See [smoltcp adapter](../../reference/smoltcp-adapter.md).

## SEC-22: fresh protocol randomness

TCP/IP obtains 40 complete entropy bytes before constructing the smoltcp
interface. It uses the architectural random syscall first and the owned,
capability-mediated VirtIO RNG service for fallback. Missing or failed entropy
withholds networking; there is no deterministic/clock/MAC fallback. Eight
bytes seed smoltcp. A separate 32-byte key supplies HMAC-SHA-256 counter-based
port selection over 49152–65535, with unbiased 14-bit reduction. Interface-seed
knowledge does not reveal the independent port key. Key material is wiped.

HMAC and SHA-256 are direct dependencies on versions already present in the
lockfile; no dependency version was changed. Host tests reject complete and
partial entropy failures, distinguish boot states, verify port bounds and key
independence. The AArch64 guest boots with host-backed VirtIO entropy and
completes normal networking and scoped-client checks.

This removes public deterministic startup state; it does not change smoltcp's
internal PRNG into a cryptographic generator, prove off-path attack resistance
for every protocol, or authenticate peers.

See [entropy](../../reference/entropy.md).

## Validation

- `cargo fmt --all -- --check` and `git diff --check`: passed.
- `scripts/run-host-tests.sh`: passed. New coverage includes nine socket/random
  tests and the application stack-arena exclusion test; existing owned-runtime,
  protocol, authorization, signing, retirement-slot and scratch tests pass.
- `scripts/build-catten-services.sh --embed`: passed; the signed bundle includes
  the corrected syscall emitter and renewed EL0 probe.
- Workspace Clippy with `-D warnings`, excluding service/user/host-tool packages,
  passed for both custom AArch64 and x86 kernel targets. Bundled AArch64 service
  Clippy with `--bins --lib -- -D warnings` passed.
- Final isolated AArch64 guest: **19 passed, 0 failed, 0 pending**, both scoped
  probes reported `checks=0xffff`, service publication generations advanced
  through 1 and 2, and the cancellation sender completed 4,484 requests before
  retirement. Synchronous thread/loader regressions also passed.
- Updated-lockfile cargo-audit: **177 dependencies, zero advisories matched,
  zero warnings**. RustSec database had 1,290 advisories, commit
  `ef6173cbc5c50ec8166f9a5b28f07834144373ee`, updated
  `2026-10-03T10:14:03+02:00`. Machine-readable result:
  [dependency scan](2026-10-05-security-remediation-dependencies.json).

The final guest command used an isolated fresh-storage instance and existing
rebuilt service bundle:

```sh
CATTEN_SKIP_SERVICE_BUILD=1 CATTEN_HTTP_HOST_PORT=18285 \
CATTEN_DEPLOY_HOST_PORT=17585 scripts/run-aarch64.sh --security-test \
  --instance remediation-20261005-complete --fresh-storage --timeout 130
```

An initial sandbox guest could not bind forwarding ports. A subsequent guest
caught a missing AArch64 emitter for the new syscall; it was corrected before
the successful runs. The final run includes the provisional stack owner, allocation-failure
regression and publication-error destruction outside scheduler guards, rather
than relying on the earlier passing image.

Validated artifact SHA-256 values:

- kernel: `49d18dd49ca5e8131876c2ac338220dc45ec2a20f742ae1a2b1d9bde368c59ea`
- `tcpip.elf`: `b18a2fb8bdc001d4b8a51ef5e3cf43d49fde979ba53987e77d93ab80487699f3`
- `Cargo.lock`: `4b225cd56b859513c0eb9139fecd0986efad2c1409c67fb99b0f97681a3fdb82`

There was no x86 guest execution, physical-device fault injection, deliberate
whole-node physical/table exhaustion, dedicated simultaneous EL0 spawn stress,
or live socket-owner death/flood campaign. Mapping collision and initial
allocation rejection were kernel fixtures; socket reclamation/failed delivery
were production host fixtures plus guest liveness integration. These limits
qualify the evidence, not the implemented rejection/ownership contracts.

## Remaining audit scope

SEC-19–22 corrections do not close the earlier audit's broader resource and
bulk-retirement gaps, cluster peer authentication, authenticated security time,
management authentication or production provisioning. Keep the deployment
restrictions and remaining work recorded in the original remediation ledger
and renewed audit. Passing these tests is not a production security certification.
