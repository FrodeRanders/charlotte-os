# CharlotteOS, Sitas, and Xous: A Co-Designed Architecture for Isolated Asynchronous Services

> **Status:** Architecture and implementation redirection note\
> **Audience:** CharlotteOS and sitas contributors; intended as direct
> input to Codex\
> **Purpose:** Replace the earlier *Sitas on CharlotteOS* working draft
> with a unified model that combines:
>
> -   **CharlotteOS** as the protection, scheduling, interrupt, memory,
>     and completion substrate;
> -   **sitas** as the shard-per-core userspace execution and ownership
>     model;
> -   **Xous** as the reference model for isolated userspace servers,
>     connection-oriented IPC, and MMU-enforced memory-message
>     semantics.
>
> This document distinguishes architectural commitments from experiments
> already implemented in the fork. It also identifies where the current
> direction should be preserved, corrected, or replaced.

------------------------------------------------------------------------

## 0. Current in-tree implementation status

CharlotteOS now has the first kernel-side slice of this architecture:

-   `crates/catten/src/ipc/mod.rs` implements a scalar-only endpoint IPC
    registry with per-address-space capability tables, endpoint caps,
    connection caps, pending-call caps, and one-shot reply-token caps.
-   Syscalls `18..=27` expose endpoint creation, same-address-space
    connection minting, scalar send, scalar call, nonblocking receive,
    reply, reply polling, reply-time connection delegation, IPC cap
    close, and blocking receive.
-   `crates/catten/src/memory/object.rs` implements first-class
    page-backed memory objects with per-address-space memory
    capabilities, map/unmap/close, move, read-lend, write-lend, and
    address-space teardown.
-   Syscalls `28..=36` expose memory-object allocation, mapping,
    unmapping, close, moved-memory send/call/reply, and reply-bound
    read/write borrows. `IPC_RECV` returns an attached memory cap in
    `x7`; `IPC_REPLY_POLL` returns a reply-time memory cap in `x3`.
-   Syscall `39` (`IPC_SCALAR_CALL_CONNECTION`) implements call-time
    connection delegation: the caller attaches an attenuated connection
    minted from its own endpoint cap (or from a re-delegable connection
    cap bearing `MINT_CONNECTION`); the receiver observes the minted
    connection cap in `x8` of `IPC_RECV`/`IPC_RECV_BLOCK`.
    `IPC_REPLY_CONNECTION` now accepts re-delegable connection caps as
    its minting source and carries an explicit reply result, so a
    service can return connections to endpoints it does not own.
    Cancellation and endpoint teardown reclaim queued attached
    connections, and pending calls track a `ResultObserved` state so
    closing an observed pending-call cap no longer revokes returned
    capabilities.
-   Syscall `40` (`IPC_SCALAR_CALL_CONNECTION_COPY`) delivers the first
    combined attachment: one call carries both a copied memory object
    (`x7` at receive) and a minted connection (`x8`). This is the step
    toward the §6.5 envelope's independent `segment_count` and
    `capability_count`, and it is what lets a service register under a
    memory-carried name and hand over its endpoint authority in a
    single message.
-   `crates/catten/src/service/` contains the first Phase 3 supervisor
    slice: a generalized EL0 ELF loader (`loader.rs`), the config-page
    bootstrap-capability contract (`bootstrap.rs`, mirrored by
    `catten_rt::config::bootstrap_cap`), and domain spawn /
    exit-observation / teardown supervision (`supervisor.rs`). The
    supervisor creates the name-service registry endpoint inside the
    new domain before it runs and delegates bootstrap connections
    strictly downward; no userspace code ever names an ASID or LP.
-   `crates/catten-services` provides the reference EL0 service
    programs built as real ET_EXEC ELFs: a userspace name service
    (names → re-delegable connection plus instance generation;
    re-registration bumps the generation and closes the previous
    instance's connection; LOOKUP replies with attenuated `SEND|CALL`
    connections), an echo service that creates its own endpoint and
    registers itself, and a client that bootstraps, looks up, and calls
    purely through delegated capabilities. Names travel either as an
    interim 8-byte packed scalar or, for longer names, in a copied
    memory object addressed to a unified byte-keyed registry; the
    reference echo service registers under both a short scalar name and
    a 30-byte memory-carried name, and the reference client resolves
    the service through the long name.
-   `crates/catten-syscall` has EL0 wrappers for endpoint IPC,
    memory-object operations, and memory IPC transfer operations.
-   Boot-time self-tests cover cross-address-space delegation through
    the direct kernel API, cross-address-space EL0 client/server calls,
    reply-time connection delegation without userspace ASID parameters,
    queue backpressure, invalid-cap/type failures, scalar call/reply,
    reply-token single use, teardown, blocking endpoint receive, and
    same-address-space syscall dispatch. They also cover kernel-internal
    copied, moved, and lent memory IPC plus real EL0 two-domain memory
    IPC smoke tests for copy, move, read-borrow, write-borrow, queued
    moved/borrowed-memory cancellation, and delivered borrowed-memory
    cancellation. They also cover server teardown with queued or
    delivered memory-bearing calls. Phase 3 adds kernel-level
    connection-attachment tests (non-mintable attachment denied,
    wrong-type attachment denied, rights attenuation on re-delegation,
    cancellation of queued connection-bearing calls, endpoint close
    completing queued register calls, stale connections failing
    `EndpointClosed`) and an end-to-end EL0 test (`el0_service.rs`)
    that spawns three isolated EL0 domains from the reference ELFs and
    verifies bootstrap delivery, registration, lookup, call, voluntary
    shutdown, domain teardown, stale-connection failure, restart, and
    generation-2 re-lookup against the live EL0 name service.

This is intentionally not the full Xous-style model yet. The first
version does not include a userspace name service, arbitrary
target-domain delegation syscalls, blocking call scheduling beyond
blocking receive, general capability attachment transfer beyond
reply-time connection delegation and memory-object transfer,
userspace-facing cancellation policy, or production resource accounting.
Blocking endpoint receive exists as a first readiness integration point,
and the smoke-test two-domain EL0 client/server flows now work through
kernel-delegated connection capabilities.
Phase 3 has since filled the service-management gap at smoke-test
fidelity: a userspace (EL0) name service now exists, bootstrap
capability delivery follows a documented config-page contract, service
instances carry generations that increment on restart, and stale
connections fail deterministically with `EndpointClosed`. Long service
names are supported through copied memory objects (combined
connection + memory attachments on one registration call), with the
8-byte packed scalar form retained as a fast path. Still
unimplemented: policy gating on lookup beyond connection delegation,
automated restart policy in the supervisor, a general process loader
contract (stack and guard pages, argument/environment blocks, declared
heaps), and production resource accounting. The reference service ELFs
are prebuilt and embedded in the kernel image rather than loaded from
storage.
Completion queues remain separate and should continue to be used for
kernel/device operation completion rather than as universal IPC
endpoints.

Current evidence:

-   `c86a8a9` added a real EL0 scalar endpoint IPC smoke test covering
    endpoint creation, same-AS connection minting, send, call, receive,
    reply, and reply polling.
-   `f635a9c` added `IPC_RECV_BLOCK` (`svc #27`) using endpoint
    readiness observers and a lost-wake guard.
-   `089d4a7` added a two-thread EL0 test proving that a server can
    block in endpoint receive and be woken by a later client call.
-   `a97d9e4` added a two-address-space EL0 test proving that a client
    protection domain can call a server endpoint through a delegated
    connection capability without either userspace side knowing or
    passing the other's ASID or LP.
-   `b13fb76` exposed first-class memory objects and memory IPC
    move/borrow operations through the syscall ABI.
-   `9af956f` added a real EL0 two-address-space memory IPC smoke test:
    the client allocates, maps, writes, and moves a memory object to a
    server; the sender's moved-from cap no longer maps; the server maps
    and updates the object; and `IPC_REPLY_MOVE` returns the object to
    the caller.
-   The current tree extends that EL0 memory IPC smoke test to cover
    reply-bound `BorrowRead` and `BorrowWrite`: read-borrow denies a
    writable receiver mapping, normal reply revokes the receiver's
    borrowed cap, and write-borrowed mutations become visible to the
    owner after reply.
-   The current tree also adds EL0 cancellation smoke coverage:
    the caller queues moved-memory and write-borrow calls, closes both
    pending-call caps before the server receives, observes that the
    moved-from cap remains consumed and the write borrow is revoked back
    to the owner, and the server observes `NoMessage` for both cancelled
    requests. It also covers a delivered write-borrow cancellation: once
    the server has received, mapped, and updated the borrowed memory, the
    caller closes the pending-call cap; the owner regains mapping
    authority while the server's borrowed memory cap and reply token are
    revoked.
-   Kernel-internal adversarial tests cover server teardown for
    memory-bearing calls: queued moved-memory calls complete as
    `EndpointClosed` without reviving the moved-from cap, and delivered
    write-borrow calls complete as `Cancelled` while revoking the server
    borrow and preserving the server's last write for the owner.
-   The current tree adds `Copy` memory IPC at the kernel, syscall, and
    EL0 SVC layers: the receiver gets a new memory object containing a
    byte copy, while the sender keeps its original cap and receiver
    writes do not mutate the original.
-   The current tree implements Phase 3: syscall 39 call-time
    connection delegation with re-delegation and rights attenuation,
    `crates/catten/src/service/` (loader, bootstrap contract,
    supervisor), the `crates/catten-services` reference EL0 programs
    (name service, echo, client), and the `el0_service` boot-time test.
    The test spawns three isolated EL0 protection domains and verifies
    the complete flow — bootstrap capability delivery, REGISTER with
    attached re-delegable connection, LOOKUP returning an attenuated
    connection plus generation, echo call, voluntary shutdown, domain
    teardown, stale connection failing `EndpointClosed`, restart, and
    generation-2 re-lookup — against the live EL0 name service.
    Architectural findings: (a) call-time connection attachment plus
    connection re-delegation are the only two kernel mechanisms the
    name service needed — naming stayed entirely in userspace; (b) the
    receive ABI had to widen to `x8`, which required moving smoke-stub
    state out of `x8` and demonstrates why hand-written stubs should
    migrate to `catten-syscall` wrappers; (c) pending calls needed an
    explicit `ResultObserved` state so a caller can close a completed
    call cap without losing capabilities it received in the reply.
