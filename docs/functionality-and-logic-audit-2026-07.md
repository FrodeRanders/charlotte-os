# CharlotteOS functionality and logic audit (2026-07-19)

## Executive summary

This audit reviewed commit `57db5ac` (`relmsg: implement Reliable Message Layer
service`) against the current implementation and the architecture described in
`charlotte-sitas-xous-architecture.md` and
`charlotte-networking-architecture.md`.

The kernel's AArch64 foundation is substantially functional under QEMU TCG on
Apple silicon. A fresh four-LP build booted through Limine, initialized all four
LPs, passed the synchronous physical-memory, virtual-memory, allocator,
capability, IPC, CQ, and device-object checks, entered EL0, completed an EL0 IPC
round trip, initialized the scheduler and GIC, parsed ACPI, and began PCI ECAM
probing. Both AArch64 and x86_64 kernel configurations also compile.

The newer networking layers are not yet ready to be called functionally
validated. Static review found correctness defects in smoltcp frame-length
handling, reliable-message bounds checking, and vector memory IPC rollback.
The Raft defects found by this audit have since been repaired and its two-node
EL0 path is now exercised successfully under QEMU TCG. The remaining findings
are independent of HVF's EL0-MMIO
limitation and should be fixed before KVM testing. The test harness also reports
"All Tests Passed" before deferred EL0 tests have completed, which makes some
existing boot-validation claims stronger than the evidence supports.

The original audit was read-only. The Raft remediation described below was
implemented and validated in a subsequent pass on 2026-07-19.

## Scope and method

The audit covered:

- the capability, endpoint, memory-object, completion-queue, syscall, service,
  and device abstractions central to the Sitas/Xous architecture;
- the AArch64 and x86_64 kernel build configurations;
- the userspace runtime and service binaries;
- the virtio-net protocol, smoltcp adapter, reliable-message service, and Raft
  implementation emphasized by the networking architecture;
- boot scripts and the distinction between synchronous boot tests and deferred
  scheduler/EL0 tests.

The following checks were performed on an Apple-silicon macOS host using
`rustc 1.99.0-nightly (b6839f4d0 2026-07-17)` and QEMU's AArch64 `virt`
machine:

| Check | Result |
|---|---|
| AArch64 kernel check, `acpi` | Pass, including `RUSTFLAGS=-D warnings` |
| AArch64 kernel check, `acpi,hvf_compat` | Compiles normally; fails with `-D warnings` due to two cfg-dependent unused imports |
| x86_64 kernel check, `acpi` | Pass |
| Host `cargo check --workspace` | Fail: the no-std kernel is built for the Mach-O host and its ELF Limine section names are invalid |
| `cargo fmt --all -- --check` | Fail: widespread formatting drift (over 2,000 diff lines) |
| Host `cargo test` for protocol/runtime/Raft/smoltcp crates | Fail before tests due to duplicate `core` lang items caused by the workspace build-std configuration |
| Strict userspace-services check | Fail first in `catten-graft` on five `unused_unsafe` warnings |
| Fresh AArch64 QEMU TCG boot, four LPs | Boots; synchronous tests pass; deferred execution begins; see qualification below |

The KVM-only virtio-net runtime path could not be executed on this host. The
findings below therefore distinguish directly observed behavior, compile-only
evidence, and unverified platform paths.

### Raft provenance follow-up

The Raft findings were compared with
`../../gautelis/raft/graft-rust` at commit `1a818d4`. All three are artifacts of
the CharlotteOS no-std adaptation, not defects copied from the source project.
The relevant safeguards were already present in upstream's initial Rust commit
`cf9de4a` (2026-06-10), before `catten-graft` was introduced by CharlotteOS
commit `1000cf1` (2026-07-18). Upstream's `gautelis-graft-core` tests passed
(5/5), as did the `graft-tests` integration suite (16/16). Those tests provide a
healthy baseline, although the current suite does not directly exercise every
specific election-response and higher-term snapshot case identified here.

### Raft remediation and end-to-end validation (2026-07-19)

The remediation went beyond F1-F3 and completed the previously skeletal
CharlotteOS integration:

- added bounded, versioned one-page wire encoding for votes, AppendEntries,
  snapshots, and all responses, with malformed/oversized input rejection;
- implemented asynchronous Charlotte IPC request submission, returned-memory
  response polling, completion delivery into `RaftNode`, dynamic peer lookup,
  pending-call deduplication, and stale-term cancellation;
