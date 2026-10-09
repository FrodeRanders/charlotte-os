# CharlotteOS test paths

CharlotteOS has two intentionally different test environments. Pure logic
should run on the development host. Kernel behavior and EL0 entry-point wiring
must run on the CharlotteOS target, normally under QEMU.

## Host-testable logic

Run every host-side Rust suite and tool self-test with:

```sh
scripts/run-host-tests.sh
```

The runner currently covers:

| Component | What is tested |
|---|---|
| `charlotte-authorization` | principal binding, role separation, default deny, attenuation, policy and service fencing, one-shot redemption, and bounded state |
| `charlotte-protocol-disco` | discovery encoding, decoding, and malformed input |
| `charlotte-protocol-msg` | v3 framing, checked parsing, 32-bit message lengths and fragmentation offsets, typed IPC envelopes, and session fencing |
| `charlotte-protocol-net` | NIC status decoding |
| `catten-graft` | Raft election, membership, joining, snapshots, persistence projections, and wire format |
| `charlotte-smoltcp` | receive-queue bounds and clock progression |
| `charlotte-launch` | checked user-address mapping ranges, alias/kernel/null rejection and overflow boundaries |
| `catten-services` | shared authorization/client logic, resource adapters and total monotonic deadlines |
| `cluster-sign` | restricted key-file handling, digest, signed metadata, and placement-policy self-tests |

The script invokes Cargo from a temporary directory. This is necessary because
the repository's `.cargo/config.toml` asks Cargo to rebuild `core`, `alloc`, and
`compiler_builtins` for freestanding targets. If an ordinary host test is
started from the repository tree, Cargo discovers that target configuration
and may link a second copy of `core`. The wrapper keeps the pinned toolchain and
absolute manifests while preventing the bare-metal configuration from leaking
into host tests. It discovers crates containing `#[test]`; library targets
containing such tests must enable their host harness. A `test = false` library
can otherwise cause Cargo to skip those unit tests while still successfully
running documentation tests. Check the test names/counts when adding a suite.
Freestanding binary targets keep their harness disabled. CI calls the same
wrapper.

## Target-only tests

The `catten` kernel, `catten-user`, and the binaries in `catten-services` are
`no_std`/`no_main` target programs. Cargo's host test harness is not their
execution environment. Their target declarations therefore retain
`test = false`; kernel and service integration behavior is exercised by
`scripts/run-aarch64.sh` and the boot-time self-test registry.

The runners' ordinary runtime configuration is intentionally separate from
their verifier selection. They attach a NIC by default, so DHCP, discovery,
cluster, TCP/IP, HTTP, and time services run even when no network-test feature
is compiled. `--net-test`, `--dhcp-test`, `--disco-test`, and related options
register additional target verifiers; `--no-network` is the explicit runtime
opt-out. Tests should never be the mechanism that enables a production
capability.

### Scoped-launch security verifier

```sh
CATTEN_HTTP_HOST_PORT=18081 CATTEN_DEPLOY_HOST_PORT=17445 \
  scripts/run-aarch64.sh --security-test --instance security-grants --fresh-storage --timeout 100
```

This verifier needs ordinary networking and TCG/KVM, not HVF. It cannot be
combined with isolated shutdown, upgrade or ingress suites. Fixture generation
creates fresh, independent artifact and deployment signing roots in a private
directory under ignored `target/security-test`. Private key files are mode 0600;
only public keys, signed descriptors and the signed probe ELF enter the test
kernel. These roots test the configurable launcher path; they do not provision
production trust or replace the bundled platform-service development root.

The application receives its admitted descriptor and grant-controller
connection, without a name-service connection. A second valid descriptor names
the same ELF but widens tcpip rights. The controller must reject this replacement
policy, undeclared services, excess rights and re-delegation of client authority.
Positive controls establish publication and ordinary calls. Missing targets and
cancelled requests must not prevent acquisition of the available endpoint;
a separately hosted silent endpoint tests the publication helper's deadline.

The primary probe performs 384 cancelled requests in each of two launches. A
concurrent probe submits at least 512 more. All fifteen result bits (`0x7fff`) must
pass, publication generations must advance, and the retired descriptor must no
longer attest. The ordinary boot suite, network verifier and security verifier
register 19 tests in this configuration.

The eleventh bit exercises aggregate memory-object admission: the primary
holds many one-page allocations until allocation is refused, completes scalar
IPC while its allocation budget is full, drops the owning batch, and acquires
and calls its service again. Synchronous memory-object tests separately check
page and object-count ceilings, transfer/rollback sponsorship, late unpin after
retirement, and the platform progress pool.

The twelfth bit fills completion-timer capacity with owned hour-long timers,
drops the batch within five seconds, churns 64 additional hour-long timers and
waits for a short timer after recovery. Synchronous timer-admission tests also
exercise platform reserve, deferred event reclamation and exact numeric
namespace reuse. See [completion-timer budgets](../reference/completion-timer-budgets.md).

The thirteenth bit fills the application's endpoint budget, checks scalar IPC
while it is full, drops the owning batch and performs 128 create/drop cycles.
Synchronous tests check queue backing limits and resize rollback, platform
reserve, retained delegated records, unobserved-return cleanup, and forced
ASID reuse. See [endpoint budgets](../reference/endpoint-budgets.md).

The same thirteenth bit now fills connection admission and submits 512
outstanding calls to a larger queue, using owning batches. Dropping the calls
must permit a successful request; dropping the connections must permit another
128 mint/drop cycles. Kernel fixtures separately exercise retained completed
calls, all attachment variants' pre-transfer rejection, failed-attachment
rollback, live loans after rejected replies, observed/unobserved returns,
queued delegation cancellation, retirement publication fencing and ASID reuse.
Ordinary/total node saturation uses counters, not full registry allocation.
A host owner test checks token cancellation without closing its grant source
on a resource-limited reply. See [IPC record budgets](../reference/ipc-record-budgets.md).

The fourteenth bit fills shared completion-record capacity with owned
endpoint-close watches. Timer submission must fail while the record budget is
full, scalar IPC must still work, and closing the watched endpoint must allow
all watches to complete and a short timer to run. Synchronous tests cover
retained strong and weak references and detached results, ordinary/total record pools,
platform progress, CQ replacement, retirement and a stale captured close after
exact numeric ASID/capability reuse, including an old weak allocation that must
not refund the successor's account. Six host tests execute the kernel's actual
charged allocator for lifetime, allocation-failure destruction order, clone
admission and concurrent final weak release. See
[completion-record budgets](../reference/completion-record-budgets.md).

Kernel-only CQ admission tests additionally exhaust queue counts and kernel
backing bytes, check failed replacement preserves pending data and capabilities,
reject physical-ring aliases without resetting the ring, and exercise retirement
and generation reuse. A signed domain load fails partway through installing its
five CQs, returns the staged charges and frees its ASID; trusted platform
preparation then succeeds using the reserve. These are synchronous tests, not
another EL0 probe bit. See
[completion-queue budgets](../reference/completion-queue-budgets.md).

The fifteenth bit drops a full batch of owned endpoint-close watches while its
endpoint remains live, then churns 128 more watches within five seconds. A new
watch must still be pending until the endpoint closes; a short timer checks
recovery. Synchronous registration tests cover entry limits, rollback,
detached notification storage, callback reentrancy, a late callback after
cancellation, retirement and reuse. See
[close-watch budgets](../reference/close-watch-budgets.md).

Thread-exit fixtures keep that mask unchanged. Synchronous kernel tests fill a
128-entry target, check transactional rejection, run 512 cancel/rearm cycles,
observe normal/stale-generation exit, reenter registries during callbacks, and
exercise retained tokens, shared-account exhaustion and cancellation before
owner installation. Actual address-space/capability reuse checks late-install
rejection, and failed worker setup must not execute or notify. The scheduled
CQ verifier cancels/rearms 128 watches against a live target, joins its eventual
exit, runs 32 immediate-return workers and verifies deferred cancellation of a
held worker. These are kernel fixtures, not additional real-EL0 quota probes or
physical allocator-failure injection.