-   A follow-up increment adds memory-carried (long) service names:
    syscall 40 (`IPC_SCALAR_CALL_CONNECTION_COPY`) carries a copied
    memory object and a minted connection on one registration call, the
    name service resolves both scalar and memory-carried names through
    a single byte-keyed registry, and the reference echo/client
    programs register and resolve a 30-byte name end to end. Kernel
    self-test `test_endpoint_ipc_connection_copy` covers the combined
    attachment, copied-name delivery and verification, sender-ownership
    retention under copy, and reclamation of both attachments when a
    queued combined call is cancelled.
-   The first Phase 6 slice normalizes the operation model: completion
    lifecycle state is now an explicit named state machine
    (`InFlight → CancelPending → Completed → Observed`, §12.1/§18.2)
    replacing the previous scattered `cancelling`/`drained` booleans;
    every submission is assigned a monotonically allocated, never-reused
    `OperationId` (§8.2) distinct from the reusable capability slot
    index; and the effective terminal result (forced `Cancelled` when a
    cancel was pending) is what reaches both the capability and the CQ
    ring, with idempotent re-completion no longer able to post duplicate
    CQ entries.     Self-tests assert the state transitions, cancellation
    idempotence, slot-reuse-versus-operation-identity distinction, and
    ring/capability result agreement. Remaining Phase 6 work: per-shard
    CQ partitioning, a capability-free submission path keyed on
    `OperationId`, CQ batching, and migrating `sitas-charlotte` from
    busy polling to `CQ_WAIT`.
-   The second Phase 6 slice adds the capability-free submission path
    (§8.4): `submit_detached` returns an `OperationId` without
    consuming a capability-table slot, the submitter's `user_data`
    correlation cookie is what the CQ entry carries on completion, and
    `cancel_detached` forces the effective result to `Cancelled`.
    Detached operations share the submission-backpressure budget with
    capability-backed ones and use the same non-lossy backlog; the CQ
    entry's first field is now formally an opaque cookie (capability
    index or user data by protocol convention). A completed detached
    operation is reclaimed immediately — there is no post-terminal
    record, so double completion and post-completion cancellation are
    rejected.     Self-tests cover delivery, cancellation, budget sharing,
    reclamation, and refusal without an attached CQ. Remaining Phase 6
    work: per-shard CQ partitioning, the richer §8.2 completion record
    (status/flags/returned capability), CQ batching, and migrating
    `sitas-charlotte` from busy polling to `CQ_WAIT`.
-   The third Phase 6 slice retires the last busy-poll. The kernel CQ
    wait is now wake-aware and timed: `wake` posts a consume-on-wait
    cross-thread wake (§7.3/§9.4), `wait_on_cq` returns on either a
    completion or a wake, and `wait_on_cq_timeout` adds a deadline
    (syscalls `CQ_WAKE` = 41 and `CQ_WAIT_TIMEOUT` = 42, plus
    `catten-syscall` wrappers). `sitas-charlotte`'s `CharlotteReactor`
    was migrated off its `core::hint::spin_loop` busy poll: its `wait`
    now drains the ring and then blocks in `CQ_WAIT`/`CQ_WAIT_TIMEOUT`,
    its `ReactorWaker::wake` posts `CQ_WAKE`, and `sleep` blocks on a
    timed CQ wait rather than spinning. Kernel self-test
    `test_cq_wait_wake` proves a thread blocked in `wait_on_cq` is
    released both by a posted completion and by an explicit wake with no
    entry. `basic_kv`'s data path uses `spin_recv` on shard channels
    rather than the reactor, so the migration is verified for
    no-regression by the existing `el0_sitas` smoke test. Until the CQ
    is partitioned per shard, the wake is process-wide (one ring per
    address space), so a wake releases every blocked shard of the
    process rather than one target LP. Remaining Phase 6 work: per-shard
    CQ partitioning, the richer §8.2 completion record, and CQ batching.
-   The fourth Phase 6 slice partitions completion queues per shard
    (§8.1) and batches backlog delivery. An address space now owns a set
    of queues keyed by `CqId` (queue 0 is the default and the
    destination for capability-backed completions); each queue has its
    own ring, non-lossy backlog, pending wake, and blocked waiters.
    `submit_detached` selects a delivery queue, `open_cq`/`open_cq_phys`
    attach additional per-shard rings, and the `CQ_WAIT`/`CQ_WAKE`/
    `CQ_WAIT_TIMEOUT` syscalls take a queue id (0 preserves the previous
    behaviour, which the sitas reactor uses until per-shard rings are
    mapped into user space). Backlog flushes are now batched: retained
    entries are published with a single ring head update. Self-tests
    cover queue routing isolation, refusal of unknown queues, and a
    blocked wait on a second queue being released by a wake targeted at
    that queue. Remaining Phase 6 work: the richer §8.2 completion
    record and mapping per-shard rings into userspace (a loader-contract
    extension) so sitas shards can each wait on their own queue.
