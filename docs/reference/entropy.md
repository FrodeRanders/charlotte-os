# Cryptographic entropy

CharlotteOS exposes two fallible cryptographic entropy paths:

- the `RandomU64` syscall uses Arm `RNDR` or x86-64 `RDRAND` when the
  architectural instruction is available; and
- the node-local `rng` service uses a modern VirtIO RNG PCI device.

Security-sensitive callers fail closed when neither path can satisfy a
request. There is no clock-, counter-, or deterministic-seed fallback.

## VirtIO RNG service

The steady-state launcher discovers PCI device `1af4:1044` (or its
transitional `1af4:1005` identity), creates a private protected-DMA domain for
that requester, and launches `rng.elf`. The driver receives only its four-page
modern VirtIO MMIO window and DMA-domain capability. It polls the queue and is
therefore not granted an interrupt capability.

### Publication and request readiness

Before publication the owned request endpoint binds to the launch-owned default
CQ. Registration uses the shared `publication::register`: only unaccepted exact
queue-full rejection permits bounded admission backoff; accepted work is
observed once. Its existing central compile-time policy allows 1000 ms for
admission and 5000 ms for accepted-call observation, with 10 ms admission
backoff. These are separate phase budgets, not one five-second end-to-end
startup guarantee. Permanent/timeout/malformed publication and CQ binding or
wait-admission rejection abort with the initialized device and endpoint live;
they do not replay a registration or claim device quiescence.

The ordinary idle service checks lifecycle, drains CQ hints and tries to
receive from its endpoint. An empty receive waits event-only on that same CQ,
whose generation/recheck contract covers enqueue races. Endpoint enqueue
and exact-root lifecycle request publication both signal it. Every wake
rechecks actual sources; CQ readiness neither receives a request nor adopts
returned capabilities. There is no 10 ms idle timer. Unrelated wakeups repark
without a watchdog, and accepted requests still use the owning transaction
below. Lifecycle wakeup is not proof of hardware teardown; that separate
existing shutdown path is unchanged.

The shared publication constants live in `publication.rs`; changing them affects
all its service callers. Rebuild/requalify boot ordering, queue-full/silent-peer
publication and owner retention against loader/readiness budgets together.
There is no new boot-resource input, live configuration or independent quota.
See the [configuration guide](../guides/configuration.md).

The service registers the short name `rng`, interface version 2. `OP_FILL = 2`
requires a writable memory object moved from the caller, exactly
`entropy::BUFFER_BYTES` bytes, and a requested count from 1 through
`entropy::MAX_REQUEST`. Both currently equal 4096 bytes. Success returns that
same buffer and the exact initialized count, with no connection attachment.
Only the requested prefix is written. Retired opcode 1, missing/oversized
buffers, invalid counts and connection attachments reject before device work;
there is no service-allocating fallback.

The shared `entropy_service::serve_fill` transaction retains the complete
incoming request, reply/loan authority and transient writable mapping. Local
access and received memory end before an error reply or loan cancellation.
Mapping/unmapping failure returns `ERR_MEMORY`, device failure `ERR_DEVICE`;
ordinary rejected requests return no memory. The client must move a fresh
buffer for another independent operation, never replay accepted uncertainty.
The driver handles short device completions by resubmitting until the complete
request has been filled; its fixed private DMA buffer remains separate from
the caller's CPU-only reply backing.

The kernel keeps the original allocating generation's charge through moves,
unobserved results, return, retention and sponsor retirement. Retaining RNG
replies therefore consumes caller backing rather than fresh producer backing.
This does not eliminate IPC metadata admission, queue contention, producer
mapping costs or denial of service from authorized flooding.

Virtqueue memory uses `catten_rt::owned::SharedDmaMemory`. This owner keeps the
memory object, CPU mapping, and DMA mapping together; access is volatile, and
drop order removes device access before unmapping CPU access and closing the
memory capability. Setup and teardown errors retain the precise mapped or
unmapped owner so callers cannot accidentally adopt a handle twice.

## QEMU

The ordinary AArch64 runner attaches the device even when no test verifier is
selected:

```text
-object rng-random,filename=/dev/urandom,id=charlotte-rng
-device virtio-rng-pci,rng=charlotte-rng,disable-legacy=on,iommu_platform=on,addr=4
```

`iommu_platform=on` is required: CharlotteOS negotiates
`VIRTIO_F_ACCESS_PLATFORM` and maps the queue only through the device's SMMU
domain. This supplies entropy to QEMU's named `cortex-a710` model without
pretending that the CPU implements `RNDR`.

The S3 TLS service prefers architectural random words and transparently uses
the `rng` service for any remainder. Other applications should resolve `rng`
through the name service and retain the returned owned connection rather than
accessing VirtIO or DMA directly.

## Client policy and related dimensions

The shared owned client allocates its buffer before admission, within its
original deadline; allocation failure is `EntropyError::Preparation`, distinct
from submission/observation failure. It consumes the buffer with `call_move`,
retains returned memory/mapping in its completion owner and checks the exact
buffer size as well as count/attachments. Explicit cancel and Drop preserve
the existing checked cancellation policy. See [TLS ownership](tls-client.md).
S3, Kafka and TCP/IP use that client. DNS boot-nonce collection also uses it,
carrying one central `CAPACITY_NONCE_TIMEOUT_MS = 100` budget through immediate
name lookup and fill, replacing its reply poll/sleep loop. Collection can still
pause the DNS reactor for that bounded observation; this is not a general
asynchronous capacity-report reactor migration or a latency guarantee for
kernel cleanup. Independent failed collections retain the existing report
interval and fail-closed no-nonce behavior.

