# Mailbox publication and explicit close contexts

This extends C18/G4 of the [cleanup/recovery strategy](../../reference/cleanup-recovery.md)
using the existing admitted-map and shared authority machinery. SEC-07 and
SEC-18 remain partial: **20 corrected, six partial and four open** findings,
eighteen owner families and seven acceptance gates. No custody registry or retry
interface is introduced.

## Defect and correction

Mailbox opens allocated namespace budgets and BTreeMap payload storage beneath
`ADDRESS_SPACE_LIFECYCLE`/`USER_MAILBOX_CAPS`. Shared capability preparation also
allocated there. Authority became live before infallible payload insertion.
Explicit close destroyed both payload and authority nodes while the outer
mailbox registry remained held. Count admission did not qualify these storage
and destruction contexts.

[`PreparingMailbox`](../../../crates/catten/src/syscall/mailbox_publication.rs)
now owns the exact root operation, captured identity/classification, fallible
namespace/endpoint nodes, unused namespace budget, shared `PreparedReservation`,
staged reservation and family charge. Preparation precedes local lifecycle and
mailbox guards. Both payload maps use the existing `AdmittedMap`. Publication
revalidates generation, retirement and closing state, then reserves charges,
publishes borrowed authority and relinks prepared payload storage under the same
serialization. There is no fallible payload allocation after authority publication.
The containing preparation stores its identity; publication cannot substitute a
fresh numeric root. Existing per-LP receiver reuse needs neither new storage nor
spare admission. The obsolete lifecycle-only allocator helper is removed.

Ordinary completion/cancellation explicitly disposes unused storage and staged
charges after local guards leave, completing the exact root last. Preparation
Drop retains every field without locks, counter/allocator work or logging.

Explicit close acquires its exact root before registry access and detaches both
charged nodes into `RetiredMailbox`. Post-guard completion releases payload and
authority before completing the root. Abandonment retains all three owners;
detached authority is not a completion proof or retry authority. Mailbox close
still reports its existing zero/success and one/failure ABI.

Final-root mailbox teardown detaches the complete admitted namespace, then
drains its existing nodes without a teardown snapshot after the local capability
registry leaves. It **still holds the enclosing lifecycle guard**. Legacy queue
BTreeMap allocation/destruction also remains separate. This correction does not
claim to qualify those wider contexts.

## Selected ownership evidence

The [new serialized fixtures](../../../crates/catten/src/syscall/mailbox_publication_tests.rs)
run inside the existing mailbox self-test on all targets:

- Namespace node, endpoint node, budget and shared authority-node rejection
  precede charge/serial changes. Original counters/node admission remain equal;
  the next successful open gets the next identity.
- Existing receiver lookup bypasses an armed storage-rejection hook. Earlier
  real-dispatch fixtures still cover receiver reuse at the 512-record ceiling,
  shared quota rejection, 1,024 open/close cycles and exact generation reuse.
- Publication and payload/authority detachment succeed with the actual heap
  held. Completion runs after local guards leave. Unstarted staged cancellation
  explicitly returns both charges.
- A staged root close rejects a previously prepared open. Explicit cancellation
  completes its operation; root close then completes instead of retaining a leak.
- Guarded preparation/retirement abandonment holds lifecycle, both mailbox
  registries, CPU tables and physical/heap allocators simultaneously. It retains
  **two exact root leases, two ordinary mailbox charges and two original shared
  authority charges**, including one detached record. Both roots remain Pending.
  All unused nodes/budget control blocks and installed CPU-root/table backing
  remain with those transactions. No mailbox queues, heap/image data frames or
  IOMMU table/data backing are created by these abandonment probes.

Allocation/disposal entry probes require those local guards and the unified
capability guard available, preserving the original IRQ state. Counts are phase
entries, not allocation/deallocation balance or quiescence proofs. These are
serialized rejection/reentrancy fixtures, not actual OOM, panic unwinding or
cross-LP interruption stress.

## Initial failed fixture

The first Intel artifact had SHA-256
`c7db2c6552531a940019aafbbbb159805f8663dc275f9c29fdfe65fcaf74004c`.
It stopped before an authoritative final result at:

```text
syscall/mailbox_publication_tests.rs:211:5: assertion left == right failed
left: 1
right: 0
```

The fixture incorrectly expected the detached authority's original budget charge
to be zero. `RetiredRecord` correctly retained it after detachment and guarded
abandonment. The assertion now requires one, matching the preparation-side
retained charge and both retained family charges. This is a fixture correction;
the failed run is not merged into passing evidence or described as a production
cleanup failure. It does not explain historical user-stack lease timeouts.

## Validation

Fresh-storage guests run sequentially, reusing validated embedded bundles because
only kernel code changed:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance mailbox-authority-intel-repeat-20261010 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance mailbox-authority-amd-20261010 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18779 CATTEN_DEPLOY_HOST_PORT=18079 scripts/run-aarch64.sh --security-test --instance mailbox-authority-arm-20261010 --fresh-storage --timeout 180
```

| Guest | Final result | Mailbox preparation / disposal phase entries |
| --- | --- | ---: |
| Intel VT-d, corrected repeat | 15 passed, zero failed/pending; `0x1bbff` | 12 / 15 |
| AMD-Vi | 15 passed, zero failed/pending; `0x1bbff` | 12 / 15 |
| Arm SMMUv3 security guest | 19 passed, zero failed/pending; `0x2003ffff` | 12 / 15 |

Arm scoped checks also pass `0xffff` at publication generations 1 and 2.
Final kernel SHA-256:

- Intel/AMD x86: `d253544803aca21ec0c889e49c6c61db96003721cfe34ab8ec26e60b722b7fb1`.
- Arm security guest: `e8ffbd353bca4df47995d1113a9aa8fb60b50ba5cf79c4c6e4dea7758cff81e6`.

Strict locked default-feature Clippy passes on both architectures (`-D warnings`),
as do formatting and diff checks. Runners verify native assembly section/load
permissions (249 x86 entries, one Arm entry). Shared admitted-map code/host
fixtures are unchanged; the preceding eight-test/full-host-harness evidence
remains in the [namespace report](2026-10-09-security-capability-namespace-storage.md).
That harness is not rerun in this kernel-only pass; new ownership adapters and
existing quota/churn/generation fixtures execute in all three guests.

## Remaining boundary

Final-root mailbox teardown still needs post-lifecycle ownership qualification;
legacy queues need their own storage/admission work. Other authority callers,
active reservation/escrow fallback, general heap-byte/principal/progress budgets,
all enclosing syscall masks/guards, cross-LP pressure and abandoned-owner custody
remain open. Historical intermittent Intel/AMD user-stack lease timeouts remain
causally unresolved; successful new guests do not close C16/G1/G2/G7. The
[current ledger](../../reference/security-remediation.md) preserves their failed
runs separately from repeats and retains the original SEC-07/18 criteria.