Kernel completion-callback fixtures check 128-entry source saturation, full-batch
notification with retained owners, 512 cancel/rearm cycles, shared-watch rejection,
reentrant/late delivery, buffer preservation, producer cancellation, rollback
with retained objects and exact namespace reuse. Injected entry-allocation failure
checks charge/weak-reference rollback and recovery; it does not exhaust the
allocator or inject every allocation failure. Both actual boot-status sources
also check 64-entry limits, rearming and detached notification before schedulers
start. The scheduled verifier checks 64 publication and 64 result timeout cycles
with an isolated waiter sponsor, then races registration against 32 immediate
workers. The scoped mask remains unchanged. See
[kernel callback budgets](../reference/completion-callback-budgets.md).

Deferred IRQ host tests cover per-route coalescing under a flood, retirement
watermarks, fresh publication after claim, binding identity exhaustion,
concurrent stale publishers and competing claimers. The device kernel fixture
delivers more than twice the former shared queue capacity before route reuse,
checks that retired readiness cannot reach the replacement and verifies a
fresh wake/ack. Another fixture detaches a CQ wake, reuses its exact numeric
ASID/CQ and checks that notification cannot select replacement waiters or
change replacement work generation. Both this callback and an actual deferred
IRQ callback reenter completion/device registries. Live driver IRQ rounds
remain part of the device/UART tests.
These are not exhaustive controller MMIO or cross-LP mask/rearm checks. See
[interrupt wake storage](../reference/interrupt-wake-storage.md).

Mailbox kernel fixtures exercise the actual syscall dispatch path with 512
mixed sender/receiver handles, over-limit rejection, receiver reuse at capacity,
slot recovery and 1,024 open/close cycles. Serial-exhaustion injection checks
staged refund without damaging existing authority. Real domain handles test
retirement and late publication after ASID reuse; a retained charge cannot
credit the replacement. Isolated production counters check ordinary/node
ceilings and the platform reserve. No extra real-EL0 quota probe was added;
the scoped mask remains `0x7fff`. See
[mailbox capability budgets](../reference/mailbox-capability-budgets.md).

Shared capability-admission fixtures fill 4,096 actual entries across all six
object kinds. They cover hidden staging, cancellation, quota rejection without
serial consumption, source escrow/rollback at full capacity, committed source
revocation and exact-number namespace replacement. Real domain registration,
retirement and ASID reuse test stale-token publication and captured-generation
rejection. Mailbox dispatch and completion/timer submissions reject aggregate
pressure below their family ceilings, refund staged family charges and recover
when a slot is freed. An isolated counter test checks shared node/ordinary
limits without filling the live pool. Every capability kind now has bounded
allocation; the temporary bypass and its permissive fixture are removed. No extra EL0
quota probe or atomic IPC-vector admission test is implied; the scoped mask
remains `0x7fff`. See
[shared capability admission](../reference/capability-admission.md).

Device fixtures reject MMIO/IRQ/DMA grants at the shared ceiling, recover real
MMIO mapping and IRQ grants after freeing a slot, and fence exact ASID/handle
reuse. Fake DMA backends verify admission before creation, creation-failure
refund, retirement before publication and exactly-once destroy, including a
simulated destroy failure. These do not test real IOMMU ACK-timeout/quarantine.
Observer fixtures check quota recovery, stale generations and cancelled startup
claims using isolated atomics, without resetting the already-live observer.
Pre-bootstrap connection and observer rejection reclaim the entire unstarted
fixture namespace and delegated metadata. Real boot still exercises successful
observer launch and telemetry access. These are kernel fixtures, not new real
EL0 quota probes or production hardware pressure tests.

Real-domain IPC shared-admission fixtures check endpoint/direct-grant rejection
and family refunds, receiver reply admission under namespace pressure, unchanged
queue/result bytes and active loans, retry/cancellation and one-way receive at
the ceiling. Invalid/read-loaned result pages return speculative reply authority
without consuming queued work. A paused reservation checks rejection after
retirement before publication. This is kernel registry/ABI testing, not a new
EL0 quota probe or exhaustive concurrent retirement exploration.

Call-side fixtures reject all scalar/vector variants at the caller ceiling,
reject receiver attachment admission after staged call metadata, and test a
connection+copy with one versus two receiver slots. Paused copy-only/four-mode
calls test caller/receiver retirement before joint publication; every staged
IPC/memory handle remains inaccessible and original sources/charges recover.
Invalid copied buffers and mapped-source move/write-loan rejection refund
metadata. A prepared call survives sponsor teardown and exact ASID/handle reuse;
late failure returns old charges without changing a successor's pending call.
Returned-connection pressure preserves its solicited reply/loan for retry, and
observed returned authority survives pending-call close. These are kernel
fixtures, not new real-EL0 quota probes, allocator-failure injection or an
exhaustive concurrent proof.

Memory-object fixtures also fill spare shared-namespace slots in real domains.
They check allocation/copy/read-loan/write-loan rejection, source access and
backing refunds, prepared-move cancellation at the source ceiling, successful
two-object commit and all-or-none rejection after destination retirement.
Paused source retirement checks rollback to original slots for cleanup. Source
and destination teardown/reuse fixtures assert exact numeric ASID/capability
reuse, old-frame retention until cancellation, and unchanged successor budgets
and authority. Existing IPC vector/reply tests exercise the integrated move
owners.

Additional fixtures pause hidden copies and both loan modes, attempting guessed
handle lookup/map/write/close, physical queries and DMA/copy pinning. They check
private-copy refunds, snapshot independence, loan cancellation at the source
ceiling, read-only source mapping, mixed-mode retirement, and exact source/target
ASID/capability reuse. Real kernel vector calls exercise all four modes, failure
after each prepared kind, denied private-copy backing after a staged loan,
mapped vector loans on reply/cancellation, reply-token close and queued endpoint
close. Queued copies/moves are reclaimed, delivered ownership is retained, and
both vector loans end. Kernel snapshot/write/DMA checks reject owner access
forbidden by committed loans. These deterministic kernel fixtures are not a new
EL0 quota test or exhaustive concurrent-retirement proof.

Boot runners require a complete newline-terminated authoritative result before
stopping QEMU. The shared parser accepts LF or CRLF, recognizes complete failed
verdicts as terminal, and requires zero failed/pending counts and bitmaps for
success. Its host tests reject every partial prefix of a successful record,
malformed/truncated results and kernel panics, and allow a later partial log
line after an already complete verdict. The poll loop caches completion, so
long application holds do not repeatedly launch the parser.

Real-domain fixtures use `register_user_address_space`, not direct insertion
into `ADDRESS_SPACE_TABLE`. This initializes the same generation, limits,
accounting and capability namespace as production domain creation. Adversarial
IPC scenarios using pseudo-domain IDs allocate no unused real address spaces.

Completion/CQ waiter tests keep the fifteen-bit scoped mask unchanged. Kernel
tests cover source/domain/node admission and rollback, detached entries,
promotion and ASID reuse; the scheduled CQ verifier checks Running/Ready/new
rejection, wake/reap cleanup despite a retained Waker and 64 completion/CQ timeout cycles.
Host owner tests check retry and Drop after timed admission failure. Comprehensive
allocator-failure and arbitrary callback-capture accounting remain outside this coverage.
See [scheduler waiter budgets](../reference/scheduler-waiter-budgets.md).

IPC waiter tests also leave the scoped mask unchanged. They cover source
ceilings, 512 cancel/rearm cycles, reentrant notification after message/reply/
source closure, reply-token cancellation, revocation of a live loan and retired
sponsorship. The scheduled verifier checks 64 reply/readiness timeout cycles,
non-mutating full-source rejection, and forces untimed receive/reply recovery
by withholding the helper's message/reply until an admission retry is recorded.
The reply helper checks that its loan is still available before replying and
the caller checks it is revoked on return. These are kernel-only fixtures.