-   The first Phase 7 slice unifies the shard wait at the kernel:
    `IPC_ENDPOINT_BIND_CQ` (syscall 43) binds an endpoint's readiness to
    one of the owner's completion queues. The kernel posts a coalesced
    wake to that queue on the endpoint's empty→nonempty transition and
    on closure (readiness is a notification to inspect the endpoint,
    not a completion record — §16.3; wakes coalesce per §9.4). A shard
    can therefore block on one `CQ_WAIT` and be released by kernel
    completions, explicit peer wakes, and endpoint messages alike; a
    release with an empty ring means "drain your endpoints". Self-test
    evidence: a thread blocked in `wait_on_cq` is released by a posted
    completion, an explicit wake, a per-queue wake on a second shard
    queue, and an incoming IPC message on a CQ-bound endpoint, which it
    then receives (success criterion 5's kernel mechanism). Remaining
    Phase 7 work is userspace: a sitas executor loop that drains the CQ
    and ready endpoints, wakes tasks, and re-arms the single wait.
-   The second Phase 7 slice demonstrates the unified wait in a real
    EL0 service. The service loader now maps a CQ ring page at the
    canonical `0x11000` into every service domain and opens the
    kernel-side default queue, so services can use the completion
    syscalls, detached operations, timed waits, and readiness binding.
    The reference echo service was converted from blocking receive to
    the §7 event-loop skeleton: it binds its endpoint to queue 0, blocks
    on one `CQ_WAIT`, and drains every ready message (`IPC_RECV` until
    `NoMessage`) before re-arming — the existing end-to-end service
    test (lookup, calls, shutdown, restart, generation bump) passes
    unchanged over the event-driven server. The memory-name scratch
    address moved above the image (`0x100000`) to make room for the CQ
    page in the fixed layout. Outstanding: porting this loop into the
    sitas executor proper (task wakeup and budgeted polling), per-shard
    ring mapping for multi-shard services, and replacing
    `kv::spin_recv`'s channel busy-wait with the wake path.
-   The first Phase 8 slice builds the kernel device-capability
    mechanism — the substrate a userspace driver needs (architecture doc
    §10) — reusing the Phase 7 notification machinery. A new
    `crates/catten/src/device/` subsystem adds two first-class object
    types as derived facilities on the three primitives: **MmioRegion**
    (a page-granular device register window) and **Interrupt** (a routed
    interrupt source). MMIO regions map into an EL0 driver domain through
    a new user-accessible Device-nGnRnE page path
    (`Walker::map_user_mmio_page` /
    `AddressSpace::map_user_mmio_page`, execute-never, non-zeroed).
    Interrupt readiness is delivered exactly like endpoint readiness
    (§16.3): the AArch64 IRQ dispatcher's previously-unhandled SPI arm
    now calls `device::deliver_interrupt`, which masks the source at the
    GICv3 distributor (new `enable_spi`/`disable_spi`/`set_spi_pending`/
    `clear_spi_pending` with Group-1 config and `GICD_IROUTER` affinity
    routing), marks the interrupt object pending, and posts a **coalesced
    wake** to the owning driver's completion queue — so a driver shard
    blocked in one `CQ_WAIT` wakes for device interrupts, completions,
    and endpoint messages alike (§9.4, unified shard wait of §7). The
    interrupt-context path uses `try_lock` throughout and never blocks.
    Grants are minted only kernel-side (the supervisor), never through a
    syscall, so a driver receives only its delegated MMIO and IRQ
    authority (§10.1). Syscalls `44..=48` (`DEVICE_MMIO_MAP`,
    `DEVICE_MMIO_UNMAP`, `DEVICE_IRQ_BIND_CQ`, `DEVICE_IRQ_ACK`,
    `DEVICE_CLOSE`) plus `catten-syscall` wrappers expose the driver-side
    operations; `close_user_address_space` reclaims device caps
    (unmapping regions, masking and unrouting interrupts) on teardown.
    Kernel self-test `test_device_capabilities` proves the capability
    model (unknown-cap, wrong-type, unbound-ack, unmapped-unmap
    rejections; double-map and double-bind rejections), maps and unmaps
    an MMIO region in a real address space, and releases a thread blocked
    in one `wait_on_cq` **both** through the deterministic kernel
    delivery path and through a real GICv3 software-pended SPI routed by
    the live interrupt path, verifying pending/ack/re-arm state across
    rounds. This is the kernel half of success criterion 8. Outstanding
    (next slice): the EL0 UART driver service itself — supervisor device
    grants delivered through the bootstrap contract, a console endpoint
    protocol, deferred read replies, and an end-to-end EL0 test with
    driver restart and device reset (success criteria 8 and 9). Known
    prototype risk: `deliver_interrupt` calls `completion::wake` (which
    takes the completions lock) from interrupt context; the `try_lock`
    discipline avoids a same-core deadlock by degrading to "no wake this
    delivery" under contention, but a durable design should hand the
    wake to a deferred, lock-free path.
-   The second Phase 8 slice adds the reference **userspace UART driver**
    — the first complete userspace driver (architecture doc §10) — and
    proves the delegated-authority model end to end at EL0. The
    config-page contract is extended with two device-capability slots
    (`MMIO_CAP_OFFSET`, `IRQ_CAP_OFFSET`, mirrored in `catten-rt`), and a
    new supervisor entry point `spawn_driver_with_name_service` grants a
    driver domain exactly a `DriverGrant { mmio_phys_base, mmio_pages,
    intid }` — an MMIO region and an interrupt minted kernel-side — plus a
    bootstrap connection to the name service, and nothing else (§10.1).
    The `catten-services` crate gains a console protocol
    (`charlotte-protocol-console` v1: `OP_WRITE`/`OP_STATUS`/
    `OP_SHUTDOWN`), the `uart` driver program, and a `cclient` console
    client. The driver maps its delegated PL011 register window into its
    own address space as EL0 device memory (`device_mmio_map`),
    registers a console endpoint by name, binds both endpoint readiness
    and its interrupt to the default completion queue, and serves from
    one `CQ_WAIT` (the unified shard wait of §7): each `OP_WRITE`
    transmits a byte through the PL011 transmit FIFO with a **direct EL0
    MMIO write**, and each device interrupt is acknowledged and re-armed
    with `device_irq_ack`. The end-to-end self-test `test_el0_uart`
    spawns the name service, the driver (granted the real QEMU `virt`
    PL011 at `0x0900_0000` and its SPI, INTID 33), and the console
    client from real ELFs; it verifies the client looks the driver up by
    name and writes a message that the driver transmits through the
    device registers from EL0, then software-pends the real PL011 SPI
    through the GIC and observes the driver acknowledge a delegated
    device interrupt from EL0. This is the EL0 half of success
    criterion 8: a userspace driver runs with only delegated MMIO and IRQ
    authority. Outstanding: interrupt-driven receive with deferred read
    replies, driver restart with device reset and outstanding-operation
    reconciliation (success criterion 9's driver half), and moving the
    interrupt-context wake to a deferred lock-free path.
-   Both Phase 8 slices are boot-validated in QEMU (`-M
    virt,gic-version=3`): `test_device_capabilities` and `test_el0_uart`
    print their SUCCESS lines and the run reaches "Testing Complete. All
    Tests Passed!". Two findings surfaced only at run time. (a)
    `enable_spi`/`disable_spi` touch the GIC distributor MMIO, but the
    mapping installed by `GicV3::init_lp` lives in the boot kernel
    address space; an SPI configuration call from a self-test thread
    running under a different active translation regime faulted, so the
    GIC layer now maps the distributor into the *current* address space
    (idempotently) before each SPI access. (b) The interrupt-context wake
    did not deadlock in practice — the driver shard is blocked in
    `CQ_WAIT` (not holding the completions lock) when its interrupt fires
    — but the `try_lock` degradation path remains the documented
    durable-design gap. A `scripts/qemu-boot-macos.sh` helper reproduces
    the boot on a macOS host using `hdiutil`/`diskutil` in place of
    `losetup`/`parted`/`mkfs.fat` and the rustup `llvm-ar` for the
    flanterm C library.
-   The third Phase 8 slice adds **interrupt-driven deferred read
    replies** (§7.2): the console protocol gains `OP_READ_DEFERRED`, on
    which the driver does *not* reply — it retains the reply token,
    returns to its single `CQ_WAIT`, and completes the retained reply
    only when the next device interrupt arrives, reading the PL011
    receive register (a real EL0 MMIO read; the reply encodes the byte in
    bits 0..8 and the driver's interrupt count above, so the caller can
    verify the reply was interrupt-driven). The driver also unmasks the
    PL011 receive interrupt (`IMSC.RXIM`) and clears it on service
    (`ICR`), and publishes a READ_ARMED config marker while a token is
    retained. The console client issues the deferred read after its
    writes; the verifier waits for READ_ARMED, software-pends the real
    PL011 SPI **once**, and asserts the read completed with a nonzero
    interrupt count. Boot-validated in QEMU (deferred-read result
    `0x100`: RX empty, completed by interrupt #1). Architectural
    findings: (a) the reply token *is* the natural deferred-completion
    object — the driver needed no new kernel mechanism to hold a request
    across an interrupt, exactly as §7.2 intends; (b) wake discipline
    matters: an earlier version of the verifier software-pended the SPI
    in a tight loop, and the resulting storm of `completion::wake` →
    `submit_ready_thread` calls hit a scheduler race in which waking an
    already-runnable thread panicked (`ThreadAlreadyAssignedToLp`) — the
    same non-idempotent-wake fragility that the post-boot `cross_lp_demo`
    tripped as `AlreadyBlocked` (`sleep`'s block/yield window versus a
    same-LP timer fire). This was then fixed at the source (see the
    scheduler slice below) rather than only avoided.
-   A scheduler slice makes wakes **idempotent**, which the unified shard
    wait requires: `submit_ready_thread`/`add_thread` treat re-admitting
    an already-`Ready`/`Running` thread as a benign no-op (the event
    state — `wake_pending`, ring entries, endpoint queues — is posted
    before the wake and re-checked by the waiter, so no wake is lost),
    a late wake for an exited thread returns `InvalidThread` instead of
    panicking, and `RoundRobin::next` re-queues the outgoing thread only
    while it is still `Running` (so a thread woken in the
    `block_thread`→context-switch window is not double-enqueued). This
    removes both the `ThreadAlreadyAssignedToLp` and the long-standing
    `cross_lp_demo` `AlreadyBlocked` panics; boot-validated in QEMU with
    the demo running to completion (all 8 receivers) alongside the Phase 8
    tests, zero panics.
-   The fourth Phase 8 slice completes **success criterion 9's driver
    half**: driver restart with device reset and outstanding-operation
    reconciliation (§13). The console protocol gains `OP_CRASH` — an
    uncooperative driver exit that releases neither its device
    capabilities nor any retained reply, modelling a crash — and
    `device::interrupt_route_owner` reports which domain currently owns an
    interrupt route. The `test_el0_uart` verifier (a second console
    client via the direct kernel API) leaves a deferred read outstanding,
    crashes the driver, and verifies teardown (a) reclaims the delegated
    device authority — the interrupt route is unrouted
    (`interrupt_route_owner → None`) and the MMIO mapping is torn down
    with the address space (device reset); (b) reconciles the outstanding
    operation — the orphaned deferred read completes as `Cancelled`
    rather than hanging, because the retained reply token is reclaimed on
    teardown; and (c) invalidates stale connections — calls on the dead
    instance fail `EndpointClosed`. A restarted generation-2 instance then
    registers under the same name with freshly minted device grants,
    serves a console write, and owns the interrupt route again.
    Boot-validated in QEMU. With this slice, success criteria 8 and the
    driver half of 9 are met; the general userspace-driver model
    (authority delegation, EL0 MMIO, IRQ→CQ delivery, deferred replies,
    supervised restart with device reset) is demonstrated end to end
    without networking. Remaining Phase 8/9 work moves to the virtio-net
    driver and network service (Phase 9), plus moving the
    interrupt-context wake to a deferred lock-free path (Phase 10).



------------------------------------------------------------------------

## 1. Executive conclusion

CharlotteOS, sitas, and Xous fit together, but not by collapsing their
abstractions into one universal "async handle."

The coherent architecture is:

``` text
CharlotteOS kernel
    protection domains
    threads and LP scheduling
    capabilities and endpoint connections
    interrupt routing
    memory objects and page mappings
    operation submission and completion queues
    minimal notifications and timers

Userspace service process
    one or more externally visible IPC endpoints
    protocol-specific request/reply handling
    one sitas executor per assigned shard
    shard-local state
    typed in-process commands
    driver or service implementation

Client process
    typed protocol library
    endpoint connection capabilities
    futures backed by IPC replies or completion records
    owned buffers whose transfer mode is explicit
```

The most important redirection is:

> **Do not model all inter-process activity as asynchronous syscalls
> returning completion capabilities.**

Instead, CharlotteOS should provide **two complementary kernel
mechanisms**:

1.  **Xous-inspired endpoint IPC** for communication across protection
    domains:
    -   connection capabilities;
    -   bounded server queues;
    -   small scalar messages;
    -   page or memory-object messages;
    -   explicit copy, move, immutable-lend, and mutable-lend semantics;
    -   synchronous and deferred replies;
    -   service discovery outside the kernel.
2.  **Completion queues** for asynchronous operations whose lifecycle
    belongs to the kernel or a device:
    -   timers;
    -   device and DMA operations;
    -   asynchronous kernel work;
    -   completion of operations submitted through a resource
        capability;
    -   aggregated waiting by a sitas shard.

Sitas then supplies the third layer:

3.  **Shard-local asynchronous execution inside a process**:
    -   one executor per shard;
    -   cooperative futures;
    -   typed internal commands;
    -   bounded mailboxes;
    -   shard-owned mutable state;
    -   explicit placement and flow ownership.

This separation prevents several conceptual mistakes in the earlier
draft:

-   a Rust `Waker` is not a kernel completion;
-   a completion capability is not an IPC endpoint;
-   an endpoint connection is not merely a waitable event;
-   a shard is not identical to an LP;
-   a wake is not identical to an IPI;
-   "async throughout" does not mean that every syscall must allocate an
    asynchronous operation.

------------------------------------------------------------------------

# 

# 2. Fundamental Kernel Objects

Before discussing IPC, drivers, scheduling, or asynchronous execution,
the architecture should be understood in terms of three orthogonal
kernel abstractions.

These are the conceptual foundation of CharlotteOS.

``` text
                 Kernel Objects
                      │
      ┌───────────────┼────────────────┐
      │               │                │
 Capabilities      Endpoints      Memory Objects
      │               │                │
  Authority      Communication     Ownership
   Security        Rendezvous     Data Movement
```

Every other kernel facility is a composition of these three primitives.

## 2.1 Capabilities

Capabilities answer one question:

> "May I perform this operation?"

They represent authority, not work.

Examples include:

-   endpoint capabilities
-   connection capabilities
-   memory-object capabilities
-   interrupt capabilities
-   DMA-domain capabilities
-   timer capabilities

The capability model should remain independent of scheduling and IPC.

## 2.2 Endpoints

Endpoints answer:

> "Who am I communicating with?"

They provide bounded rendezvous between protection domains.

Endpoints know nothing about packet buffers, futures, DMA, or ownership.
They carry requests and replies.

## 2.3 Memory Objects

Memory objects answer:

> "Where is the data, and who currently owns it?"

They represent page-backed storage whose ownership can move
independently of communication.

Their responsibilities include:

-   page ownership
-   mappings
-   transfer modes
-   DMA state
-   lifetime

## 2.4 Derived facilities

Everything else should be expressible as compositions.

 | Facility          | Capabilities | Endpoints | Memory Objects |
 |-------------------|--------------|-----------|----------------|
 | Completion Queue  | ✓           | ✓        |                |
 | Reply Token       | ✓           | ✓        |                |
 | Socket            | ✓           | ✓        | ✓             |
 | Pipe              | ✓           | ✓        | ✓             |
 | Userspace Driver  | ✓           | ✓        | ✓             |
 | Network Stack     | ✓           | ✓        | ✓             |
 | Filesystem Read   | ✓           | ✓        | ✓             |
 | DMA Mapping       | ✓           |           | ✓             |

This gives the architecture an explicit design rule:

> When adding a new subsystem, first ask whether it can be expressed
> using these three primitives. New kernel primitives should be
> introduced only when they cannot be naturally composed from the
> existing ones.

# 2. The combined thesis

The three systems approach the same design space from different
directions.

### 2.1 CharlotteOS

CharlotteOS contributes the mechanisms that must be privileged:

-   address-space isolation;
-   kernel scheduling;
-   LP-local state and interrupt routing;
-   page-table manipulation;
-   MMIO and DMA authority;
-   capability tables;
-   event observation and thread wakeup;
-   syscall dispatch;
-   asynchronous completion delivery.

The CharlotteOS fork already contains substantial experimental work in
this direction, including completion capabilities, a shared
completion-queue ring, real AArch64 EL0/SVC execution, bounded IPI
queues, typed kernel mailboxes, `ShardLocal<T>`, and a no-std sitas
userspace smoke path.

### 2.2 Sitas

Sitas contributes an execution discipline:

> Only the owning shard mutates shard-local state. Cross-shard
> interaction occurs through bounded typed messages carrying owned
> values.

Its main value for CharlotteOS is:

-   one executor per shard;
-   cooperative task scheduling;
-   futures as userspace state machines;
-   bounded backpressure;
-   shard-local ownership;
-   explicit placement;
-   typed command/reply APIs;
-   structured shutdown and cancellation;
-   observability through owned snapshots.

### 2.3 Xous

Xous contributes the missing protection-domain service model.

Its most valuable ideas are:

-   services are userspace servers;
-   clients connect to servers and receive connection identifiers;
-   connections express authority;
-   IPC is a kernel primitive rather than a convention over shared
    memory;
-   small scalar messages are distinct from memory-bearing messages;
-   memory IPC distinguishes ownership transfer from borrowing;
-   mutable lending temporarily grants exclusive writable access;
-   a userspace name service maps human-readable names to otherwise
    opaque server identities;
-   drivers and system services can live outside the kernel.

CharlotteOS currently has strong completion machinery and an emerging
mailbox model, but it does not yet have a sufficiently explicit, durable
**userspace server and memory-message IPC architecture**. That is the
principal gap Xous helps expose.

------------------------------------------------------------------------

## 3. Architectural vocabulary

The implementation should use the following terms consistently.

### 3.1 Protection domain

A **protection domain** is an address space and authority boundary. It
owns:

-   a capability table;
-   memory mappings;
-   one or more threads;
-   resource accounting;
-   failure and teardown state.

A process may be the concrete implementation of a protection domain, but
architectural documents should prefer the semantic term where
appropriate.

### 3.2 Execution context

An **execution context** is a kernel-scheduled thread.

It may have:

-   LP affinity;
-   priority or scheduling class;
-   a current protection domain;
-   a wait state;
-   an associated userspace executor.

### 3.3 Logical processor

An **LP** is CharlotteOS's representation of a schedulable hardware
execution context.

An LP is not a sitas shard. A useful initial mapping is:

``` text
one sitas shard
    ↔ one LP-affine userspace thread
    ↔ normally one logical CPU
```

The architecture must retain the distinction so that CPU hotplug, test
oversubscription, migration, asymmetric CPUs, and multiple sharded
processes remain possible.

### 3.4 Sitas shard

A **shard** is a userspace ownership and execution domain:

-   one executor runs its tasks;
-   at most one task at a time mutates shard-owned state;
-   cross-shard access occurs through messages;
-   placement is explicit but is not the shard's identity.

### 3.5 Endpoint

An **endpoint** is a bounded server-side IPC receive object.

It has:

-   an owning protection domain;
-   a queue capacity;
-   a protocol/interface identity;
-   receive rights;
-   lifecycle and closure state.

### 3.6 Connection capability

A **connection capability** is a client-side authority to send or call
an endpoint.

Possession of a service name is not authority. A name service or policy
service resolves a name and conditionally returns a connection
capability.

### 3.7 Message

A **message** is a kernel-validated IPC envelope consisting of:

-   interface or protocol identifier;
-   opcode;
-   flags;
-   inline scalar words;
-   zero or more memory-object attachments;
-   zero or more delegated capability attachments;
-   optional reply authority.

The kernel understands the envelope, not arbitrary Rust types.

### 3.8 Reply token

A **reply token** is one-shot authority to complete a blocking or
deferred call.

It is:

-   created by the kernel;
-   bound to the caller's pending call;
-   consumable exactly once;
-   invalidated by cancellation, caller death, or endpoint teardown;
-   optionally delegable when explicitly allowed.

### 3.9 Resource capability

A **resource capability** authorizes operations on a kernel-managed
object such as:

-   a memory object;
-   an endpoint;
-   a connection;
-   an interrupt;
-   an MMIO region;
-   a DMA domain;
-   a device;
-   a completion queue;
-   a timer.

### 3.10 Operation and completion queue

An **operation** is submitted work whose progress may depend on the
kernel, hardware, or a privileged service.

A **completion queue (CQ)** is the aggregated, per-shard or per-process
destination for operation results. It is waitable and has non-lossy
terminal completion semantics.

### 3.11 Notification

A **notification** means:

> State may have changed; inspect the corresponding object or queue.

It may be coalesced. It is not itself a result.

### 3.12 Rust `Waker`

A Rust `Waker` means:

> Poll this userspace future again.

It is a userspace scheduling hint. It is not a kernel completion record
and should not cross the ABI directly.

------------------------------------------------------------------------

## 4. What "asynchronous throughout" means

"Asynchronous throughout" should be retained as a design goal, but
defined precisely.

It means:

1.  operations that may wait for external progress have
    submission/completion semantics;
2.  no subsystem boundary forces the caller to dedicate a thread to
    waiting for one operation;
3.  a userspace shard can wait on one aggregated kernel source;
4.  completions are retained until observed;
5.  buffers and capabilities have explicit ownership during an
    operation;
6.  cancellation and teardown have defined state transitions;
7.  bounded queues preserve backpressure across layers;
8.  service implementations can suspend one task while continuing other
    work on the shard.

It does **not** mean:

-   every syscall is asynchronous;
-   every call returns a newly allocated completion capability;
-   no thread ever blocks;
-   every wake sends an IPI;
-   all waitable objects have identical semantics;
-   userspace code is interrupted by arbitrary asynchronous callbacks.

### 4.1 Blocking is still required

When no sitas task is runnable, the shard's execution context should
block in one kernel wait operation.

That is desirable:

``` text
ready tasks exist
    → poll tasks

no ready tasks
    → wait on CQ / endpoint readiness / deadline

event arrives
    → execution context becomes runnable
    → drain events
    → wake relevant futures
```

CharlotteOS's advantage is not "threads never park." Its advantage is:

> The shard parks directly on native completion and message state; no
> helper-thread pool is required merely to convert blocking kernel
> interfaces into futures.

### 4.2 Immediate operations remain immediate

Operations that cannot meaningfully block should complete synchronously.
Examples include:

-   capability duplication;
-   metadata queries;
-   nonblocking CQ drain;
-   validation-only operations;
-   querying current process/thread identity;
-   closing an already quiescent object.

Potentially waiting operations must have an asynchronous form. They may
still complete immediately.

### 4.3 No asynchronous userspace upcalls

Ordinary completions should not inject callbacks into userspace at
arbitrary instructions.

The path should be:

``` text
hardware interrupt
    → kernel records result or notification
    → blocked execution context becomes runnable
    → normal return to userspace
    → sitas executor drains CQ/endpoints
    → executor wakes relevant Rust tasks
```

Kernel preemption is allowed. Arbitrary userspace task upcalls are not
required.

------------------------------------------------------------------------

## 5. Two kernel data paths, not one

The earlier CharlotteOS/sitas draft leaned toward a universal
completion-capability model. Xous shows why that is insufficient.

CharlotteOS needs two distinct paths.

## 5.1 Endpoint IPC path

Use endpoint IPC when one protection domain invokes another protection
domain.

Examples:

-   application → network service;
-   network service → NIC driver;
-   application → filesystem service;
-   filesystem service → block service;
-   service → name service;
-   driver manager → driver process.

The receiver is a userspace server, not the kernel operation engine.

``` text
client task
    → send/call through connection capability
    → kernel validates and enqueues message
    → server endpoint becomes ready
    → server shard receives message
    → service handles or delegates work
    → optional reply token completed
```

## 5.2 Operation completion path

Use submission/completion when the operation belongs to the kernel or
hardware-facing mechanism.

Examples:

-   timer expiry;
-   asynchronous page operation;
-   IRQ-derived device completion;
-   DMA completion;
-   waiting for thread/process exit;
-   kernel-managed asynchronous object work.

``` text
userspace submits operation
    → kernel validates resource capability and buffers
    → operation becomes in flight
    → result posted to CQ
    → shard wakes and consumes result
```

## 5.3 Composition

A userspace driver may receive an endpoint message and then submit a
kernel operation.

Example:

``` text
network service sends TX packet to NIC driver endpoint
    → NIC driver validates message
    → NIC driver submits/updates hardware descriptor
    → IRQ arrives
    → driver CQ receives device completion
    → driver replies or posts protocol completion to network service
```

The endpoint message and the hardware completion are related, but they
are not the same kernel abstraction.

------------------------------------------------------------------------

## 6. Xous-inspired IPC for CharlotteOS

## 6.1 Server and connection model

CharlotteOS should add kernel objects analogous to Xous servers and
connections, but use capability terminology explicitly.

Proposed conceptual API:

``` rust
fn endpoint_create(
    interface: InterfaceId,
    version: InterfaceVersion,
    capacity: usize,
) -> Result<EndpointCap, IpcError>;

fn connection_mint(
    endpoint: &EndpointCap,
    rights: ConnectionRights,
) -> Result<ConnectionCap, IpcError>;

fn connection_delegate(
    connection: &ConnectionCap,
    target_domain: ProtectionDomainCap,
    reduced_rights: ConnectionRights,
) -> Result<(), IpcError>;
```

Ordinary clients should not mint arbitrary connections. A service
manager or name/policy service normally delegates them.

## 6.2 Name service outside the kernel

Follow Xous's principle:

-   the kernel uses opaque endpoint identities and capabilities;
-   a userspace name service maps human-readable names to services;
-   lookup is policy controlled;
-   service restart changes instance generation;
-   stale connections fail deterministically.

Suggested identity:

``` rust
struct ServiceInstanceId {
    service_uuid: [u8; 16],
    generation: u64,
}
```

A name such as `"system.network.v1"` is discovery metadata, not
authority.

## 6.3 Message classes

CharlotteOS should support at least:

``` rust
enum MessageKind {
    ScalarSend,
    ScalarCall,
    MemorySend,
    MemoryCall,
}
```

The public API may expose more ergonomic operations, but the ABI should
keep small messages distinct from memory-bearing messages.

### Scalar message

Suitable for:

-   opcodes;
-   handles;
-   small numeric arguments;
-   status values;
-   queue indices;
-   compact control-plane operations.

### Memory message

Suitable for:

-   structured requests;
-   strings and names;
-   packets or blocks;
-   larger replies;
-   descriptors and configuration structures.

## 6.4 Memory transfer modes

Adopt the strongest Xous idea and generalize it around CharlotteOS
memory objects:

``` rust
enum TransferMode {
    Copy,
    Move,
    BorrowRead,
    BorrowWrite,
}
```

### Copy

-   receiver gets copied data;
-   sender retains ownership;
-   simplest semantics;
-   suitable for small or untrusted input.

### Move

-   ownership transfers to the receiver;
-   sender loses mappings and use rights;
-   receiver becomes responsible for later transfer or destruction;
-   safe Rust wrapper consumes the buffer value.

### BorrowRead

-   receiver receives temporary read-only mapping;
-   sender retains ownership;
-   reply or call completion revokes receiver mapping;
-   writable aliases in the receiver are forbidden.

### BorrowWrite

-   receiver receives temporary exclusive writable mapping;
-   sender access is revoked for the duration;
-   reply or call completion restores sender ownership/access;
-   mutable lending must be hardware enforced, not inferred from Rust
    types.

The implementation must not rely on safe Rust alone. Unsafe code or a
non-Rust process must still be contained by page tables and capability
checks.

## 6.5 Proposed message envelope

``` rust
#[repr(C)]
struct MessageHeader {
    interface: u128,
    version: u32,
    opcode: u32,
    flags: u32,
    inline_len: u16,
    segment_count: u16,
    capability_count: u16,
    reserved: u16,
    user_tag: u64,
}

#[repr(C)]
struct MemoryAttachment {
    object: CapabilityId,
    offset: u64,
    len: u64,
    mode: TransferMode,
}
```

Constraints:

-   fixed-width ABI fields;
-   explicit alignment;
-   no Rust enum layout assumptions;
-   no persistent use of `usize`;
-   checked offsets and lengths;
-   explicit protocol versioning;
-   maximum attachment counts;
-   no arbitrary userspace pointers as durable message identity.

## 6.6 Typed Rust protocols remain in userspace

The kernel does not understand `M: Send`, Rust enums, or drop semantics.

A protocol crate supplies typed wrappers:

``` rust
trait Protocol {
    const INTERFACE: InterfaceId;
    const VERSION: u32;

    type Request;
    type Response;
}
```

Generated or hand-written bindings encode requests into the kernel
message envelope.

``` rust
struct Client<P: Protocol> {
    connection: ConnectionCap,
    _protocol: PhantomData<P>,
}
```

The existing kernel `ShardMailbox<M>` remains useful internally. It must
not be mistaken for the stable userspace IPC ABI.

------------------------------------------------------------------------

# 6A. Memory Objects and Ownership Transfer

The endpoint IPC model defines **who communicates** and **how requests
are routed**. The memory-object model defines **how data crosses
protection-domain boundaries**.

These are independent concerns and should evolve independently.

## 6A.1 First-class memory objects

CharlotteOS should introduce a dedicated kernel object representing
page-backed memory with explicit ownership and capability semantics.

The kernel manages:

-   physical frames;
-   mappings;
-   ownership;
-   transfer state;
-   DMA state;
-   lifetime.

Userspace never exchanges raw pointers across protection domains.
Instead, IPC messages attach **memory-object capabilities**.

## 6A.2 Transfer semantics

The architecture adopts four transfer modes:

-   **Copy** --- duplicate data.
-   **Move** --- transfer exclusive ownership.
-   **BorrowRead** --- temporary immutable mapping.
-   **BorrowWrite** --- temporary exclusive writable mapping.

The MMU enforces these semantics. Rust ownership complements, but does
not replace, kernel enforcement.

## 6A.3 Ownership rather than copying

The preferred primitive is:

> Transfer ownership by changing mappings instead of copying bytes.

For `Move`, the kernel:

1.  validates ownership;
2.  reserves destination queue space;
3.  prepares receiver mappings;
4.  removes sender mappings;
5.  performs required TLB invalidation;
6.  commits ownership;
7.  publishes the IPC message.

Physical pages never move.

## 6A.4 Control plane and data plane

Endpoint IPC belongs to the control plane.

Memory objects belong to the data plane.

For networking:

-   endpoint IPC configures queues and drivers;
-   descriptor rings coordinate producer/consumer state;
-   packet payloads move as memory objects.

This avoids turning every packet into a copied IPC payload while
preserving strict ownership.

## 6A.5 DMA

DMA introduces a third ownership dimension:

-   CPU ownership;
-   DMA ownership;
-   Rust/API ownership.

Future IOMMU/SMMU integration should extend capability enforcement to
DMA transactions.

## 6A.6 Why this section comes before request/reply

Request/reply semantics build upon the memory model.

Once endpoint IPC and transfer semantics are defined, synchronous calls,
deferred replies, userspace drivers, and networking all become natural
applications of the same primitives.

------------------------------------------------------------------------

## 7. Request/reply and asynchronous service calls

Xous is commonly synchronous at the client API even though communication
is message based. CharlotteOS and sitas should retain both
synchronous-looking futures and true asynchronous service execution.

## 7.1 Client call

A client performs:

``` rust
let response = network.connect(address).await?;
```

The Rust API is asynchronous. Underneath it:

1.  the runtime sends a call message;
2.  the kernel creates a one-shot reply token;
3.  the caller task becomes pending;
4.  the caller shard continues running other tasks;
5.  the server receives the message;
6.  the server replies immediately or retains/delegates the reply token;
7.  reply arrival wakes the client future.

No client OS thread is dedicated to the request.

## 7.2 Deferred replies

A server must be able to retain reply authority while waiting for other
work:

``` rust
struct PendingRequest {
    reply: ReplyToken,
    buffer: BorrowedWriteBuffer,
}
```

This is essential for:

-   network receive;
-   filesystem read;
-   device operations;
-   service chains;
-   timeouts;
-   batching.

## 7.3 Service-side task routing

The external endpoint receiver should be an ingress adapter. It routes
work to a sitas shard:

``` rust
async fn endpoint_loop(
    endpoint: Endpoint,
    shards: ShardedSubmitter,
) -> Result<(), ServiceError> {
    loop {
        let envelope = endpoint.receive().await?;
        let shard = route(&envelope);

        shards.submit_to(shard, async move {
            handle_message(envelope).await
        }).await?;
    }
}
```

The service's mutable state remains shard-owned.

## 7.4 Avoid one server loop as a scalability constraint

Do not copy a simplistic "one process, one blocking receive loop"
structure from Xous.

A CharlotteOS service process may own:

-   several endpoints;
-   several receiving tasks;
-   several sitas shards;
-   one or more completion queues;
-   per-shard endpoint ingress queues.

The server abstraction is a protection and authority boundary, not a
single-thread requirement.

------------------------------------------------------------------------

## 8. Completion queues for kernel and device operations

## 8.1 Aggregated wait

A sitas shard should wait on one kernel-managed CQ or wait set, not
independently wait on every operation capability.

``` text
many operations
    → one per-shard CQ
    → one executor wait
```

## 8.2 Completion record

Suggested minimum:

``` rust
#[repr(C)]
struct CompletionEntry {
    operation: OperationId,
    user_data: u64,
    status: CompletionStatus,
    result: i64,
    flags: u32,
    returned_capability: CapabilityId,
}
```

A result may also refer to memory ownership returned through a
registered operation table.

## 8.3 Non-lossy terminal results

The current fork's decision to retain CQ overflow in a kernel backlog is
correct and must become an architectural invariant:

> A terminal operation result must never be silently lost because the
> userspace CQ ring is full.

The ring's overflow counter is pressure telemetry, not a count of
discarded completions.

The kernel may:

-   retain overflow in a per-domain backlog;
-   stop accepting new submissions;
-   propagate `WouldBlock`;
-   force the application to drain completions.

It must not discard terminal results.

## 8.4 Per-operation capability is optional

The existing experimental model returns a `CompletionCap` per
submission. Do not make this mandatory in the final ABI.

Use:

-   `OperationId` plus a CQ for the common path;
-   a first-class operation capability only where independent waiting,
    cancellation, delegation, inspection, or policy requires it.

This avoids capability-table pressure for high-rate operations.

## 8.5 Race-free waiting

CQ waiting must atomically avoid lost wakeups:

``` text
inspect CQ
if completion threshold met:
    return

arm waiter against CQ generation/state
reinspect CQ
if threshold met:
    disarm and return

block
```

The observer implementation is an internal mechanism. The ABI contract
is that state transition and waiter registration cannot lose a wake.

------------------------------------------------------------------------

## 9. Sitas inside a service process

## 9.1 One executor per shard

Each assigned shard normally has:

-   one LP-affine execution context;
-   one sitas executor;
-   one ready queue;
-   one timer structure;
-   one CQ or CQ partition;
-   shard-owned service state.

## 9.2 Internal messages are not kernel IPC

Within one protection domain, use sitas typed mailboxes where practical:

``` text
external cross-process boundary:
    CharlotteOS endpoint IPC

internal cross-shard boundary:
    sitas typed owned messages
```

Do not route every internal command through kernel endpoint IPC. That
would add unnecessary privilege crossings and ABI encoding.

## 9.3 Shard-local state

Retain the closure-based non-escaping `ShardLocal<T>` discipline, with
explicit kernel constraints.

For kernel `ShardLocal<T>`, access is valid only when:

-   execution is bound to the owning LP;
-   migration cannot occur during access;
-   no interrupt or deferred path accesses the same object;
-   re-entrant access is rejected;
-   panic/unwind behavior cannot leave the borrow flag permanently set.

Keep `PerLp<T>` for data requiring interrupt-safe or cross-LP access. Do
not present `ShardLocal<T>` as a universal replacement.

## 9.4 Cross-shard wake semantics

Replace the mental model:

``` text
wake = IPI
```

with:

``` text
wake = make the target shard eligible to observe queued work
```

Implementation rules:

-   enqueue before signaling;
-   send an IPI only when required;
-   coalesce wakes;
-   prefer empty-to-nonempty transition signaling;
-   avoid one IPI per message;
-   let the scheduler decide whether a remote reschedule interrupt is
    necessary.

------------------------------------------------------------------------

## 10. Userspace drivers

Xous's userspace-driver orientation should become a central CharlotteOS
direction.

## 10.1 Driver process authority

A driver manager creates a driver protection domain and delegates only
required resources:

``` rust
struct DriverGrant {
    device: DeviceCap,
    mmio: Vec<MmioRegionCap>,
    interrupts: Vec<InterruptCap>,
    dma_domain: Option<DmaDomainCap>,
    bootstrap_endpoint: EndpointCap,
}
```

A driver must not obtain arbitrary physical memory or arbitrary
interrupt vectors.

## 10.2 Interrupt delivery

The kernel interrupt path should:

1.  identify and acknowledge/mask the interrupt as required;
2.  mark an interrupt object pending;
3.  post a notification or CQ entry;
4.  make the owning driver shard runnable;
5.  return from the exception.

Repeated interrupts should coalesce where the device model permits it.

## 10.3 DMA isolation

Strong userspace-driver isolation requires an IOMMU/SMMU:

-   CPU page tables protect against driver CPU access;
-   IOMMU/SMMU protects against device DMA;
-   Rust ownership protects safe client code from accidental reuse.

Without an IOMMU/SMMU, userspace drivers still improve fault containment
and architecture, but a malicious or broken DMA device can violate
memory isolation.

## 10.4 Driver service layering

Use separate failure and authority domains where justified:

``` text
Application
    → socket endpoint
Network service
    → packet endpoint
NIC driver
    → MMIO / DMA / IRQ
Hardware
```

An initial implementation may combine socket and protocol-stack
functions, but the NIC driver should remain separately isolated.

## 10.5 Control plane versus data plane

Do not interpret message passing as "one IPC per packet descriptor."

### Control plane

Use endpoint IPC for:

-   queue creation;
-   device configuration;
-   buffer-pool registration;
-   MTU and link settings;
-   service discovery;
-   reset and lifecycle;
-   authority delegation.

### Data plane

Use:

-   shared descriptor rings;
-   transferred memory objects;
-   buffer-pool ownership;
-   batching;
-   per-queue/per-shard layout;
-   coalesced notifications;
-   CQ batches.

Example:

``` text
network shard writes N TX descriptors
    → signals only on required transition
NIC driver/hardware processes descriptors
    → produces N completions
driver posts batch notification/completion
network shard drains all available entries
```

------------------------------------------------------------------------

## 11. Backpressure

Backpressure must be defined end to end.

``` text
application request queue
    ↕
service endpoint queue
    ↕
service shard mailbox
    ↕
driver endpoint/ring
    ↕
kernel submission queue
    ↕
hardware descriptor ring
```

Every bounded layer needs explicit behavior:

``` rust
enum SendPolicy {
    Try,
    Wait,
    Deadline(Instant),
    DropIfCoalescible,
}
```

Rules:

-   terminal completions are never dropped;
-   coalescible notifications may be merged;
-   control messages normally fail or wait explicitly;
-   data-plane producers stop before unbounded allocation;
-   high-priority kernel-internal messages must not use arbitrary
    force-eviction of unrelated work;
-   emergency paths require reserved capacity or a separate priority
    queue.

The current bounded IPI queue is a useful experiment, but "force-evict
fallback" is not a durable general policy. Replace it with one or more
of:

-   reserved slots for mandatory kernel messages;
-   separate priority classes;
-   per-message retry/defer behavior;
-   synchronous fallback only where proven safe.

------------------------------------------------------------------------

## 12. Cancellation, teardown, and ownership

## 12.1 Operation state

Use an explicit state machine:

``` text
Created
    → Submitted
    → Accepted | Rejected
    → InFlight
    → Completed | Failed | Cancelled
    → ResultObserved
    → ResourcesReclaimed
```

## 12.2 Cancellation result

``` rust
enum CancelResult {
    Cancelled,
    AlreadyCompleted,
    TooLate,
    NotCancellable,
    UnknownOperation,
}
```

Cancellation may itself complete asynchronously.

## 12.3 Buffer ownership

For an operation receiving an owned buffer:

``` rust
async fn read(
    resource: ResourceCap,
    buffer: OwnedBuffer,
) -> Result<(OwnedBuffer, usize), ReadError>;
```

Safe Rust prevents ordinary reuse after move. The kernel must
additionally ensure:

-   memory-object ownership is valid;
-   pages remain stable while used;
-   DMA mappings are controlled;
-   ownership is returned exactly once;
-   cancellation does not free memory still reachable by the kernel or
    device;
-   domain death triggers cleanup, quarantine, reset, or deliberate leak
    according to policy.

Retain the sitas "drain or leak rather than use-after-free" principle
for experimental paths, but design the production kernel to reclaim
through explicit operation and device teardown.

## 12.4 IPC call cancellation

Cancellation of an endpoint call differs from cancellation of a kernel
operation.

The kernel must define:

-   whether an enqueued but unreceived call can be removed;
-   whether a delivered call can be marked cancelled;
-   whether the server may still complete it;
-   how lent mappings are revoked;
-   what happens to a delegated reply token;
-   whether cancellation is advisory or authoritative.

Do not reuse kernel-operation cancellation semantics blindly for service
calls.

------------------------------------------------------------------------

## 13. Service supervision and restart

Userspace drivers and services make failure semantics part of the
architecture.

A service manager should:

-   start protection domains;
-   grant capabilities;
-   register service names;
-   monitor process exit;
-   revoke stale connections;
-   restart according to policy;
-   increment service generation;
-   reset devices before driver restart;
-   reconcile or fail outstanding calls.

Stale client connections should fail with a clear error such as:

``` rust
enum IpcError {
    EndpointClosed,
    ServiceRestarted,
    PermissionDenied,
    QueueFull,
    DeadlineExceeded,
    Cancelled,
    InvalidMessage,
    ResourceExhausted,
}
```

A restarted service must not silently inherit outstanding reply tokens
or DMA ownership from the previous instance.

------------------------------------------------------------------------

## 14. Scheduling and priority propagation

A synchronous-looking service chain can suffer priority inversion:

``` text
high-priority application
    calls filesystem
        calls block service
            calls driver
```

The initial implementation may use ordinary priority inheritance on
endpoint waiters.

The ABI should leave room for:

-   deadline propagation;
-   priority inheritance;
-   scheduling-context or budget donation;
-   cancellation propagation;
-   admission control.

Do not encode these policies directly into sitas task priority. They
cross protection and trust boundaries and require kernel mediation.

------------------------------------------------------------------------

## 15. What to retain from the current fork

The following work aligns with the combined architecture and should
continue.

### Retain and harden

-   AArch64 EL0/SVC path;
-   caller identity derived from the current trap/thread context;
-   per-address-space capability tables;
-   shared CQ ring;
-   non-lossy kernel CQ backlog;
-   `CQ_WAIT` blocking through scheduler/observer machinery;
-   bounded queues and explicit `WouldBlock`;
-   no-std sitas split and CharlotteOS backend;
-   per-shard executor model;
-   typed in-kernel `ShardMailbox<M>`;
-   closure-based `ShardLocal<T>` with restricted use;
-   segment-aware ELF loading as an experimental base;
-   QEMU boot CI across AArch64 and x86-64.

### Reframe

-   completion capability work as the **operation completion
    subsystem**, not the universal service IPC subsystem;
-   observer/waker code as internal scheduling machinery, not the public
    semantic identity of all async objects;
-   IPI queue closures as a temporary kernel work mechanism, not the
    long-term cross-domain messaging model;
-   the current mailbox syscall ABI as a smoke-test interface only.

------------------------------------------------------------------------

## 16. What to rethink or replace

This section is the explicit redirection plan.

## 16.1 Stop expanding raw mailbox syscalls

### Current direction

The fork has incremental sender/receiver mailbox capability syscalls,
built from a typed kernel mailbox and earlier raw-LP smoke calls.

### Problem

A mailbox tied directly to LP identity is not a complete userspace
service abstraction:

-   it exposes placement where authority should be exposed;
-   it lacks protocol identity;
-   it conflates shard transport with protection-domain IPC;
-   it does not define scalar versus memory messages;
-   it lacks move/lend semantics;
-   it risks making service topology part of the ABI.

### Redirect

Implement endpoint and connection capabilities independent of LP
identity.

The endpoint owner may choose which shard receives a message. Clients
address the service connection, not a target LP.

## 16.2 Do not allocate one completion capability for every service request

### Current direction

The async-syscall model tends toward `submit -> CompletionCap`.

### Problem

For userspace service calls, the natural object is a reply token
associated with an endpoint call. For high-rate kernel operations,
per-operation capabilities may create unnecessary table pressure.

### Redirect

Use:

-   reply tokens for endpoint calls;
-   operation IDs plus CQ entries for ordinary kernel operations;
-   first-class operation capabilities only where independent authority
    is required.

## 16.3 Separate readiness, notification, and completion

### Current direction

Earlier prose treats waitable handles, observers, wakers, and
completions as essentially the same object.

### Problem

Their contracts differ:

-   readiness means progress may be possible;
-   notification means inspect state;
-   completion means an operation changed state and has result data;
-   waker means poll a future again;
-   capability means authority.

### Redirect

Keep one aggregated wait mechanism, but define distinct object semantics
and event records.

## 16.4 Replace arbitrary cross-LP closures

### Current direction

Kernel IPI RPC can carry boxed `FnOnce()` work.

### Problem

-   hard to account;
-   difficult to inspect;
-   brittle under backpressure;
-   obscures authority and lifetime;
-   unsuitable as a durable ABI model;
-   unsafe recovery/downcast paths have already caused concern.

### Redirect

Use closed kernel enums for mandatory kernel RPCs and typed mailbox
instances for subsystem-specific work.

For generic deferred kernel work, use an allocated work object with a
stable vtable/lifecycle owned by one subsystem, not an unrestricted
closure transported through the IPI layer.

## 16.5 Do not equate shard wake with IPI delivery

### Current direction

Remote wake maps directly to `send_ipi(target_lp)`.

### Problem

This can generate excessive interrupts and embeds mechanism in the
abstraction.

### Redirect

Queue first, mark pending, and send a coalesced reschedule IPI only when
the target cannot otherwise observe the transition promptly.

## 16.6 Introduce memory objects before rich IPC

### Current direction

The completion ABI and mailbox work are ahead of a general cross-process
memory-transfer model.

### Problem

Xous's most important contribution---move/lend semantics---cannot be
implemented safely with transient pointers or an ad hoc global result
page.

### Redirect

Prioritize:

1.  memory-object capability;
2.  mapping and unmapping;
3.  move between domains;
4.  temporary read lending;
5.  temporary exclusive mutable lending;
6.  revocation on reply/cancel/death;
7.  optional DMA pin/map integration.

Only then promote rich IPC payloads.

## 16.7 Replace global smoke-test ABI surfaces

### Current direction

Some experimental paths rely on fixed/global result pages, embedded test
images, and smoke-specific launch metadata.

### Redirect

Build a general process loader and startup contract:

-   ELF `PT_LOAD` mapping;
-   stack and guard pages;
-   declared heap or allocator bootstrap;
-   initial capability vector;
-   argument/environment block;
-   bootstrap endpoint;
-   no caller-supplied ASID;
-   no global shared result page.

## 16.8 Treat service discovery as userspace policy

Do not add string-based global service lookup to the kernel.

Build a userspace name/policy service that receives bootstrap authority
from the service manager and delegates connection capabilities.

------------------------------------------------------------------------

## 17. Proposed implementation sequence

The implementation order should change from "complete async syscall ABI,
then generalize mailboxes" to the following.

## Phase 0 --- Preserve the working baseline

Before architectural changes:

-   keep the current booting `dev` branch green;
-   retain AArch64 and x86-64 CI;
-   document all current syscall numbers and smoke-only interfaces;
-   mark experimental ABI modules explicitly;
-   add tests that distinguish completion, notification, and mailbox
    behavior.

## Phase 1 --- Capability and object model cleanup

Deliverables:

-   unified `CapabilityId` representation;
-   rights masks;
-   generation-safe lookup;
-   per-protection-domain capability table;
-   object teardown hooks;
-   transfer/delegation rules;
-   explicit object types.

Required object types:

``` text
Endpoint
Connection
ReplyToken
CompletionQueue
MemoryObject
Interrupt
MmioRegion
DmaDomain
Device
Timer
```

Decision gate:

> Can stale, forged, cross-domain, and wrong-type capabilities be
> rejected uniformly?

## Phase 2 --- Endpoint IPC v1: scalar messages

Implement:

-   endpoint creation;
-   connection delegation;
-   bounded receive queue;
-   scalar send;
-   scalar call;
-   reply token;
-   deferred reply;
-   close and domain-death cleanup;
-   endpoint wait/readiness integration;
-   explicit queue-full behavior.

Do not yet add rich memory messages.

Reference test:

``` text
EL0 client
    → scalar call
EL0 echo service
    → deferred reply from sitas task
client future completes
```

## Phase 3 --- Userspace name and service manager

Implement:

-   service instance identity;
-   name registration;
-   policy-controlled lookup;
-   connection-capability delegation;
-   generation changes on restart;
-   stale connection errors;
-   bootstrap capability delivery.

Reference services:

-   log service;
-   echo service;
-   timer facade.

## Phase 4 --- Memory objects (foundation for all rich IPC)

Implement:

-   allocation;
-   mapping;
-   unmapping;
-   protection changes;
-   ownership accounting;
-   capability delegation;
-   move between domains;
-   teardown on process death.

Reference test:

``` text
client allocates memory object
    → moves to service
service modifies and moves it back
client cannot access while ownership is absent
```

## Phase 5 --- Xous-style memory IPC

Implement:

-   `Copy`;
-   `Move`;
-   `BorrowRead`;
-   `BorrowWrite`;
-   reply-bound revocation;
-   cancel and server-death recovery;
-   mapping alias checks;
-   typed userspace wrappers.

Reference tests must include malicious/unsafe-style attempts:

-   sender accesses moved object;
-   receiver writes read-only lend;
-   sender accesses mutable lend while outstanding;
-   service dies while holding lend;
-   caller cancels deferred call;
-   reply token used twice.

## Phase 6 --- CQ subsystem normalization

Refactor current completion code around:

-   per-shard CQ;
-   operation IDs;
-   optional operation capabilities;
-   non-lossy backlog;
-   atomic wait;
-   cancellation state machine;
-   buffer return;
-   CQ batching.

Migrate `sitas-charlotte` to `CQ_WAIT` and remove busy polling.

## Phase 7 --- Sitas endpoint backend

Add a CharlotteOS userspace runtime layer with two event sources unified
at the executor boundary:

-   CQ completions;
-   endpoint receive readiness/messages.

The kernel may expose a wait set or allow both to signal one shard event
object.

The executor should:

1.  drain CQ;
2.  drain ready endpoints;
3.  wake tasks;
4.  run ready tasks within budget;
5.  wait again.

## Phase 8 --- Userspace UART driver

Use UART as the first complete userspace driver:

-   driver protection domain;
-   MMIO capability;
-   interrupt capability;
-   console endpoint;
-   scalar configuration messages;
-   moved/lent bulk buffer writes;
-   deferred read replies;
-   restart and reset behavior.

This validates authority, IRQ delivery, endpoint IPC, memory messages,
and service supervision without networking complexity.

## Phase 9 --- Virtio-net driver and network service

Architecture:

``` text
test application
    → socket/network endpoint
network service with sitas shards
    → packet/control endpoint
virtio-net driver
    → MMIO/virtqueue/IRQ
```

Start with copied packets, then move to registered pools/shared rings.

Milestones:

1.  scalar control path;
2.  copied packet path;
3.  moved packet buffers;
4.  buffer pool;
5.  batched CQ;
6.  per-queue shard placement;
7.  UDP;
8.  TCP after ownership and backpressure are stable.

## Phase 10 --- Kernel concurrency cleanup

After userspace architecture is proven:

-   replace generic IPI closures;
-   introduce reserved/priority kernel RPC capacity;
-   audit `ShardLocal<T>` use;
-   ensure no interrupt path accesses shard-local-only state;
-   add wake coalescing;
-   add LP migration/hotplug assumptions explicitly.

------------------------------------------------------------------------

## 18. Codex implementation rules

Codex should follow these rules while modifying either repository.

### 18.1 Preserve semantic boundaries

Do not:

-   use completion capabilities as endpoint connections;
-   expose LP IDs as service addresses;
-   pass arbitrary Rust types across the syscall ABI;
-   use raw userspace pointers as durable kernel references;
-   inject userspace callbacks from interrupt context;
-   convert every immediate syscall into an asynchronous operation;
-   silently drop completion records;
-   use unbounded queues.

### 18.2 Prefer explicit state machines

For IPC calls, operations, cancellation, endpoint closure, and memory
lending, implement named states and test transitions.

Avoid behavior that depends on scattered booleans.

### 18.3 Keep unsafe code narrow

Unsafe code should be localized to:

-   trap-frame access;
-   page-table manipulation;
-   userspace memory copy/validation;
-   MMIO;
-   DMA mapping;
-   carefully audited queue/ring primitives.

Protocol decoding, capability checks, and state transitions should
remain safe Rust where possible.

### 18.4 Add adversarial tests

Every capability and memory-transfer feature requires negative tests:

-   wrong domain;
-   stale generation;
-   wrong object type;
-   insufficient rights;
-   double reply;
-   double close;
-   queue full;
-   server death;
-   caller death;
-   cancellation race;
-   malformed message;
-   overlapping memory range;
-   writable alias during mutable lend.

### 18.5 Keep spike APIs visibly experimental

Until stabilized:

-   place experimental syscalls behind an explicit module/feature;
-   document that syscall numbers are unstable;
-   avoid building user libraries around smoke-only raw calls;
-   prefer typed wrappers so ABI changes remain localized.

### 18.6 Update this document with evidence

For each phase, append:

-   commit;
-   branch;
-   tests;
-   architectural findings;
-   rejected alternatives;
-   unresolved risks;
-   decision whether the experiment becomes durable architecture.

------------------------------------------------------------------------

## 19. Suggested repository-level changes

## 19.1 CharlotteOS

Suggested modules:

``` text
crates/catten/src/capability/
    table.rs
    rights.rs
    object.rs

crates/catten/src/ipc/
    endpoint.rs
    connection.rs
    message.rs
    reply.rs
    wait.rs

crates/catten/src/memory/object.rs
crates/catten/src/memory/transfer.rs
crates/catten/src/completion/
    queue.rs
    operation.rs
    cancel.rs

crates/catten/src/device/
    interrupt_object.rs
    mmio_object.rs
    dma_domain.rs
    device_cap.rs

crates/catten/src/service/
    bootstrap.rs
    supervisor.rs
```

Keep architecture-specific interrupt and trap handling under the
existing ISA modules.

## 19.2 Sitas

Suggested separation:

``` text
sitas-core
    executor
    futures
    shard-local
    typed internal mailboxes
    cancellation
    snapshots

sitas-unix
    readiness backend
    io_uring backend
    thread/affinity backend

sitas-charlotte
    CQ backend
    endpoint backend
    syscall wrappers
    memory-object wrappers
    protocol bindings
```

The CharlotteOS backend should not emulate Unix `RawFd` readiness. It
should implement native CQ and endpoint event handling.

## 19.3 Protocol crates

Create small versioned protocol crates:

``` text
charlotte-protocol-log
charlotte-protocol-console
charlotte-protocol-device-manager
charlotte-protocol-network
```

Each crate defines:

-   interface ID;
-   version;
-   opcodes;
-   request/response types;
-   encoding;
-   validation;
-   client wrapper;
-   server decoding helpers.

------------------------------------------------------------------------

## 20. Decision record

The combined architecture adopts the following decisions.

### Adopt

-   isolated userspace services and drivers;
-   capability-bearing endpoint connections;
-   userspace service discovery;
-   scalar and memory message distinction;
-   move/read-lend/write-lend semantics;
-   deferred replies;
-   sitas shard-per-core execution inside service processes;
-   completion queues for kernel/device operations;
-   non-lossy terminal completion delivery;
-   bounded queues and explicit backpressure;
-   no arbitrary userspace upcalls;
-   control-plane IPC plus ring/buffer-based data planes.

### Reject

-   one universal completion capability for all communication;
-   endpoint identity based on LP;
-   arbitrary typed Rust messages as a kernel ABI;
-   one IPI per message;
-   arbitrary cross-LP closures as a durable kernel work model;
-   string names as kernel-granted authority;
-   synchronous-only service loops;
-   unbounded completion or mailbox queues;
-   global smoke-test result pages as a process ABI.

### Defer

-   scheduling-context donation;
-   full priority/deadline propagation;
-   zero-copy shared data planes beyond registered memory objects;
-   live service upgrade;
-   distributed capability revocation;
-   formal verification of IPC state machines;
-   production SMMU/IOMMU support on every platform.

------------------------------------------------------------------------

## 21. Success criteria

The redirection is successful when all of the following are true:

1.  Two EL0 protection domains can communicate through a bounded
    endpoint without knowing each other's ASID or LP.
2.  A client can make a deferred asynchronous call while its shard
    continues running other tasks.
3.  A service can receive a moved memory object, and the sender is
    unable to access it until it is returned or retransferred.
4.  Immutable and mutable lending are enforced by mappings, including
    service-death cleanup.
5.  A sitas shard blocks on one native wait path and wakes for both
    endpoint work and kernel/device completions.
6.  Terminal CQ results survive userspace ring overflow.
7.  Remote shard wakes are coalesced rather than mapped one-for-one to
    IPIs.
8.  A userspace UART driver runs with only delegated MMIO and IRQ
    authority.
9.  Restarting that driver invalidates stale connections and reconciles
    outstanding operations.
10. A virtio-net data path can batch packet buffers without one syscall
    or copied IPC payload per descriptor.
11. All public ABI structures use fixed-width stable representations.
12. Capability misuse and cancellation races have adversarial tests.

Current status:

-   Criterion 1 has smoke-test evidence in `a97d9e4`: two separate EL0
    protection domains communicate through a bounded endpoint using
    delegated local caps only. Phase 3 strengthens this into a
    service-manager flow: the `el0_service` boot test exercises
    bootstrap capability delivery, userspace service naming, restart
    generations, and deterministic stale-connection failure across
    three isolated EL0 domains and a supervised restart.
-   Criterion 5's kernel half has self-test evidence: one blocking CQ
    wait is released by kernel completions, explicit cross-thread
    wakes, and CQ-bound endpoint readiness (`test_cq_wait_wake`). At
    EL0, the reference echo service serves its entire protocol —
    including shutdown and restart — from one `CQ_WAIT` with endpoint
    readiness bound to its default queue. The remaining gap to the full
    criterion is the sitas executor integration (task wakeup from the
    drained events).
-   Criterion 9 is met: its connection-invalidation half has smoke-test
    evidence for a generic service (echo) — restarting the service domain
    invalidates stale connections (`EndpointClosed`) and the userspace
    name service reports a new instance generation — and its
    driver-specific half is demonstrated by `test_el0_uart`: an
    uncooperative driver crash is followed by teardown that resets the
    device (unroutes the interrupt, unmaps the MMIO), reconciles the
    outstanding deferred read (`Cancelled` rather than hung), and fails
    stale connections `EndpointClosed`, after which a restarted
    generation-2 instance serves with freshly minted device grants.
-   Criterion 8 is met. Its kernel half has self-test evidence
    (`test_device_capabilities`): a driver domain can be granted an MMIO
    region and an interrupt source as capabilities, map the region into
    its own address space as user Device memory, and be released from one
    `wait_on_cq` by a real GICv3 software-pended SPI routed through the
    live interrupt path. The EL0 half is demonstrated by
    `test_el0_uart`: the reference `uart` userspace driver runs in an
    isolated EL0 domain with only a delegated PL011 MMIO region and its
    interrupt (plus a name-service bootstrap connection), maps the
    register window, serves a console client's writes through the PL011
    registers with direct EL0 MMIO writes, completes an interrupt-driven
    deferred read (§7.2), and acknowledges a delegated device interrupt
    from EL0.

------------------------------------------------------------------------

## 22. References

Primary project references:

-   CharlotteOS fork: <https://github.com/FrodeRanders/charlotte-os>
-   Upstream CharlotteOS: <https://github.com/charlotte-os/charlotte-os>
-   Sitas: <https://github.com/FrodeRanders/sitas>
-   Xous core: <https://github.com/betrusted-io/xous-core>
-   Xous kernel documentation: <https://xous.dev/kernel/>
-   Xous server/API guide:
    <https://github.com/betrusted-io/xous-core/blob/dev/API.md>

Relevant Xous concepts to inspect in source:

-   `SID` and `CID`;
-   server creation and connection;
-   `Scalar` and blocking scalar messages;
-   memory `Send`, `Lend`, and mutable lend messages;
-   name service;
-   message receive and reply;
-   process and server teardown.

------------------------------------------------------------------------

## 23. Working summary for Codex

Implement the architecture in this order:

``` text
capability model
    → scalar endpoint IPC
    → userspace service/name management
    → memory objects
    → move/lend IPC
    → normalize completion queues
    → sitas endpoint/CQ backend
    → UART userspace driver
    → virtio-net driver and network service
    → kernel concurrency cleanup
```

Preserve the current completion, AArch64, CQ, and sitas work, but
reinterpret it as one half of the design.

The missing half is Xous-inspired protection-domain IPC.

The final model is built upon three kernel primitives---Capabilities,
Endpoints, and Memory Objects---and the remaining subsystems are layered
on top of them.

The final model is:

``` text
Xous ideas:
    who may talk to whom
    how server IPC works
    how memory crosses protection boundaries

CharlotteOS:
    how authority and isolation are enforced
    how hardware and operations complete
    how threads wait and wake

Sitas:
    how each service uses its CPUs
    how mutable state remains owned
    how asynchronous tasks compose internally
```

That division of responsibility should guide every subsequent change.