- completed service-side request decoding, response encoding, timer-driven
  elections, inbound-before-timeout ordering, and chunked snapshot progress;
- randomized initial election deadlines from the node ID rather than the ID
  length (equal-length IDs previously received identical initial deadlines);
- fixed the EL0 reply-poll assembly ABI so the x1 input capability cannot be
  confused with the x1 result, and used the completion-capability close syscall
  rather than `ipc_close` for timer capabilities;
- moved the Raft RPC scratch page away from `0x800000`, which is occupied by the
  current AArch64 EL0 layout and caused `MAP_FAILED` for received RPC pages; and
- replaced the spawn-only Raft self-test with a deferred verifier that starts a
  two-voter cluster, waits for one Leader and one Follower, and requires an
  asynchronous RPC completion before reporting success.

Validation evidence:

- `catten-graft`: 7/7 host tests pass (five Raft regressions and two complete
  wire-format tests).
- A clean four-LP AArch64 QEMU TCG boot reports:
  `[raft] SUCCESS: two-node cluster elected one leader (states 3/1, completions 1/1).`
- The Raft service release ELF builds for `aarch64-unknown-none` and is embedded
  in the kernel self-test image.

This proves registration, peer discovery, vote request/response memory IPC,
wire decoding, completion polling, and leader election together. It is not a
substitute for longer fault-injection, partition, restart, persistence, or
multi-node replication/snapshot testing on Linux KVM.

## Findings

### F1 — Critical: Raft counts every configured voter after one response

**Provenance:** CharlotteOS adaptation defect; not present in
`../../gautelis/raft/graft-rust`.

**Resolution (2026-07-19):** fixed in `catten-graft`. Elections now retain a
set of distinct configured voter IDs for the current term, ignore duplicate and
unknown responses, and recognize a single-voter self-majority. Host regression
tests cover all of those cases.

`RaftNode::handle_vote_response` ignores `peer_id` and, on any granted response,
sets `granted` to one for itself and then increments it once for every other
configured voter. It does not record which peers granted votes and does not
deduplicate responses. In a five-node cluster, one granted response therefore
looks like five grants and immediately elects the candidate.

Evidence: `crates/catten-graft/src/node.rs:184-213`.

Impact: two candidates can each become leader without receiving a majority,
violating Raft election safety and enabling split brain.

Recommended correction: track granted voter IDs per election term, count the
self-vote once, ignore duplicate/non-voter/stale responses, clear the set on
term or election changes, and add 1/3/5-node tests covering rejection,
duplication, stale terms, and competing candidates.

The source implementation already provides the appropriate model:
`graft-core/src/raft_node.rs:574-627` builds a `HashSet<String>` from response
peer IDs, includes self exactly once, rejects stale election rounds, and asks
the membership object for a (possibly joint-consensus) majority. The runtime
also accumulates distinct peer IDs before making the same majority check
(`graft-runtime/src/runtime.rs:210-270`). Git blame attributes the core logic to
the original Rust implementation commit `cf9de4a` on 2026-06-10. CharlotteOS's
no-std port was introduced later in commit `1000cf1` on 2026-07-18 and replaced
that aggregation with the erroneous configured-voter loop.

### F2 — High: Raft commit quorum omits the leader and has broken edge cases

**Provenance:** CharlotteOS adaptation defect; not present in
`../../gautelis/raft/graft-rust`.

**Resolution (2026-07-19):** fixed in `catten-graft`. Commit advancement now
counts the leader explicitly, counts only configured voting followers, scans
only current-term entries, and immediately advances single-voter commits. Tests
cover one-node commit and leader-plus-one commit in a three-voter cluster.

`become_leader` initializes `match_index` only for other peers
(`node.rs:226-238`). `submit_command` tries to update a self entry that does not
exist (`node.rs:451-467`). `advance_commit_index` nevertheless indexes the
peer-only values using a quorum calculated from all voters (`node.rs:472-489`).

For a three-voter cluster this commonly requires both followers to acknowledge
instead of the leader plus one follower. A single-node cluster has no
`match_index` entry and cannot commit. `start_election` also does not immediately
promote a single-node candidate based on its own vote.

Impact: loss of Raft liveness and incorrect majority behavior.

Recommended correction: include and maintain the leader's own replicated index,
derive quorum over voter IDs rather than raw map contents, and explicitly cover
single-node and three-node majority cases in unit tests.

