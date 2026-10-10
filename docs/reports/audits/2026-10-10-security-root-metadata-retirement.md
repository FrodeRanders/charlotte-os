# Final-root mailbox and authority metadata retirement

This continues the [mailbox publication correction](2026-10-10-security-mailbox-publication.md)
within C02/C03/C18 and G1/G4/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md). SEC-07 and
SEC-18 remain partial: **20 corrected, six partial and four open** findings,
eighteen owner families and seven acceptance gates. The existing final-root
custody registry retains a larger complete owner; no new registry, ticket,
controller or generic metadata-retry interface is introduced.

## Defect and correction

Explicit mailbox close had qualified post-guard release, but final user-root
cleanup still drained mailbox payload and unified authority nodes under
`ADDRESS_SPACE_LIFECYCLE`. Legacy queue backing also dropped there. Leaving each
local registry did not qualify the outer lifecycle context. Failed final
invalidation consequently retained the root after its metadata charges had
already ended.

[`ClosingAddressSpace`](../../../crates/catten/src/memory/retirement.rs)
now owns final metadata detachment through the root handoff. After exact
device/IPC/memory completion, zero leases and cleanup sealing, existing final
scratch/completion/accounting work still runs under lifecycle. Then
[`RetiredMailboxes`](../../../crates/catten/src/syscall/mailbox_retirement.rs)
captures the admitted mailbox namespace and legacy queue payload, and
[`RetiredNamespace`](../../../crates/catten/src/capability.rs) captures the entire
unified authority node with its existing ordered records. Both check the exact
generation and retired sponsorship before extraction. No teardown snapshot is
allocated. The closing transaction holds every detached field before final
table extraction transfers them to `RetiredAddressSpace` with the original root
and software-slot lease.

Final invalidation rejection returns that complete owner with metadata and both
original record accounts still charged. Existing bounded custody, exhaustion,
claim and retry-limit rules apply to all fields. A diagnostic ticket conveys no
independent metadata authority.

After confirmed final invalidation, the same receipt explicitly drains mailbox
payload nodes and releases queue backing, then drains the unified authority
namespace. Both run after local lifecycle/mailbox/capability/recovery guards
leave, before the existing one-shot physical walk and slot completion. Original
classification and charges follow their actual metadata lifetime. Neither owner
reconstructs a namespace from a numeric ASID. Mailbox fallback retains all fields
behind `ManuallyDrop`; authority fallback retains the admitted node and its
charges. Root/closing fallback therefore keeps the whole transaction without
allocator/counter work or logging.

The scalar mailbox teardown adapter is now explicitly confined to serialized
boot fixtures. **Legacy queue BTreeMap node removal still deallocates under its
registry/lifecycle.** Only queue payload backing release is qualified here.
Completion, scratch/accounting metadata and other authority callers retain
separate context/admission work.

## Selected execution evidence

The [actual custody fixtures](../../../crates/catten/src/memory/retirement/recovery/tests.rs)
use a real root with one committed heap page, two mailbox grants, one observer
grant and queued legacy words:

- Final preparation removes payload/authority visibility but retains **two
  mailbox charges and three shared authority charges**. Original heap backing
  and free-frame state survive injected invalidation rejection. The actual
  bounded registry returns `AwaitingRetry` with the complete owner. Reading its
  queued fixture word afterward confirms queue content survived that rejection.
- A confirmed retry releases both record accounts before physical callbacks;
  the real heap/table walk completes and the slot can be reused. The successor
  gets a distinct generation with the same initial numeric grant sequence.
  Stale namespace extraction and terminal old-ticket claims preserve its two
  family and three shared charges. Ordinary successor close refunds both.
- Guarded **actual custody-attempt** abandonment holds the registry, lifecycle,
  both mailbox registries, CPU tables and physical/heap allocators. It retains
  **one additional complete detached root/slot, two ordinary mailbox charges,
  three ordinary shared authority charges, one heap page, the original CPU table
  backing and queue backing**. Free-frame state is unchanged; the record is
  `Abandoned`, future claims reject, and a new root cannot reuse that software
  slot. This is terminal retention, not an automatically recoverable owner.
