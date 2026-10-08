# Bounded final-root recovery receipts — 2026-10-08

This batch advances **SEC-18 / R18-3**. It preserves the reconciled audit totals:
**20 findings corrected in scope, six partial and four open**. SEC-18 and R18-3
remain partial; production trust admission remains disabled.

## Scope and implementation

Previously an explicit final-root invalidation rejection discarded its detached
owner into quarantine. The complete root was safe from reuse, but production had
no bounded custody/status mechanism through which a trusted controller could
retry the still-unstarted physical phase. Private recovery tests could retain
that owner directly, which did not provide production receipt custody.

The final-root release adapter now transfers its complete `RetiredAddressSpace`
into eight fixed inline registry slots after invalidation rejection. This owner
retains the original root, hardware tag, software slot and heap/image/table
accounts. Admission allocates nothing; exhaustion or serial overflow returns
ownership, then quarantine occurs outside the registry hold. Only recovered
history slots can be reused, with a fresh checked serial. No receipt can be
reconstructed from diagnostic ASIDs or physical addresses.

A trusted kernel retry hook claims the exact slot/serial, marks it running and
moves its owner out before releasing the registry guard. Masked interrupt state
rejects before claiming. Stale, competing, terminal and exhausted claims reject
before callbacks or physical work. Each receipt permits two explicit attempts;
each production x86 invalidation attempt retains its existing three fresh
epoch-fenced rendezvous attempts. Rejection returns the same complete owner.
No controller, registry, lifecycle or root-table guard is introduced around the
invalidation or physical walk.

`RetiredEntry::release_value_with` now reports resource-specific completion
before destroying its payload and returning the linear slot token. Both owning
architecture walks expose their rejected-frame count and retain their existing
one-shot disarming/account-quarantine rules. A failed physical walk is terminal,
including when a prefix already returned frames. As in the existing contract,
confirmed invalidation permits tag/software-slot completion while rejected
physical backing and its whole original charges remain quarantined. A physical
interruption cannot invoke a second destructor or return an unfinished slot.

Attempt Drop only marks its stable admitted cell atomically and quarantines its
root receipt. It neither locks nor logs nor performs physical cleanup. Completion
disarms that mark under serialization before publishing reusable history, so an
old attempt cannot mark a successor slot. Lost completion status is terminal even
when a controlled physical walk had already completed; it authorizes no replay.

Thread-statistics wire version 8 adds six current registry-state counts,
rejected admissions and capacity. Only a validated `SystemObserver` receives
these words; ordinary caller snapshots contain zeros. The existing `observe`
`OP_THREAD_SNAPSHOT` response forwards the owned memory containing these fields.
No telemetry right becomes a retry/reset right. HTTP/cluster history formats
remain unchanged. Both architecture service bundles were rebuilt and signed for
the updated ABI.

Contract: [bounded final-root recovery](../../reference/root-recovery.md).

## Tests and limits

Real-root guest probes cover:

- Failed invalidation preserves physical frames, original heap charges and the
  captured software slot. A competing claim sees busy; callbacks and physical
  release observe available registry/lifecycle/table guards.
- Fresh invalidation completes release and permits exact-generation ASID reuse.
  Reusing a completed registry slot assigns a new serial; the old ticket rejects.
- Eight occupied records reject a ninth and return the complete owner. Two
  failed attempts retain ownership but fence further retry. Serial exhaustion
  never wraps or publishes a record. Invalid slot indices reject.
- Attempt abandonment beneath the registry lock performs no nested lock or
  physical release. The abandoned slot is terminal. Dropping isolated fixture
  bookkeeping retains both the abandoned and exhausted roots' heap charges.
- A real physical walk rejects its first allocator call and releases the rest;
  original charges remain consumed, terminal quarantine rejects further claim,
  and a successor software slot cannot re-adopt that backing.
- Completion lost after a controlled successful physical walk is terminal and
  cannot replay the old addresses. This models the status-publication boundary;
  it is not a real kernel-panic injection inside the page-table walk.
- The production failure adapter registers a receipt, the kernel hook recovers
  it, and completed retry rejects. Serialized boot checks also reject masked
  production entry before claiming, using the private controlled adapter to
  complete fixture cleanup. Ordinary telemetry words are zero and privileged
  aggregate counts never exceed capacity.

The existing concurrent x86 probes now use production registry custody after
actual rejected IPI delivery and missing acknowledgement. The detached root and
its original heap charge remain retained, an intervening namespace cannot reuse
its slot, and a fresh registered retry completes the real rendezvous/destruction.
Intel and AMD run this path with four LPs. Arm exercises real root ownership and
physical cleanup with controlled final-invalidation rejection; it does not prove
an equivalent lost-ack hardware scenario.

Two new standalone host tests check that resource completion precedes destructor
and slot return, and that an actual caught panic in the generic completion
callback retains payload/slot without running its destructor. The complete
slot-owner suite now has **29 tests**. The newly removed no-result release wrapper
has no production or compatibility caller; existing tests use the consuming
completion primitive directly.

Fixtures intentionally retain two roots and one rejected physical frame, with
three heap charges in total, in addition to the base retirement suite's existing
quarantine cases. They add no recovery bypass. This is not full hostile workload,
power-loss, physical-platform or every-receipt recovery evidence.

## Validation

| Check | Result |
| --- | --- |
| AArch64 bundled services | `scripts/build-catten-services.sh --embed`: rebuilt and signed. |
| x86 bundled services | Normal `scripts/run-x86_64.sh` rebuilt and signed the missing staged bundle. |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Host regression suite | Passed, including **29 slot-owner tests**, signer/trust and remaining workspace checks. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**, registry and concurrent recovery markers. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**, registry and concurrent recovery markers. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**, registry marker, scoped probes `0xffff`, generations 1/2, 4,764 cancellation requests. |
| Formatting/links | `cargo fmt --all -- --check`, `git diff --check` and relative documentation links: passed. |

The first x86 Clippy invocation could not find the generated bootstrap bundle;
no code result was inferred from that failed invocation. The normal runner
rebuilt it. Final QEMU runs and Clippy checks used the complete signed bundles
and runners enforced assembly section permissions. The Arm runner required local
forwarding-port permission.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance root-registry-intel-final-20261008 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance root-registry-amd-final-20261008 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18523 CATTEN_DEPLOY_HOST_PORT=17823 \
scripts/run-aarch64.sh --security-test \
  --instance root-registry-arm-final-20261008 --fresh-storage --timeout 180
```

Final logs: `/private/tmp/charlotte-root-registry-{intel,amd,arm}-final.log`,
`/private/tmp/charlotte-root-registry-clippy-{x86,arm}-final.log`,
`/private/tmp/charlotte-root-registry-host-final.log` and
`/private/tmp/charlotte-root-registry-services-arm.log`.

## Remaining work

No automatic worker or authenticated operator retry endpoint is enabled. The
kernel hook is an explicit trusted-controller boundary; read-only observer
counts expose no root authority. A controller must still supply admission,
masking/lock context, deployment/restart policy and handling of cached supervisor
failure. Interrupt-state rejection does not prove every nonmasking lock absent.

Staged subsystem-cleanup failures, abandoned leases/roots, loan/mapping/scratch
claims, DMA/device receipts and kernel-stack reservations remain outside this
registry. No failure can be force-cleared, no abandoned count can be decremented,
and no partially returned physical address can be retried. Stronger authenticated
reset/reboot qualification, bounded broader owner custody, concurrent stress and
physical-platform evidence remain required by R18-3 through R18-6.