The source implementation deliberately keeps `match_index` follower-only but
does not make the CharlotteOS mistake of treating that map as the full voter
set. `advance_commit_index_from_majority` inserts the leader ID into a replicated
ID set, adds only followers whose `match_index` reached the candidate index, and
then applies the membership majority function
(`graft-core/src/raft_node.rs:996-1028`). This logic also dates to upstream's
initial Rust commit `cf9de4a`. It handles a single-node commit because the
self-only set is already a majority. The CharlotteOS adaptation instead sorted
the follower-only map and indexed it using a quorum calculated from all voters.

### F3 — High: snapshot handling does not adopt a higher term or step down

**Provenance:** CharlotteOS adaptation defect; not present in
`../../gautelis/raft/graft-rust`.

**Resolution (2026-07-19):** fixed in `catten-graft`. A higher-term snapshot now
updates and persists the term and clears the vote while stepping down. A valid
same-term snapshot also forces Candidate/Leader to Follower without clearing
the persisted vote for that term. The higher-term path has a regression test.

`handle_install_snapshot` rejects lower terms but, unlike AppendEntries and vote
handling, does not update `current_term`, persist the new term, clear `voted_for`,
or become a follower when `req.term > current_term` (`node.rs:356-408`). A
candidate or leader may therefore install a snapshot from a newer-term leader
while retaining stale term/state.

Impact: Raft term monotonicity and leader-state invariants are violated.

Upstream `accept_snapshot_chunk` explicitly updates and persists a higher term,
calls `become_follower`, records the leader, and refreshes the election timeout
before accepting data (`graft-core/src/raft_node.rs:1346-1368`). Git blame again
shows this behavior in the original upstream Rust implementation `cf9de4a`.
Those guards were omitted when the CharlotteOS `handle_install_snapshot` path
was written in `1000cf1`.

### F4 — High: the smoltcp receive adapter discards the frame length

**Resolution (2026-07-19):** fixed. The adapter now polls each reply once with
the full memory-return ABI, retains and validates the returned byte length
against both MTU and page size, closes the pending-call capability, and exposes
exactly that slice to smoltcp. Invalid lengths and failed mappings close the
returned object. A host test covers zero, MTU-boundary, oversized, and
page-oversized lengths.

`CharlotteEthDevice::receive` first calls `ipc_reply_poll`, discards its result
value, and then calls `ipc_reply_poll_with_memory` for the same pending call
(`crates/charlotte-smoltcp/src/lib.rs:132-150`). Re-polling happens to work
because the kernel retains and copies an observed `ReplyValue`, but it is
redundant and, more importantly, the code discards the reply scalar that carries
the actual frame length. `CharlotteRx::consume` consequently always presents
2,048 bytes to smoltcp (`charlotte-smoltcp/src/lib.rs:177-190`). Padding or stale
page contents become part of the apparent Ethernet frame and the supplied size
can exceed the adapter's advertised MTU.

Impact: receive traffic can be rejected as oversized or parsed with an invalid
length; the adapter's compile success does not establish usable networking.

Recommended correction: call only `ipc_reply_poll_with_memory`, retain its
result length in `CharlotteRx`, validate it against the mapped object and MTU,
and give smoltcp exactly that slice. Add a mock-syscall adapter test for pending,
ready, malformed-length, and back-to-back receives.

### F5 — High: reliable-message wire lengths allow out-of-bounds EL0 access

**Resolution (2026-07-19):** fixed for the current one-page transport. The
protocol now has a checked parser that validates EtherType, reserved bytes,
known flags, control-message payload rules, and the 1,468-byte protocol maximum.
The service checks every mapping, bounds every copy, and closes all objects on
failure. Protocol tests exercise malformed headers and oversized lengths.

The reliable-message service parses an untrusted 16-bit `payload_len` and then
copies that many bytes from offset 16 of a one-page mapping into another
one-page mapping (`crates/catten-services/src/bin/relmsg.rs:121-190`). It does not
validate `payload_len <= 4096 - 16`, does not know the actual inbound object
length, and ignores mapping failures. A payload length up to 65,535 can walk
well beyond both mappings. The same unbounded length is later used by
`try_deliver` when copying out of the queued one-page object.

Impact: a client can crash the userspace service with a data abort and may read
or overwrite adjacent mappings if present. This is especially important for a
capability system because a protocol peer should not gain authority over
unrelated pages through unchecked lengths.