- The existing physical-rejection fixture now also carries those grants and
  queue backing. Metadata finishes before the owning walk rejects one frame and
  releases later frames. Both metadata accounts remain zero while the original
  heap/table charges retain their prior terminal policy; custody is
  `Quarantined` and cannot retry. This extends the existing failed-root/frame
  episode rather than adding another physical quarantine fixture.

Existing invalidation-failure/abandoned roots now retain their already allocated
empty authority namespace nodes too. This extends metadata lifetime rather than
allocating replacement nodes. No extra image or IOMMU table/data frames are
introduced by these fixtures.

Selected phase diagnostics, identical across the three targets:

```text
[root metadata phases] 2 mailbox and 2 authority release entries outside local lifecycle/mailbox/queue/table/physical/heap/capability guards; entry IRQ state preserved
[root metadata phases] 1 mailbox and 1 authority release entries outside local lifecycle/mailbox/queue/table/physical/heap/capability guards; entry IRQ state preserved
```

These are two instrumented scopes, **three entries per metadata kind**, not a
global allocation/deallocation balance. Original IRQ state is preserved. Actual
registry availability is checked in retry invalidation and physical callbacks;
release-entry probes check the other listed local guards. Existing shared-node
fixtures qualify admitted detachment; legacy BTreeMap removal is excluded.
Serialized rejection/reentrancy does not establish real OOM, panic-unwind,
cross-LP pressure or physical-platform recovery.

## Validation

Fresh-storage guests run sequentially, reusing validated embedded bundles because
only kernel code changed:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance root-metadata-intel-20261010 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance root-metadata-amd-20261010 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18789 CATTEN_DEPLOY_HOST_PORT=18089 scripts/run-aarch64.sh --security-test --instance root-metadata-arm-20261010 --fresh-storage --timeout 180
```

| Guest | Final result | Instrumented mailbox / authority release entries |
| --- | --- | ---: |
| Intel VT-d | 15 passed, zero failed/pending; `0x1bbff` | 3 / 3 |
| AMD-Vi | 15 passed, zero failed/pending; `0x1bbff` | 3 / 3 |
| Arm SMMUv3 security guest | 19 passed, zero failed/pending; `0x2003ffff` | 3 / 3 |

Arm scoped checks also pass `0xffff` at publication generations 1 and 2.
Kernel SHA-256:

- Intel/AMD x86: `85c9cb0a836ce259028097b3051d7b7254183201a00ac4ab011a781f7d434cc2`.
- Arm security guest: `404b883596e0fe6f4999ac23ea5f681489cce69c5f9510a9e6821b923903f18a`.

All three guest runs completed on the first artifact. Injected barrier/physical
rejections are expected fixture outcomes, not failed guest runs. Strict locked
default-feature Clippy passes on both architectures (`-D warnings`), as do
formatting and diff checks. Runners verify native assembly section/load
permissions (249 x86 entries, one Arm entry). Shared admitted-node source/host
fixtures are unchanged; the preceding eight-test/full-host-harness evidence
remains in the [namespace report](2026-10-09-security-capability-namespace-storage.md).
That harness is not rerun in this kernel-only pass; actual root custody and
existing mailbox/quota/generation fixtures execute in all three guests.

## Remaining boundary

Legacy queue storage/admission, completion/scratch/accounting teardown, other
authority preparation/retirement and active token fallback, general heap-byte/
principal/progress budgets and every enclosing caller still need qualification.
Recovery diagnostics retain their existing heap/image/table counts; they do not
report retained queue bytes or family/authority metadata charges. This owner
extension does not add authenticated operator policy or reconcile cached
supervisor errors. Historical intermittent Intel/AMD user-stack lease timeouts
remain causally unresolved; successful guests do not close C16/G1/G2/G7. The
[current ledger](../../reference/security-remediation.md) preserves their separate
failed-run/repeat evidence and the original SEC-07/18 criteria.