Blocking-lock fixtures likewise keep the scoped mask unchanged. Synchronous
tests check mutex/reader/writer ceilings, cancel/rearm, detached charges,
source destruction, retirement, reentrant callbacks, expired writers and
final-reader notification. Scheduled tests force each lock fallback counter
before allowing remote contention to park, then verify acquisition/release and
64 timed cleanup cycles. Holders poll while a peer runs; they never explicitly
park/yield with a data guard. These tests do not establish fairness, owner-death
recovery, production use or allocation-failure coverage.

Timer waiter fixtures leave the scoped mask unchanged too. They cover source
limits/rollback, 512 cancel/rearm cycles, callback reentrancy, expired callbacks,
cancelled-event suppression, event destruction with retained tokens and retired
sponsorship. Scheduled tests run 64 normal sleeps and 64 competing-watchdog
cleanup cycles. A pre-filled timer source forces the normal sleep path into its
runnable fallback and checks elapsed time; rejection leaves state/constraints
unchanged. This is not a forced-quota EL0 syscall or timer-queue capacity test.

Scheduler timer-event fixtures separately allocate 1,024 sorted nodes, test
reuse and iterative removal, and keep the quantum inline while anonymous
admission is full. Shared node/ordinary saturation is counter-only. They cover
platform promotion, retirement, exact ASID reuse and an injected node-allocation
failure. Cancellation-backing fixtures remove real nodes with a retained handle
and 128 weak aliases, reject admission until final release and preserve the
successor's account after exact ASID reuse. Preparation rejection covers both
handle/event destruction orders. Scheduled tests substitute only their current kernel thread's sponsor
to force event rejection before parking, check generic/CQ waits and synthetic
timed-completion status 3 with the pending capability retained, then cancel and
close it. Sleep/watchdog cycles reconcile event charges; busy-local cancellation
retains its charge until purge. A cancellation-owner fixture simulates relocation
before publication, not actual remote-LP reclamation. See
[scheduler timer-event budgets](../reference/scheduler-timer-budgets.md).

Observer-list allocation fixtures retain empty lists with tokens and 128 weak
aliases, inject construction rejection and check captured platform classification
through sponsor promotion/retirement. Counter-only node pressure rejects list,
completion, CQ, callback, endpoint and call preparation, while a real platform
waiter progresses under ordinary pressure. Release permits retry and exact
counter reconciliation. These are serialized kernel fixtures, not physical OOM
or an EL0 many-client flood. See
[observer-list admission](../reference/observer-list-admission.md).

The scoped application verifier does not force ASID reuse, restart the grant
controller, inject allocation failures or prove many-client fairness. It does
not exercise the complete S3/Raft release pipeline with independent roots, or
establish production key custody.

The HTTP verifier also runs EOF and idle-client availability probes before the
node/cluster JSON checks. Use an isolated instance and unused forwarded ports:

```sh
CATTEN_HTTP_HOST_PORT=18080 CATTEN_DEPLOY_HOST_PORT=17444 \
  scripts/run-aarch64.sh --http-test --instance security-http --fresh-storage --timeout 75
```

`scripts/tests/test-http-keyhole-liveness.py` checks EOF and abortive peer-close
recovery, and that an idle client is timed out while subsequent metrics
requests are attempted. Transport retries are bounded and spaced to span that
idle budget. The
five-second request deadline bounds this serial listener's per-client delay;
it does not establish resilience against a sustained connection flood.

![Ordinary boot and optional test validators](../manual-v2/figures/boot-and-testing.svg)

The two-guest `--relmsg-test` verifier sends and compares a 70,000-byte
payload. This intentionally crosses v2's 65,535-byte limit and exercises the
v3 IPC envelope, fragmentation, adaptive retry, reassembly, and cumulative
delivery acknowledgement end to end.

The cluster-wide TCP data path has a dedicated host-visible fixture:

```sh
./scripts/run-distributed-ingress-test.sh
```

It creates a shared stream-backed L2 segment with three guests and an
independent host-side Ethernet/TCP client. The test requires committed stable
three-voter membership, deterministic selection across all three backends,
remote one-hop frame forwarding, leader/VIP-advertiser loss, replacement
gratuitous ARP, and a request completed over a previously established flow on
a surviving backend. It then requires a fresh connection and complete HTTP
exchange with a live backend, demonstrating reconnect after the failed
advertiser also took one backend connection with it. `--cluster-ingress-test`
is used only by this harness; ordinary operation is enabled by giving every
member the same repeatable `--cluster-service [NAME=]VIP:port` bootstrap policy.