Recommended correction: carry an authenticated byte length alongside every
memory message, require `HEADER_SIZE + payload_len` to fit both the received
length and object size, check every map result, cap lengths to the protocol MTU,
and reject malformed EtherType/reserved/flag combinations before copying.

### F6 — High: vector IPC transfer is not transactional

**Resolution (2026-07-19):** fixed. Vector metadata and duplicate entries are
validated before mutation. Applied copies, moves, and lends are recorded and
rolled back in reverse order on transfer or enqueue failure; moved objects are
restored under the sender's original capability number. Unaligned vector-page
entries are now accessed with unaligned operations rather than volatile typed
access. A boot self-test forces the second entry to fail after a move and proves
that the original cap remains valid and no message is enqueued.

The architecture calls for explicit, safe ownership transfer. `read_vector_page`
performs copy, move, and lend operations sequentially
(`crates/catten/src/ipc/mod.rs:1590-1630`). If entry N fails, entries 0..N-1
remain transferred or lent; neither `vector_send` nor `vector_call` rolls them
back (`ipc/mod.rs:1520-1587`). An enqueue failure after successful transfers has
the same problem. For move entries, the syscall can return an error after the
sender has already lost ownership.

Impact: partial side effects contradict the apparent all-or-error operation,
leak caps into the receiver, and can strand moved or borrowed memory without a
message/reply lifecycle to return it.

Recommended correction: validate the entire vector first, reserve all required
cap slots and queue space, apply transfers under an explicit transaction with a
reverse-order rollback path, and add negative tests where the second or last
entry fails for every transfer-mode combination.

### F7 — Medium: boot self-test success is printed before deferred tests finish

**Resolution (2026-07-19):** the premature global success claim was removed.
Boot now explicitly reports that synchronous tests passed while deferred
verifiers remain pending. Bounded QEMU runs and CI require the individual
terminal success markers for every supported deferred test and return failure
when any marker is absent. The virtio-net marker is intentionally excluded from
TCG/HVF aggregation pending the separate Linux KVM milestone.

Several functions called by `run_self_tests` spawn verifier threads and return
immediately; the boot log itself says "verifier deferred." Nevertheless,
`run_self_tests` prints `Testing Complete. All Tests Passed!` immediately after
spawning the UART test (`crates/catten/src/self_test/mod.rs:59-80`). In the fresh
TCG boot, this success line appeared before scheduler start, PCI probing, the
EL0 IPC success message, and the net/UART verifiers.

Impact: logs and architecture status documents can misclassify "scheduled to be
tested" as "passed." A KVM run that only searches for the global success string
does not prove the network driver or deferred restart path passed.

Recommended correction: give every deferred test a terminal result, aggregate
those results after the scheduler runs, print one final summary only after all
expected verifiers reach terminal states, and make QEMU exit with an
automatable pass/fail code.

### F8 — Medium: the documented AArch64 timeout/SMP runner arguments are broken

**Resolution (2026-07-19):** fixed. The runner now parses value-taking options
in one pass, rejects missing values, and captures a single serial stream for
bounded runs.

The first argument loop accepts the `--smp` and `--timeout` option names but
rejects their following numeric values before the second loop can parse them
(`scripts/run-aarch64.sh:31-50`). For example:

```text
./scripts/run-aarch64.sh debug --smp 4 --timeout 45
Unknown argument: 4
```

This also breaks `boot-smp1.sh` and `boot-smp2.sh`, which always pass numeric
values in this form.

Impact: repeatable bounded boot tests do not run as documented; developers are
encouraged toward manual interactive boots with incomplete log capture.

### F9 — Medium: test/build automation does not cover the workspace it describes

**Resolution (2026-07-19):** improved. CI now has an explicit host-library job
for the reliable-message protocol, Raft core/wire implementation, and smoltcp
adapter, executed outside the repository directory so the bare-metal
`build-std` configuration is not applied to host tests. Kernel target builds
remain a separate matrix. Full repository-wide formatting cleanup remains a
separate mechanical change.

The root host workspace check attempts to compile the bare-metal kernel for the
Mach-O host and fails on ELF Limine section attributes. The workspace's only
Rust `#[test]` is the message-header round trip; the Raft and smoltcp crates have
no unit tests. A multi-package host test currently fails earlier with duplicate
`core` lang items from the build-std setup. Strict userspace checking fails on
warnings, and `cargo fmt --check` is far from clean.

