# AArch64 NVMe object-client completion timeout

Date: 2026-09-13

## Incident

An AArch64 broker soak stopped during boot with an explicit self-test panic:

```text
panicked at crates/catten/src/self_test/el0_nvme.rs:313:5:
[nvme] object-client completion deadline expired
```

The raw return addresses resolve to `rust_begin_unwind`, `assert_failed`,
`catten::self_test::el0_nvme::verify_el0_nvme`, and
`kernel_thread_trampoline`. This is not another exception-return translation
fault: the verifier deliberately panicked after its 60-second deadline.

The complete failing serial capture and its matching symbol sidecar were not
available after the incident. The conclusions below therefore distinguish
what the supplied excerpt proves from what the next capture must establish.

## What the repeated deployment message means

The node agent used to print this line immediately before every invocation of
its artifact retrieval function:

```text
[agent] fetching "broker" from S3 key "deployments/broker.elf"
```

It is one complete S3 GET attempt, not one object-data chunk. A successful GET
streams multiple protocol chunks internally and then prints one
`fetched and verified N bytes` line. In the incident, the absence of that
success line and repetition at roughly 600 ms intervals show that complete
launch attempts were failing and the 500 ms reconciliation loop was trying
again.

Repeated full GETs can also be legitimate across desired-state generations.
In a validation boot, the agent first restored the persisted broker generation,
retired it when the fixture committed a replacement, and fetched the new
generation. Those attempts were separated by successful verification and
launch messages and are normal reconciliation, not retry churn.

## Changes made

The agent now:

- calls the operation a `full S3 GET` in its log;
- logs whether failure occurred while locating the connector, beginning or
  reading the GET, validating response metadata, mapping a chunk, closing the
  operation, checking length, or checking the digest;
- keeps retry state per deployment or operational-profile generation;
- applies exponential retry delay from 1 second through a 30-second cap;
- discards retry delay immediately when a replacement generation appears; and
- removes retry records when the corresponding desired state disappears.

The NVMe verifier now prints, before panicking:

- object-client ASID, TID, generation, stage, byte counters, and thread state;
- object-store stage, error, block operation/result, reply status, detail, and
  thread state;
- NVMe stage, interrupt count, last command, outstanding-command state, and
  thread state; and
- name-service stage, handled-message count, last opcode, waiter count, and
  thread state.

The object-client stage has direct diagnostic meaning: stages 1 through 6 are
create, resize, write, flush, read, and payload verification. Values in the
`0xdea*` range mean that the client detected an error and exited; `0x900d`
means that its object work succeeded and it reached completion-name
publication.

## Reproduction results

Three AArch64 broker boots completed after the changes:

1. A fresh-disk boot passed all 20 self-tests, launched the broker once, and
   sustained 120 seconds of load with 1,097 produced, 1,096 consumed, no gaps,
   and no errors.
2. A boot reusing that disk passed storage recovery, restored the persisted
   assignment, and sustained 60 seconds with 551 produced, 549 consumed, no
   gaps, and no errors.
3. A further persistent-disk boot built with the final timeout diagnostics
   passed all self-tests and a short broker probe.

Preserved captures are in `/private/tmp` under:

- `charlotte-nvme-investigation-first-boot-*`
- `charlotte-nvme-investigation-persistent-boot-*`
- `charlotte-nvme-investigation-final-diagnostics-*`

The timeout did not reproduce. These runs show that persistent desired state
and overlap with network/S3 startup do not deterministically cause the storage
failure.

## Follow-up: deliberate persistent assignment without its fixture

The broker soak committed a broker assignment to the durable NVMe-backed
catalog. A subsequent `scripts/run-aarch64.sh --clean` boot deliberately
preserved that image while omitting the ephemeral RustFS/S3 fixture. The valid
broker generation therefore remained desired while its separately provisioned
S3 connector was absent.

With bounded retry, the agent now reports the missing connector and backs off
to 30 seconds without destabilizing the node. This is the correct production
failure mode: loss of external infrastructure must not implicitly delete
committed desired state. It also forms a useful recovery scenario: after an
operator restores the machine-provisioned S3 connector, the unchanged broker
assignment should become runnable without rebooting the node or recommitting
the deployment.

The observed run reached failures 7 and 8 at the 30-second cap. Between those
attempts, the network, frame router, TCP/IP service, time service, and
observability archiver continued reporting normally, including an archive
rotation. No kernel panic followed. This validates bounded degraded operation;
it does not yet validate restoration. In this particular fixture, restarting
RustFS alone is insufficient because the `s3` connector itself was omitted
from the boot, and the ephemeral RustFS volume no longer contains the signed
broker object. Both the machine-provisioned connector and the exact
digest-matching object must return before the durable assignment can converge.

## Next decisive evidence

On recurrence, preserve the serial log, `.symbols`, and `.kernel.sha256`
sidecars before another run. The new timeout lines distinguish four materially
different defects:

- a `0xdea*` object-client stage: an application-visible object-store failure;
- stage 1--5 with an outstanding NVMe command: lost or unprocessed block-I/O
  completion;
- stage 1--5 without outstanding device work: IPC scheduling or object-store
  progress failure; or
- stage `0x900d` with a remaining name-service waiter: completion-name
  registration is blocked or the deferred lookup was not woken.

Until one of those states is captured, increasing the deadline or serializing
production startup behind completion of self-tests would hide the failure
rather than identify it. Production readiness should continue to depend on
the durable object store serving, not on optional self-test completion.