`--s3-test --timeout 240` is an explicitly test-only integration fixture. It
adds a local TLS RustFS Docker container, a provisioned S3 service profile, and
an in-guest PUT/HEAD/GET/DELETE verifier. Network, DHCP, time synchronization,
and the S3 client itself are ordinary services; the switch supplies the
ephemeral external server, test credentials/CA, and pass/fail observer. The
VirtIO RNG device and entropy service are part of ordinary QEMU operation, not
test-only support. See
[S3 client service](../reference/s3-client.md#rustfs-integration-test).

`--deployment-ingress-test --timeout 240` extends the same fixture with the
release path. The host uploads the signed `greet` ELF to RustFS, signs and
wraps its `CDEPLOY5` descriptor in a signed `CRELEASE` envelope, atomically
admits the release, and waits on the management API until the exact desired
generation owns the active service name on its assigned node.

The two-guest `--deploy-test` exercises the runtime retirement paths. It moves
the lifecycle-aware `greet` domain between nodes and requires an acknowledged
cooperative exit, then returns the assignment with a test-injected zero grace
period and requires forced termination, generation-safe reaping, and continued
reachability through the replacement generation.

`./scripts/run-distributed-ingress-test.sh` forms a three-member cluster and
also validates the placement-control reporting path. Before failover, one
leader must seed fresh filtered capacity state for all three members. After the
VIP advertiser/leader is stopped, a surviving leader must rebuild fresh state
for both remaining reporters; replicated samples from the former term do not
extend the leader-local lease. The fixture continues to check established-flow
survival, gratuitous ARP, and reconnect after backend loss.

For a deterministic test that does not require networking or cluster
formation, use:

```sh
./scripts/run-aarch64.sh release --shutdown-test --fresh-storage --timeout 120
```

The isolated verifier registers both probes with the real deployment-domain
retirement state machine. One drops an owned endpoint and acknowledges a
propagated `NodeShutdown` request. The other is deliberately unresponsive: its
signed child grace expires before the enclosing node deadline, so it must be
forcibly terminated and reclaimed. The test also requires the distinct
acknowledged/forced node-shutdown counters to advance. The same switch is
available through `scripts/run-x86_64.sh`. Three additional cooperative probes
exercise the generic node-service coordinator and require ingress, dependent
service, and storage phases to remain strictly gated; hardware-root domain
ownership must not become available until all three phases are reclaimed. A
fourth probe represents a hardware adapter: the device coordinator must publish
its request, observe the distinct `DEVICE_QUIESCED` acknowledgement and thread
exit, and only then reclaim its domain.

The verifier then exercises the production steady-state owners, not only the
probes. It drains and reclaims the real object store, transfers the actual NVMe,
VirtIO RNG, and (on an ordinary network-enabled run) VirtIO NIC domains, and
requires every retained driver to finish its device-specific flush, drain, and
reset path before the test can complete. AHCI, VirtIO block, and E1000E use the
same lifecycle contract and are compile-checked; their shutdown paths still
need dedicated platform fixtures for runtime fault injection.

The network-enabled shutdown fixture also exercises lifecycle-aware deployment
ingress, HTTP ingress when present, and UTC time domains. They are idle at the
drain boundary, so the test proves that their bounded socket/NTP waits observe
the request and release their owned resources without delaying the later
storage and device phases. `--http-test` separately sends a real host request
through the bounded receive path and validates the complete response.
The shutdown fixture also requires TCP/IP and the frame router to release
their protocol sockets, deferred reply tokens, pending frame transfers, and
NIC connection before the VirtIO NIC is reset. It deliberately begins the
production drain before publishing the boot-ready marker and asserts that
HTTP, time, and TCP/IP interrupt that startup wait and acknowledge normally;
per-phase outcome counters distinguish this from forced termination.
The same assertion covers deployment ingress/control/agent and
DNS/reliable-messaging/discovery, so every planned high-level production phase
must report one acknowledged exit and no unacknowledged or forced exit before
device ownership can transfer.

On AArch64 this fixture is also the terminal-poweroff test. After the
authoritative success record, the kernel selects the SMC/HVC conduit from the
ACPI FADT and invokes PSCI `SYSTEM_OFF`. The runner treats an early QEMU exit as
an error and accepts a powered-off guest only when both records are present.
The x86-64 switch currently validates drain and device quiescence but does not
yet exercise an ACPI S5 transition.

The ordinary no-network AArch64 suite exercises the ownership-aware object
store with the real NVMe service: a 12 KiB PRP-list block round trip, a 2 MiB +
4 KiB persistent object round trip, and Raft recovery across a process restart.
This catches regressions in the same owned memory, mapping, borrowed-call, and
reply-token paths used by the final shutdown flush.

`--kafka-test --timeout 300` similarly adds a disposable three-broker Apache
Kafka KRaft cluster with ephemeral verified TLS listeners and an in-guest verifier.
The verifier covers the TLS handshake, idempotent production, bounded
read-committed consumption, aborted-record filtering, and an atomic
consume-transform-produce transaction with the consumer offset included. The
runner creates a fresh single-partition `charlotte-events` topic and removes
the fixture and its volumes on exit. See
[Kafka client service](../reference/kafka-client.md#docker-integration-test).
Use `--kafka-coordinator-test` to hard-stop the transaction coordinator chosen
by Kafka, or `--kafka-fencing-test` to start a second connector with the same
transactional identity and require the stale producer to fail closed.

Both architecture runners source `scripts/lib/boot-common.sh` for dependency
validation, Limine configuration resolution, payload hashing, atomic FAT image
construction, and authoritative self-test verdict validation. To create only a
mount-free UEFI boot image from an already-built kernel, use
`scripts/create-boot-image.sh`; the Justfile's `create-image` recipe delegates
to the same implementation.

Long-running external harnesses can retain a live guest for post-failure
inspection without depending on architecture-specific command-line switches.
Set `CATTEN_QEMU_DEBUG_STUB=1` and pass `--gdb-port PORT` to expose the QEMU GDB
stub, `CATTEN_QEMU_PID_FILE=PATH` to record the QEMU process ID,
`CATTEN_QEMU_MONITOR=1` to create the instance's monitor socket, and
`CATTEN_QEMU_NET_DUMP=1` to capture packets when networking is enabled. The
Kafka broker soak harness enables these controls and, on client or readiness
failure, detaches from its still-running runner instead of killing the guest.
Its `--cleanup-on-failure` option restores teardown behavior for unattended
automation.

`catten-rt`, `catten-syscall`, and `charlotte-launch` also retain disabled
standalone harnesses. They contain target runtime/ABI support and currently
contain no dormant `#[test]` functions. Their host-compatible portions are
compiled as dependencies of the `catten-graft` and `charlotte-smoltcp` suites.

At the 12 August 2026 audit, `charlotte-protocol-net` was the only component
that had an actual Rust unit test hidden by `test = false`. Its harness is now
enabled and included in the shared runner. `charlotte-protocol-disco` already
had an enabled harness, but was missing from CI; it is now included as well.

## Demand-backed heap tests

Demand-heap admission fixtures check isolated node/ordinary saturation and
platform headroom, provisional refund and account promotion, real zeroed
backing, a reduced one-page domain quota, repeated touch, retired commitment,
exact ASID reuse and post-teardown refunds. A kernel-only mapper adapter rejects
before publication and verifies that the real frame and reservation are freed.
These are synchronous kernel tests, not a new scoped EL0 quota bit or forced
physical exhaustion. See [heap admission](../reference/heap-admission.md).

## Loader backing tests

Loader admission fixtures check quota/duplicate mapping, zeroed and filled
backing, mapping rejection rollback, retirement and exact ASID reuse. A signed
name-service image fails after partial mapping at an injected one-page ceiling;
normal retry reuses the freed ASID and releases all image/runtime backing and
CQs on teardown. Layout fixtures check bounded headers, adaptive-heap exclusion
and oversized BSS planning. This does not inject physical exhaustion or add a
scoped EL0 quota bit. See [loader admission](../reference/loader-admission.md).

Constructor-failure injection verifies that `RootAllocationFailed` returns
before namespace publication, consumes no frames/backing charges and preserves
ASID capacity. The x86-only root fixture tests rejected allocation and successful
inactive-PML4 teardown; AArch64 cannot execute that architecture-specific test.

## Page-table lifetime tests

The synchronous VM fixture creates two private, never-installed hierarchies
aliasing one data frame across three sparse leaf tables per root. Sixteen
unmap/remap rounds must retain stable table counts, reject duplicate/repeated
operations and block promotion over empty tables, preserve the other domain's
translations, and return every private table at teardown. See
[page-table lifetime](../reference/page-table-lifetime.md).
The higher-half VM fixture also flushes the final kernel page before releasing
its backing, guarding against exclusive-end overflow in range invalidation.
This checks physical ownership/reuse, not a concurrent hardware-walk race or
failed cross-LP shootdown. AArch64 guest execution does not execute x86 code.

## Translation-frame preparation tests

Per-walker fault adapters reject each private-tree allocation prefix, then check
retry and empty-table reuse with zero fresh-allocation allowance. A sparse
second branch fails after one table, retains its partial ownership, then
completes without rebuilding the prefix. Assertions check exact physical counts,
alias data preservation, unchanged active hardware roots, complete private
teardown and unchanged heap/image charges. ARM additionally rejects hardware-tag
admission after frame preparation and checks that no root/tag is published.
Scope/floor predicates and both zeroed-owner Drop paths are checked separately.
These fixtures retain no additional frames. See
[table preparation](../reference/page-table-preparation.md). They do not force
real exhaustion or prove publication unwind, hardware-walk races or x86 progress.

## Kernel data-retirement tests

Boot fixtures warm a retained kernel table subtree, then compare actual data
counts across detach, injected failed invalidation, successful retry and repeated
release. Partial allocation/map failures and a real foreign `AlreadyMapped`
leaf exercise rollback ownership. Bounds reject before mutation. The Drop
fallback fixture intentionally quarantines one 4 KiB page; this is a specified
test reservation, not a successfully released frame. See
[kernel data retirement](../reference/kernel-frame-retirement.md).

The x86-only fake IPI sender checks that failed delivery never decrements the
acknowledgement barrier. This compiles on x86 but requires its guest to execute;
it does not trigger a real delivery failure or the fatal halt path. Live
cross-LP virtual reuse, unresponsive recipients and lifecycle-lock retirement
remain separate validation work.

## Memory-object retirement tests

Memory-object retirement fixtures run in the single-mutator boot phase. They
inject the final copy/DMA unpin between detach and invalidation, check frame and
charge retention, borrower authority fencing, and scratch reuse after every
mapping barrier. Real collision leaves exercise clean and failed rollback,
installed-prefix retention and physical-identity checks. Partial detach, a
failed barrier, rejected scratch completion and abandoned receipts deliberately
retain **seven data pages and five object charges** for the guest lifetime, in addition to the kernel-range
fixture's one page. No test-only recovery bypass frees them. See
[memory-object retirement](../reference/memory-object-retirement.md). These are
interleaving fixtures, not concurrent hardware stress or x86 progress proofs.

The lock-separated detach fixture maps 35 pages and checks its three 16/16/3
frame batches. It exercises ordinary unmap, mapped-loan revoke and owner cleanup,
asserts registry/table guards are available at detach callbacks, rejects an
oversized prefix before table work, and injects the last copy unpin before the
first batch. Exact frame/charge counts return after completion, with no additional
quarantine. Whole-domain and bulk IPC cleanup retain their outer
serialization; standalone unmap and direct loan revocation now retain live
operation leases.

The scratch-completion fixture rejects one of two borrower range completions
after all invalidations. It verifies last-copy/DMA-unpin retention, safe reuse
of the completed range, non-reuse of the rejected range, loan fencing and charge
retention after all three domains close. It accounts for one of the seven
quarantined pages above. Six standalone host tests exercise the production
scratch allocator, including exact release rejection, metadata-preflight failure
and a 6,000-operation first-fit bitmap-oracle trace. Release does not allocate.
See [scratch admission](../reference/scratch-admission.md) for the remaining
registry-admission and generation-lease boundaries.

Direct loan-revocation fixtures retain both roots while checking rejection of
immediate close and completion of an admitted operation while staged borrower
close waits. They assert lifecycle/registry/table guards are available during
detach and invalidation, preserve other read borrowers, check exact scratch
reuse after the barrier, and release all root leases after failed preparation.
Rejected detachment, invalidation, scratch release and transaction Drop add
**four permanently retained data pages and four object charges**. All involved
roots close normally after these failure probes; backing remains quarantined.
The complete memory-object fixture therefore retains eleven pages and nine
object charges. These are deterministic interleavings, not a concurrent stress
test or hardware-quiescence proof.

IPC reply-ownership fixtures exercise two mapped loans, competing replies and
close of either the pending call or reply capability. An injected cooperative
wait completes the claimed reply while checking that IPC/lifecycle/table guards
are available; production uses the same wait loop with scheduler yield. The
loan callback checks those guards at detach, invalidation and scratch release.
Second-loan preparation rejection restores the first unstarted receipt. Second-
namespace lease rejection returns the first lease. Staged caller close waits
for successful reply completion. A three-loan fixture succeeds on the last loan,
injects failure on the middle one and restores the untouched first receipt.
It returns no result and retains one failed data page/object after namespace
teardown. Operation abandonment retains a second data page/object, both live
roots (including their table backing), IPC records and the reply claim. These
probes add **two data pages and two object charges** to the eleven-page,
nine-object memory-retirement fixture. No cleanup bypass is provided.
Deterministic interleavings do not establish multi-LP progress, recoverable
shootdown, or bulk IPC cleanup lock separation.

Returned-connection fixtures additionally close the exact minting source while
the reply claim is live, for both endpoint and attenuated delegated-connection
sources. They verify that the destination remains hidden until publication,
source-close waits outside IPC, observed results survive pending-call close and
unobserved results refund caller-sponsored grants. An unrelated endpoint-owner
domain closes during loan cleanup: its delegated source retains the closed
endpoint record without restoring service availability. Queued and unobserved-
result sources reject before loan/grant mutation, then become usable after
delivery/observation; closing their earlier call cannot reclaim the qualified
source. Quota rejection, second-loan preparation rejection and injected
pre-publication failure refund hidden authority/sponsorship. The staged-close,
partial-cleanup failure and abandonment probes also include returned authority;
they add no new quarantined data pages or root pairs. Abandonment retains its
source claim but refunds the unpublished grant. These remain serialized
interleavings, not concurrent source-close stress or additional model proofs.

Returned-memory fixtures retain output-source escrow and its backing pin through
unlocked input-loan cleanup. They verify source-close waiting outside the
registry, hidden destination authority, successful move and observed/unobserved
result ownership. Injected publication failure restores and closes the source
without an intermediate still-pinned state. Quota, second-loan preparation and
borrowed-output rejection preserve source authority; queued/unobserved sources
qualify only after delivery/observation. Staged close checks joint memory and
connection publication in the internal owner, without adding a combined wire
API. The existing partial-cleanup and abandonment probes include both output
kinds: they refund hidden admission and restore output memory without adding
quarantined data pages or root pairs. A committed read-loan preparation retains
its transfer fence until Drop, rejects another preparation, then permits a new
reader after pin release. Existing exact-ASID/capability-reuse fixtures also
check late Drop cannot clear a successor's fence. Paused fixtures use
`try_close_cap` for same-thread busy probes; actual waiting-close fixtures use
the production loop with a deterministic completion callback. These are not
concurrent multi-LP progress or x86 shootdown tests.

Explicit cancellation fixtures cover queued and delivered mixed read/write
loans, competing call/reply close, reply rejection, a claimed queue front's
`Pending` receive, and endpoint-close waiting outside IPC. Queued move/copy
attachments are reclaimed; delivered ownership survives call cancellation.
Claim-aware readiness registers a real endpoint waiter rather than reporting a
held front as ready. Cancellation exposes a following scalar message and invokes
the detached waiter callback with lifecycle/IPC/table guards available. Failed
queued cleanup remains unreceivable and not readable. The CQ wake uses the same
completion path, but this fixture does not independently stress CQ edges.
Cleanup callbacks assert IPC/lifecycle/table guard availability, and staged
root close remains pending until the cancellation releases its leases. Second-
loan and second-root admission rejection preserve the capability and restore
unstarted receipts. A three-loan partial failure records prior success, retains
the failed loan and refuses terminal publication; bulk reply-token cleanup
also cannot falsely report that failed loan as terminal. An abandoned queued
cancellation retains its claim, queue and both live roots. These two fault
probes add **two data pages and two object charges**, making the combined
memory-retirement/IPC fixtures retain **fifteen data pages and thirteen object
charges**, plus the separately described private tables/root/heap/kernel probes.
There is no recovery bypass. The host runtime tests check failed explicit close
returns a borrow-owning `PendingCall`, successful retry consumes it once, and
Drop/error-wait rejection reaches the fake fatal-domain boundary instead of
silently ending the borrow. The fake boundary unwinds only for observation;
another host probe verifies a `Pending` receive adopts no attachment owners.
production domain abort is non-returning. Concurrent multi-LP scheduling,
actual fatal-domain cleanup, x86 rendezvous and bulk lock separation are not
established by these deterministic probes.

## Final address-space retirement tests

Final-root retirement guest fixtures inject metadata-preflight rejection before
namespace/backing retirement, then check detached-root frame/heap-charge
retention, lifecycle/table guard availability during invalidation, registration
without leased-slot/tag reuse, exact physical release and stale-handle rejection.
Failed barrier and abandonment permanently retain two private roots, each with
one charged heap page and its private translation frames. Their frame counts
are logged. These are additional to earlier quarantine probes, not recovered
through test-only cleanup. Twenty-seven standalone host tests exercise the kernel's
generic slot owner, including table/generation identity, destructor ownership,
interleaved completion, preflight rejection and fail-closed completion metadata.
Four return-storage cases check publication rejection with the original payload
returned, mixed extraction/retirement without pointer/capacity changes,
allocation-free reuse and corrupted capacity rejected without repair.
`scripts/run-host-tests.sh` runs them even though the kernel disables Cargo's
test harness. See [final root retirement](../reference/address-space-retirement.md).
These fixtures do not prove x86 recipient progress or complete lock-safe teardown.

Registration guest fixtures additionally reject slot publication 64 times after
actual root, namespace-metadata and hardware-tag preparation. Returned roots are
released after lifecycle/table guards leave; physical frames and table charges
recover, existing namespaces remain live and the next successful registration
uses the exact expected slot/generation. The private adapters simulate metadata
rejection and instrument its release boundary, without forcing actual heap OOM.

Live-operation fixtures check overlapping leases, busy-close non-mutation,
retained mappings/tag/charges, continued admission, explicit release and
stale/detached acquisition rejection. Five new host tests check live counts,
identity, counter limits, vector growth and abandonment/table destruction.
Abandonment retains one additional live root with a charged heap page and private
table frames. Its namespace remains present and close stays busy. No production
masking guard was removed. See
[live address-space operations](../reference/live-address-space-operations.md).

Six staged-close host tests check fencing, retained close authority, existing
lease completion, metadata preparation rollback, capacity refresh and
fail-closed detachment. Guest fixtures poll a request with two then one lease,
reject new leases and competing closes, check retained mapped backing and
post-guard invalidation, and verify exact reuse after completion. Zero-budget
wait tests cover ready success and pending timeout. Timeout retains one more
closing root, one heap-page charge and private tables, software slot and ARM
tag. Finishing its last lease cannot reopen admission. No test-only recovery
path exists. These do not test nonzero wait scheduling, concurrent close stress,
production supervisor integration or x86 IPI progress.

## Owning-root physical-release tests

Both architectures share a boot fixture using their private production
destructor adapter. It rejects each of six releases (four private tables, heap
and image backing), then all six, before calling the real allocator. Assertions
check exact frame counts, continued cleanup, no second release, whole-account
charge retention, borrowed-root protection and a foreign leaf's independent
owner. Normal cleanup returns every owned frame and both charges. The seven
failure cases permanently reserve **12 physical frames and seven page charges
in each heap/image pool**; no recovery bypass frees them. These add to earlier
quarantine fixtures. See [root release](../reference/address-space-retirement.md#physical-release-and-charges).
They do not exercise real allocator corruption, panic/unwind recovery,
provisional frame rollback or x86 hardware progress.

## Joint backing-preparation tests

Serialized boot fixtures check heap/image admission, tracking and allocation
rejection; unused-owner Drop; map rejection; fill/zero preservation; and
successful ownership transfer. Failed-release adapters verify domain ceilings,
ordinary/platform pool identity, mixed live/quarantined root refunds, exact
generation reuse and non-reuse of retained physical frames. Installed-leaf
fixtures simulate interruption before and after account commit, refusing any
deallocation of uncertain publication. Abandoned charge receipts retain counts.
Twelve physical frames and six page charges in each heap/image pool remain
permanently retained, additional to earlier probes. See
[joint backing preparation](../reference/kernel-backing-preparation.md).
These simulate failure states, not real OOM, allocator corruption, panic
unwinding or x86 shootdown execution.

## Rule for new service logic

Keep the thin syscall loop and process entry point in an EL0 binary. Put policy
evaluation, codecs, state machines, bounds, and other deterministic behavior in
a dependency-free or narrowly dependent `no_std` library with an ordinary host
test harness. The authorization implementation follows this rule: the policy
engine is host-tested independently, while connection minting remains target
code and must not be enabled until the kernel supplies an authenticated,
generation-aware caller identity.

## Renewed security regressions (2026-10-05)

Deferred thread-retirement regressions additionally exercise four standalone
host list tests through `scripts/run-host-tests.sh`. A per-test-thread allocator
tracer checks zero allocations during staging, filtering, requeue and explicit
release of 1,024 prepared nodes. Abandoned nodes/lists retain their payloads.
Guest thread-admission fixtures check pre-stack node rejection, actual warmed
kernel-stack recovery, current-stack/LP-head retention, unlocked exit callbacks
and the retirement transition fence; Arm also checks a synthetic ownership flag.
The scheduled exit-watch fixture checks exact self-exit handle/context retention
before switching and completion afterward. Existing real remote-abort verifiers
still run. See
[thread retirement](../reference/thread-retirement.md) for fixture limits.

Whole-domain abort fixtures also check the inline root fence, rejection of an
already-prepared thread, repeated late preparation, callback guard availability,
TID replacement between capture and claim, ASID reuse and final root/frame
recovery. Repeated sweeps release their temporary root leases; stale, closing and
kernel targets reject before force publication. A prepared thread cannot publish
after staged root close. Isolated node/deployment bookkeeping retains failed
stale-root aborts without status access, counter updates or retry. These use real
never-scheduled roots/threads; existing scheduled cross-LP/security/shutdown
fixtures cover integration. See the
[domain abort audit](../reports/audits/2026-10-07-security-domain-thread-abort.md).

The EL0 verifier additionally faults one thread with a spinning peer in the same
exact root. A selected fixture probe checks peer abortion, rejects stale caller
generation/wrong-root local requests, and forces eight cooperative scheduler
boundaries before the self-request. The caller must remain live and unrequested;
the final mask must retain its handle/context through root-operation completion.
The verifier then requires normal peer/caller retirement and exact root close
under its existing ten-second deadline. No mask survives the forced yields and
the fault entry's IRQ state is preserved. See the
[self-handoff evidence](../reports/audits/2026-10-09-security-abort-handoff.md).

A second selected EL0 fixture faults two threads in the same exact root. Both
sweeps rendezvous after root admission, request each other, call timer sleep and
yield eight times each while their inline executor owners defer retirement.
Nested admission must reject; each final handoff completes its root operation
before releasing its executor. Normal retirement and root close must finish
under the unchanged ten-second deadline. A scheduled kernel probe also checks
root-admission and force-publication rejection release the executor owner. See
[concurrent executor evidence](../reports/audits/2026-10-09-security-abort-executor.md).
The real device-rollback callbacks bound global lock availability by one second;
a single failed try-lock is contention, not proof of caller ownership. Timeout
remains a failure and IRQ state/hardware completion rules are unchanged.
The asynchronous IPC reply fixture drains exact-root busy-close rejection under
a shared five-second deadline: result observation can precede producer lease
completion. It neither clears counts nor treats reply visibility as root quiescence.

`scripts/run-host-tests.sh` exercises the production socket registry against
fixed smoltcp storage: rejected creation delivery, dead-owner generations,
unactivated expiry, admission reserves and buffer-count recovery. It also
checks complete/partial entropy failure, independent interface/port keys and
stack-arena application mapping exclusions.

The synchronous boot suite runs `self_test::thread_admission::run` on both
compiled architecture paths. It forces a stack collision without overwriting
the existing page, rejects quota before backing allocation, injects initial
allocation rejection, fills all 64 stack reservations, churns slots, verifies
initial-launch rollback and ASID reuse, and sends raw foreign user/kernel
watch syscalls with zero, matching and stale generations. Same-domain and
trusted internal watches remain covered.

`scripts/run-aarch64.sh --security-test --instance NAME --fresh-storage
--timeout 130` additionally executes EL0 spawn-quota rejection, foreign watch
rejection and denial of TCP/IP's privileged owner-status syscall. Its scoped
probe reports `checks=0xffff`. These checks supplement the 19 deferred tests.
The final remediation evidence is linked from
[the remediation report](../reports/audits/2026-10-05-security-remediation.md).

## User entry, x86 faults and TCP/IP pressure

The normal EL0 verifier additionally runs `self_test::user_isolation::verify`.
Handwritten architecture stubs snapshot initial GPR and FP/SIMD state before
any runtime code. x86 checks default controls, zero payload and FS/GS bases,
then preserves nonzero x87/XMM/TLS state through a timer wait and another
fresh domain. Separate domains exercise invalid opcode, divide-by-zero, user
CLI, unmapped data, NX fetch and rejected stack growth. Each domain must retire
while the verifier remains alive; fixture backing uses the normal image owners.

Run the x86 guest with `scripts/run-x86_64.sh --no-network --instance NAME
--fresh-storage --timeout 160`. The AArch64 security guest also runs the first
entry snapshot and two scoped TCP/IP CALL pressure clients, verifies at least
one second of real protocol-clock progress and advancing reactor cycles, then
retires the pressure clients before the existing scoped authorization checks.
Host adapter tests cover saturated count/byte limits, sustained refill,
drain/reuse and delayed/frequent clock samples. A deliberate physical packet
flood and NIC fault recovery are separate validation work.

## Owned endpoint close

The synchronous IPC cancellation fixtures now close endpoints containing mapped
loans from two caller roots. They check staged server close, competing close and
caller cancellation, preparation rollback, readiness restoration, queued
copy/move/connection cleanup, partial physical failure and abandoned ownership.
Endpoint close watches must remain pending through loan cleanup. Failure probes
retain their exact roots and backing without a reclamation bypass.

The normal EL0 verifier also runs `ipc::cancellation::tests::run_endpoint_runtime`
after secondary LPs are online. It closes an endpoint with mapped loans from
two callers and checks terminal results and restored loan authority. x86 sends
actual synchronous shootdown IPIs during this probe. Borrower roots have no
application threads; concurrent hardware walks and failed-recipient/device
recovery remain separate validation work. Use the isolated x86 and AArch64 guest
commands above; evidence is in the
[endpoint close report](../reports/audits/2026-10-06-security-endpoint-close.md).

## Owned device retirement and DMA loans

Device boot fixtures exercise scratch/direct MMIO cleanup, fake backend DMA
completion, exact leaf/generation checks, preparation rejection, partial
physical failure and abandonment. Six additional closing roots remain retained,
including five heap-page charges. A deferred fixture closes mapped MMIO after
secondary LPs start; x86 executes real cross-LP invalidation. Roots have no
application threads and device registers are never accessed.

Loan fixtures hold real nonexclusive DMA pins for read/write loans, verify
revocation rejection preserves mappings and lender restrictions, then explicitly
unpin and complete cleanup. Queued/delivered IPC cancellation and reply must
publish no result before unpin. These are ownership/pin tests, not physical DMA
timeout or failed-IPI injection. Use the isolated guest commands above; exact
commands, results and limitations are in the
[device retirement report](../reports/audits/2026-10-06-security-device-retirement.md).

## Owned namespace memory cleanup

New boot fixtures retain local and mapped peer roots through bounded detach and
post-guard invalidation, including already-closing peers. Preparation rejection
checks an exact stale generation without walking a successor; admitted leases
roll back while mappings remain intact. Partial detach, failed barriers/scratch
and abandoned receipts retain backing, scratch and all affected roots. An unmapped
reader must stay Pending while another revocation owns its prior borrower list.

Existing failed-loan fixtures now retain their roots as well as data pages. This
batch retains 18 additional roots, nine new object pages and four heap pages;
there is no test-only recovery path. A deferred fixture exercises production
local/peer invalidation after secondary LPs start. Roots have no application
threads; hardware failure recovery and concurrent hardware-walk stress remain
separate work. Commands and evidence are in the
[memory retirement report](../reports/audits/2026-10-06-security-memory-retirement.md).


## IPC owned-memory delivery

`ipc::delivery_tests` runs before secondary LP startup using raw kernel ABI
fixtures. It captures real queued/unobserved IDs and verifies that information,
byte access, mapping, explicit close, transfer/loan preparation and DMA pinning
reject before delivery while unified authority remains live. Scalar/vector
moves and copies cover calls/sends, cancellation and successful handoff. Failed
receive result-page writing retains hidden queue ownership. Endpoint and
whole-server cleanup reclaim hidden authority; scalar cleanup checks original
sponsorship refunds. Returned memory stays hidden through readiness waiting,
then polling transfers ownership or unobserved call close reclaims it.

Existing returned-memory/loan tests check the same visibility after split-phase
reply completion. These fixtures add no deliberate quarantined roots or pages.
The x86/AArch64 isolated commands, results and scope are in the
[delivery report](../reports/audits/2026-10-06-security-memory-delivery.md).


## IPC connection delivery

The connection fixtures in `ipc::delivery_tests` capture live hidden queue/result
identities and check rejected send/call, mint/delegation, close watches, management
target resolution and explicit close, with unchanged capability/record charges.
Connection-only and combined copied-memory calls cover receive failure, delivery,
cancellation and endpoint close. Delivered grants can mint attenuated children
that survive call close; hidden ones cannot. Closing either root reclaims queued
grants and original sponsorship. Returned grants stay hidden through readiness
waiting; first poll publishes them, and repeat polling after grant close does
not republish missing authority. Unobserved call/root cleanup consumes hidden
results. Existing split-phase reply tests still exercise source-close claims,
loan cleanup and preparation/publication failures.

These fixtures add no intentional quarantined roots/pages. Exact guest commands,
results and limits are in the
[connection delivery report](../reports/audits/2026-10-06-security-connection-delivery.md).


## Returned-memory source qualification

Returned-memory fixtures verify queued/unobserved owning sources reject through
`PreparedTransfer` with no destination-charge or input-loan mutation, then become
eligible after receive/poll. A visible borrowed source rejects copy/returned-move
ownership while two input loans remain mapped and the pending result stays empty;
a normal reply subsequently revokes both. Cancellation fixtures also verify copy
and move rejection for both queued and delivered read/write loans before mapping
and exercising their existing cleanup interleavings. No additional fault
quarantine is introduced. Results and remaining scope are in the
[source qualification follow-up](../reports/audits/2026-10-06-security-source-qualification.md).

### Staged rollback and QEMU recovery

The staged-copy fixtures prepare under both exact roots outside IPC, then reject
partial vector preparation, retirement storage, publication and a closed
destination. A 35-page rollback crosses allocator batches; callbacks verify
that IPC/lifecycle/table/object/allocator guards are available and root close
still rejects until staging ends. Existing success, copy isolation, ownership
transfer, loans and returned-capability cleanup remain enabled.

Five host epoch tests cover stale/duplicate acknowledgement, non-regression,
exclusive publication and epoch exhaustion. Three IOMMU command tests cover
required VT-d drains, AMD full-address/exact-epoch completion and SMMU ring
phase/fullness. A deferred four-LP x86 fixture omits an actual IPI, separately
times out with one missing acknowledgement, retains the exact root/charge/slot,
and then completes a fresh real rendezvous. It never manufactures a hardware
acknowledgement or re-adopts a quarantined owner.

A pre-driver fixture on each supported QEMU NVMe target enables a controller
with DMA admin queues, rejects one teardown completion after detachment, checks
pin retention and retiring-map rejection, then retries real invalidation.
Reassignment rejects while old MMIO authority survives; after cleanup, a real
controller reset must precede a new domain and leave `EN/RDY=0`. Subsequent
operational NVMe/object-store/persistent-Raft tests verify storage still works.
The fixture submits no I/O command and does not suppress a physical IOMMU ACK.

Run Intel VT-d and AMD-Vi with `scripts/run-x86_64.sh --no-network`, adding
`--iommu amd` for AMD. Run SMMUv3 with `scripts/run-aarch64.sh --security-test`.
Use separate `--instance` names, `--fresh-storage` and isolated Arm forwarding
ports. Final validation passed **15/15** on each x86 backend and **19/19** on
Arm. Exact commands and limitations are in the
[staging/quiescence audit record](../reports/audits/2026-10-06-security-staged-quiescence.md).

### Private translation admission

Both architecture boot fixtures count private roots/intermediates against their
owning account. They exercise sparse partial construction, rejection/retry,
cached reuse at the ceiling and independent aliases. A public memory-object
fixture repeatedly rejects sparse mapping, verifies unchanged source backing
and usable authority, then remaps cached tables and closes normally. An ordinary
table-admission pressure adapter preserves actual counters and physical memory
while testing platform root/mapping progress and ordinary recovery.

Root-release failure fixtures now check whole-account table retention. Two new
rejected/abandoned provisional cases retain two physical frames and charges;
they simulate owner interruption, not real panic unwinding. Prefix/success
fixtures otherwise refund all table admission at confirmed physical teardown.
Run the same three QEMU targets above with fresh instance names. Exact commands,
results and limits are in the
[table admission audit record](../reports/audits/2026-10-06-security-table-admission.md).

### Shared kernel translation admission

The same architecture boot fixtures exercise real higher-half mappings with
shared node admission. They check rejection before physical allocation, unused/
zeroed-owner refunds, retained sparse construction prefixes, repeated rejection,
sixteen cached reuse rounds at the ceiling, and mapping retry after pressure
ends. Four user-root creation/destruction rounds borrow the same alias without
duplicating or refunding shared charges. Invalidation precedes return of the
foreign data frame and runs outside `KERNEL_AS`.

Linked empty fixture tables remain kernel-owned and charged. Two additional
rejected/abandoned preparations intentionally retain two frames and charges;
the pressure adapter never clears real charges or recovers backing. These are
single-mutator counter-pressure fixtures, not real physical OOM or concurrent
hardware-walk proofs. Commands and results are in the
[shared-table audit record](../reports/audits/2026-10-06-security-kernel-table-admission.md).

### Stack backing admission

The synchronous thread fixtures now test the combined maximum user/kernel
reservation and its exact refund after both physical ranges complete. A real
demand-growth fixture preserves a foreign colliding leaf, records partial
growth, releases rejected preparation and retries successfully. Counter pressure
rejects ordinary preparation before its allocator callback while allowing real
trusted-platform and kernel-only stack pairs, then ordinary recovery.

Five failed/abandoned cases retain five roots/slots and 85 admission pages.
Three data frames and one real sixteen-page kernel stack stay unavailable,
alongside the private root hierarchies. Rejected cleanup, incomplete preparation
and interrupted growth use injected owner states. These are serialized boot
fixtures, not physical node exhaustion or actual panic unwinding. Kernel-range
rollback fixtures also verify that rejected unpublished backing stays in the
post-guard retirement receipt without consuming foreign leaves. See the
[stack audit record](../reports/audits/2026-10-07-security-stack-admission.md).

The real DMA fixture also rejects each newly allocated private domain prefix
(VT-d root/MSI walk, AMD root, Arm root/CD/MSI walk) and complete preparation,
before hardware publication. It checks backend/lifecycle/device/table/physical/
heap/config availability and disabled bus mastering before physical release and
metadata disposal, then requires exact charge/authority refund and root close.
Complete-grant Drop under held guards and injected second-frame release failure
retain both exact roots/reservations; repeat release adapters must never run.
See [private rollback evidence](../reports/audits/2026-10-09-security-dma-private-rollback.md).

### IOMMU boot initialization ownership

Before ordinary installation, the selected backend rejects each real private
allocation-region prefix and complete preparation, requiring exact table-charge
refund and slot-claim release. Actual successful control waits and publication
check backend/lifecycle/device/CPU-table/heap/physical availability with preserved
entry IRQ policy. Contiguous device/stream tables are one region; these probes
do not inject a failure at every page. Repeat installed initialization must not
run preparation. Synthetic empty/private abandonment under all guards, second-
frame rejection and uncertain control/published failures retain the complete
claim/payload: five original unit charges/four frames, no new domain charge.
These run before AP schedulers leave their boot barrier, and do not simulate
real hardware timeouts or qualify outstanding I/O, physical devices or boot MMIO
rollback. See [unit initialization evidence](../reports/audits/2026-10-09-security-iommu-unit-initialization.md).

### PCI reset claim and DMA publication

The real NVMe fixture stages an unstarted endpoint claim and requires rejection
of BAR/ECAM MMIO grants and existing map/unmap/close without consuming authority.
Explicit cancellation then permits normal reset. Actual wait-boundary probes
require config/device availability, preserved IRQ state, disabled bus mastering,
captured BAR/ECAM exclusion and rejected ordinary config/MSI discovery. Final
publication probes require the exact root and newly published DMA capability
to remain busy, with bus mastering disabled, until consuming activation.

RAM-backed fixtures separately check unstarted cancellation, synthetic ready
activation and rejected uncertain activation/cancellation. Guarded complete-grant
abandonment retains two additional exact roots/reservations and endpoint/RAM
metadata without command writes; no new domain-table/data-frame charge is added.
No real timeout, activation readback failure, outstanding I/O or physical reset
is injected. Complete-unit creation now also releases lifecycle/backend holds
and probes table/allocator availability at actual reset waits. See the
[reset-claim evidence](../reports/audits/2026-10-09-security-pci-reset-claim.md).

### DMA creation ownership

Real QEMU probes at complete-unit claim, before construction, before initial
configuration and before exact restoration require backend/lifecycle/device/
CPU-table/physical/heap availability, unchanged IRQ state, exact-root busy close
and nested mutation/reset exclusion. A detached domain's real post-drain fixture
also rejects new creation before reset. Staging root close after real configuration
must return Pending, reject capability publication before activation, confirm
post-guard rollback/refund, then complete that same closing owner.

Synthetic success/error restores the actual engine state. Absent-engine and
empty-domain cells reject before work. Guarded complete-unit/grant abandonment
retains one unit table, one domain table, two separately charged data frames,
metadata, an exact root and the original reservation. These are deterministic
serialized probes, not cross-LP races or actual hardware timeouts. See the
[creation-phase evidence](../reports/audits/2026-10-09-security-dma-creation-phases.md).

### DMA map/unmap maintenance ownership

The real NVMe fixture probes successful map/unmap and rejected sparse-prefix
cleanup before actual backend completion. It requires backend/lifecycle/device/
CPU-table/physical/heap availability with unchanged IRQ state, exact root and
capability busy-close behavior, live pin protection and unit-wide mutation/reset
exclusion. Injected unmap completion rejection leaves the pin and charges live;
repeated unmap/remap cannot release it. Real acknowledged domain retirement
permits memory close and refunds the domain account.

A synthetic complete owner abandons actual backing/pin and heap metadata under
all guards, retaining its public root/capability claim. It adds one original
domain-table charge/frame plus two data frames in their independent memory
account; root backing is separate. No actual timeout, lost acknowledgement,
outstanding I/O, panic unwinding or cross-LP stress is injected. See the
[mapping-maintenance evidence](../reports/audits/2026-10-09-security-dma-mapping-maintenance.md).

### IOMMU table admission and failed-map cleanup

Serialized pre-driver fixtures on Intel VT-d, AMD-Vi and SMMUv3 exercise actual
private table walkers under small domain ceilings, linked sparse prefixes,
repeated rejection, cached reuse and retry. Common owners cover node/subpool
limits, contiguous zeroed allocation, unused refund, published abandonment and
terminal partial physical release. Five charged pages and three actual frames
are deliberately retained; physical rejection and abandonment are injected.

The QEMU NVMe recovery fixture checks charge retention through rejected drain,
real completion/refund and capability refund under domain-pool pressure. A
two-page buffer crosses a cached/fresh leaf-table boundary at the ceiling. Real
rollback maintenance releases its pin; rejected completion retains it until
real domain retirement. Duplicate mapping and premature memory close reject.
Operational storage tests run afterward. The fixture submits no I/O command,
suppresses no physical ACK and does not prove physical-platform recovery.

Use the existing Intel/AMD/Arm commands above with fresh, isolated instances.
Contracts and evidence: [IOMMU admission](../reference/iommu-table-admission.md)
and [audit record](../reports/audits/2026-10-07-security-iommu-admission.md).

### Terminal kernel-range physical release

Kernel-range boot fixtures exercise real detach/invalidation followed by physical
batches of 16/16/3 for 35 standard pages and thirty-two batches for a 2 MiB leaf.
Between-batch hooks assert the physical and kernel-table guards are available.
Rejecting the final base frame after 511 successful releases freezes that receipt;
a successor claims a freed address and rejected retry/reinitialization must leave
it untouched. A simulated interruption after real invalidation also forbids retry.
Two additional frames remain quarantined alongside the existing one-frame Drop
fixture. These are serialized injections, not actual allocator corruption, panic
unwinding, hardware failure or a worst-case latency proof. Existing failed-barrier
retry and ordinary stack cleanup remain enabled. See the
[kernel release audit record](../reports/audits/2026-10-07-security-kernel-release.md).

Provisional kernel-frame fixtures also abandon one real unpublished owner
under the physical allocator and one published leaf owner under both allocator
and kernel-table guards. Drop must return without freeing data or acquiring
those guards; the live leaf retains its exact physical identity. The fixture
detaches and invalidates that leaf afterward but keeps both abandoned frames
unavailable permanently. These add two 4 KiB pages to the previous three
kernel-range quarantine pages. Ordinary foreign-leaf/successor cleanup now uses
fresh explicit receipts. This is controlled abandonment, not panic unwinding
or an EL0-triggered fault. See
[kernel preparation audit](../reports/audits/2026-10-07-security-kernel-preparation.md).