Impact: CI-style green kernel checks leave the newest, highest-risk logic almost
entirely untested, while obvious host-test commands are not usable.

Recommended correction: separate host-testable libraries from the bare-metal
target in Cargo aliases/CI, scope build-std to custom targets, add a target
matrix for kernel and all service binaries, enforce formatting, and add pure
host tests for Raft/protocol/state-machine logic.

### F10 — Medium: zero-capacity completion rings panic or underflow

**Resolution (2026-07-19):** fixed. Both CQ constructors now return `Result`
and reject capacities below two. The CQ boot test covers zero and one entry.

`CompletionQueueRing::new_page` and `init_at_phys` accept zero entries and store
capacity zero (`crates/catten/src/completion/cq.rs:48-65`). `is_full`, `write`,
`write_batch`, and `read` then use modulo by capacity; `write_batch` additionally
computes `capacity - 1` (`cq.rs:67-131`).

Impact: any future or malformed configuration that creates a zero-entry CQ can
panic in the kernel or underflow. Current fixed-size callers may avoid the path,
but the type's public API does not enforce the invariant.

Recommended correction: return `Result` and reject capacities below two (a
one-empty-slot ring also cannot store data with capacity one), or clamp with an
explicit documented minimum.

### F11 — Documentation status is internally inconsistent

**Resolution (2026-07-19):** corrected in the README, AArch64 status document,
and main architecture status table. EL0/TCG evidence is stated explicitly and
the virtio-net path is consistently described as pending Linux KVM validation.

The main architecture document says the system is substantially implemented and
boot-validated, including EL0 phases. `README.md:35-41` and
`aarch64-port-status.md:173-175` still say EL0/userspace is unimplemented or
untested, which the fresh boot disproved. Conversely, the architecture table
says the network stack "validates on KVM," while its status and the audit context
indicate that KVM runtime validation is still the outstanding task.

Impact: contributors cannot tell which claims are current, and stale caveats
coexist with overconfident validation language.

Recommended correction: make one status document authoritative, attach each
claim to a dated command/log/commit, and use the categories **compiled**,
**synchronous boot-tested**, **deferred test completed**, **HVF-tested**,
**TCG-tested**, **KVM-tested**, and **hardware-tested**.

### F12 — High: scheduler identities and lock ordering were not lifecycle-safe

**Resolution (2026-07-19):** scheduler wake objects and round-robin
queue entries now carry a monotonically increasing thread generation as well
as the reusable numeric thread ID. Stale wakes are ignored and stale queue
entries cannot dispatch a later occupant of the same ID-table slot. The
`block_thread` path was also changed from thread-table → LP-queue locking to the
LP-queue → thread-table order already used by dispatch, wake, and abort.
Blocking and wake submission now use shared access to the immutable system
scheduler topology instead of unnecessarily taking its global write lock, and
round-robin idle state is updated whenever dispatch selects the idle thread so
remote submission sends the required wakeup IPI.

On AArch64, that wakeup previously attempted to send the generic-timer PPI
(INTID 27) through the GIC SGI interface. The GIC correctly rejects every IPI
vector above 15, but the scheduler discarded the error, leaving an idle LP
asleep until an incidental interrupt arrived. Scheduler wakeups now use a
dedicated SGI (INTID 2); its handler marks a local context switch pending, and
send failures are no longer ignored. Round-robin also starts in the idle state,
because no initial quantum is armed before its first dispatch.

Previously, a delayed observer notification named only a `ThreadId`; after
exit and ID-table reuse it could wake an unrelated replacement thread. In
addition, a queued-thread block could hold `MASTER_THREAD_TABLE` while waiting
for an LP scheduler whose dispatch path held the LP lock while waiting for
`MASTER_THREAD_TABLE`, forming an AB/BA deadlock.

The live-upgrade stall exposed the same ABA defect in `ServiceDomain`: teardown
identified the old thread only by a recycled ID and rejected teardown after the
replacement reused that slot. Service handles now retain the thread generation,
and `domain_exited` waits for the old thread to leave both the master table and
the per-LP deferred-reap list before replacement stack allocation. A four-vCPU
AArch64 TCG boot then completed generation-3 state handoff and printed the
`[service] SUCCESS:` marker. The aggregate SMP boot remains timing-sensitive in
other deferred tests (most recently UART), so this resolves the identified
scheduler/service lifecycle errors but is not evidence that every scheduling
path is exhaustively validated.