Protocol storage dimensions are grouped in `catten_services::entropy`:
`BUFFER_PAGES = 1`, `BUFFER_BYTES = BUFFER_PAGES * 4096` and
`MAX_REQUEST = BUFFER_BYTES`. The page size is the memory-object ABI's structural
unit. These are compile-time settings; no launch/boot resource or live override
is added. Increasing them enlarges every service request's caller allocation
and the driver's persistent private DMA buffer. Descriptor lengths must fit
`u32`; the compile-time check enforces that and a nonzero buffer. Rebuild all
consumers and the driver, then repeat minimum/maximum, rejection, queue,
retention, cancellation, budget and DMA qualification. The separate positive
`EntropyLimits` deadline remains installed with its source. See the
[configuration guide](../guides/configuration.md).

The [caller-buffer follow-up](../reports/audits/2026-10-26-security-rng-caller-buffers.md)
records concrete caller/producer accounting and verification limits. It does
not establish hardware quiescence or close the broader SEC-07 finding.

The [server-readiness follow-up](../reports/audits/2026-10-26-security-rng-readiness.md)
qualifies actual idle CQ registration, endpoint wakeups and repark after
unrelated/lifecycle-publication hints, including a failing old-server control.
This removes request polling, not the bounded VirtIO descriptor/status poll.
That hardware path has no implemented completion-interrupt notification. A
device completion timeout can still leave a descriptor's outcome uncertain;
the [descriptor-owner correction](../reports/audits/2026-10-26-security-rng-descriptor-fence.md)
now fences subsequent reuse. Hardware recovery remains separate work. An idle
CQ wake supplies no permission to replay device work.

### Descriptor ownership and terminal failure

`rng_device::Device` owns the complete hardware adapter: MMIO, queue/bounce
CPU and DMA mappings, their original capabilities and charges, and captured
descriptor context. It arms terminal retention before the first descriptor
mutation. A successful step requires the exact next wrapping used index,
descriptor ID zero and a nonzero length no greater than requested. Only then
can its bytes be copied and the completed position advance. Short successful
completions continue the remaining prefix; uncertainty never resubmits one.

Preparation/publication/read failure, timeout, unexpected index/descriptor,
invalid length and failed copy all fence that same owner. Its captured attempt
retains previous/expected indices, slot, requested length and prior copied
prefix. A checked completion remains recorded separately from later copy
failure or interruption. Later fills return the original typed failure without
any backend call, even if a late completion has appeared. Invalid caller lengths
reject before mutation and do not fence an otherwise healthy owner.

The retained fallback invokes no backend destructor, reset, unmap, close,
deallocation or accounting refund. This preserves every nested hardware field;
it is terminal retention, not a recovery ticket. Valid later IPC requests get
`ERR_DEVICE`; the first returned failure records diagnostic status `0xe008` and its
stage/attempt, without exposing entropy. Caller memory remains CPU-only and
the existing reply transaction never publishes a partial successful count.

On shutdown, a fenced `ServingRng` retains device, endpoint and original shutdown
request together in an event-only terminal park. It stops receiving requests
and withholds device-quiesced acknowledgement rather than running the ordinary
reset/Drop path. The existing node device coordinator retains a driver that
does not provide that acknowledgement; no force-success or power-off is added.
This is not a qualified device reset/recovery implementation. Healthy shutdown,
partial setup, actual hardware fault injection and forced termination retain
their separate proof requirements.

Hardware policy is centralized in `rng_device.rs`. `Limits::DEFAULT` allows
`1000000` completion-index polls per descriptor; `Limits::new(n)` accepts only
a positive bound before backend adoption. The bundled driver passes that
central default. This is an iteration ceiling, not milliseconds, a total
request deadline or permission to retry. Each confirmed short completion can
require a further bounded step. `QUEUE_SIZE = 8` is a compile-time power-of-two
split-ring dimension; page-rounded ring backing derives from that geometry
(currently two pages), while entropy bounce storage follows the protocol buffer.
Larger queues do not add concurrent outstanding descriptors. Changing these
inputs requires rebuilding and requalifying device capacity/layout, CPU cost,
short completions, rejection, timeout/late completion, retention and caller
observation budgets together. No live/manifest/boot-resource override is added.

The adapter still uses the existing bytewise volatile helpers for scalar ring
fields. Real-device atomicity/cache-ordering qualification and hardware index
rollover are separate work; the host wrap control checks software sequence
arithmetic only. Descriptor fencing is not a complete hardware-protocol proof.

## TCP/IP startup

TCP/IP initializes its smoltcp interface only after obtaining 40 fresh entropy
bytes through the architectural source or the owned `rng` service connection.
Failure withholds network startup; no public constant or clock supplies a
fallback. Eight bytes seed smoltcp's existing protocol RNG. The remaining 32
bytes independently key HMAC-SHA-256 counter-based ephemeral-port selection in
49152–65535. Port selection is not a public ascending sequence, and disclosure
of smoltcp's seed does not reveal the port key. Keys are wiped when dropped.

This hardens startup random state; it does not replace smoltcp's internal RNG
with a cryptographic generator or authenticate network peers.