### F13 — Observation: kernel and userspace UART output interleave by byte

The AArch64 serial log can contain isolated characters mixed into kernel log
lines, for example `U`, `A`, `R`, `T`, `-`, `O`, `K`, a newline, and later `2`.
Together these reconstruct the expected userspace UART-test output
`UART-OK\n2`: `cclient` sends `UART-OK\n` as individual synchronous
`OP_WRITE` calls, and the restart verifier sends `2` through the replacement
driver.

This is not evidence of partial thread execution or memory corruption. The EL0
UART driver writes each byte directly to the PL011 data register, while kernel
logging writes to the same physical PL011. The kernel serial mutex orders
kernel writers only; it cannot serialize a userspace domain holding direct MMIO
authority. Scheduler activity and kernel messages can therefore occur between
any two userspace UART bytes.

The result is useful positive evidence that both the original and restarted
userspace drivers reached the physical device, but it also exposes ambiguous
device ownership. A production design should establish one normal writer:
route post-bootstrap kernel logging through the userspace driver, reserve a
separate debug UART for the kernel, or retain the UART in the kernel and expose
a mediated console operation. Early-boot and panic output may remain an
explicit emergency bypass whose possible interleaving is documented.

## Architecture conformance assessment

The kernel's implemented shape follows the main architecture well: capabilities
represent authority, endpoints carry cross-domain requests, memory objects make
transfer modes explicit, and completion queues aggregate asynchronous kernel and
device events. The boot evidence supports real address-space isolation, EL0
execution, scalar IPC, and a substantial set of negative capability tests.

The main conformance gaps are currently at lifecycle boundaries:

- vector transfer needs atomic ownership semantics (F6);
- deferred test completion needs a durable observable terminal state (F7);
- network buffers need authoritative byte lengths and validation (F4/F5);
- distributed-service authority cannot be trusted until the consensus layer
  implements actual quorum rules (F1-F3).

The long-term networking document remains a proposal rather than a description
of current behavior. The code presently provides a frame protocol, compile-only
smoltcp integration, a local-loopback reliable-message prototype, and an early
Raft implementation. It does not yet make remote IPC indistinguishable from
local IPC, implement transparent distributed capabilities, or demonstrate
reliable transport over the NIC.

## What was directly verified in the fresh TCG boot

Observed before the audit terminated QEMU:

- Limine/UEFI handoff and AArch64 kernel entry;
- four LPs discovered and initialized;
- physical and virtual memory tests passed;
- kernel allocator test passed;
- synchronous memory-object, capability, IPC, syscall-dispatch, CQ, and device
  setup tests reached their pass/deferred messages;
- EL0 service domains were spawned;
- scheduler and per-LP interrupt-controller initialization completed;
- an EL0 endpoint call/reply round trip reported success;
- ACPI XSDT/MCFG parsing and PCI ECAM mapping began;
- the network, UART, service-manager, cross-LP, CQ-wait, and device interrupt
  verifiers were scheduled.

Not established by that run:

- terminal success of every deferred verifier;
- virtio-net initialization or frame TX/RX;
- smoltcp TCP/IP behavior;
- reliable-message behavior over Ethernet;
- Raft election/replication safety;
- KVM, x86_64 ring-3, real hardware, IOMMU/SMMU, or stress behavior.

## Recommended validation order

1. Fix F1-F6 before treating KVM time as a validation run; otherwise KVM will
   only reveal defects already determinable from source.
2. Repair the bounded runner and deferred result aggregation (F7/F8), then save
   complete serial logs as CI artifacts.
3. Add host-side deterministic Raft and protocol tests, including randomized
   message ordering, duplicates, stale terms, malformed lengths, and partial
   vector failures.
4. Run AArch64 TCG and HVF at 1, 2, and 4 LPs, with repeated boots and forced
   service crashes/restarts.
5. On Linux KVM, validate virtio feature negotiation, queue memory ordering,
   interrupt routing, RX/TX lengths, buffer ownership return, CQ overflow, and
   driver death during in-flight DMA.
6. Only after those pass, exercise smoltcp and reliable-message traffic under
   loss, duplication, reordering, saturation, and service restart.
7. Treat real-hardware and IOMMU/SMMU validation as separate milestones rather
   than consequences of QEMU success.
