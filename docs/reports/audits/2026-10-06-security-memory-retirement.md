# Owned namespace memory cleanup — 2026-10-06

Baseline: `310bd4db`, following
[device retirement and DMA loans](2026-10-06-security-device-retirement.md).
This advances SEC-18 for whole-domain memory cleanup and corrects a newly
identified unmapped-reader cleanup gap, SEC-28. SEC-18 remains partial.

## SEC-18: retain every mapped root outside lifecycle

Whole-domain memory cleanup previously retained lifecycle through object
detachment/invalidation. Its backing pin correctly prevented the last DMA/copy
unpin from freeing reachable frames, but mapping records carried only numeric
ASIDs and did not retain mapped peer roots independently.

Mapping publication now captures the exact `AddressSpaceHandle` before registry
access while the calling operation owns that generation. Existing mapping nodes
carry optional cleanup leases. Ordinary mappings hold no lease, allowing their
own root to retire. This adds no translation/metadata admission budget.

After device and IPC cleanup, `PreparingNamespaceObject` borrows its closing
root and acquires a backing pin while mappings remain visible in the registry.
It captures one mapping identity at a time, releases the registry, admits its
exact peer root under lifecycle/table, then installs the owner in the admitted
node. Already-closing peers are eligible before sealing. Its own closing root
and the permanent kernel root require no extra user-root lease. Competing memory
cleanup sees the pin and returns Pending, so it cannot remove the mapping and
seal a root before its lease is admitted. No root/capability/mapping snapshot is
allocated, and lifecycle is never acquired beneath the object registry.

Only after every affected peer is retained does `PreparedNamespaceObject` own
the detached mapping tree and backing pin. Batches of 16 borrowed frame
identities check leaf ownership and detach after releasing the registry. TLB
invalidation and scratch completion run without lifecycle/table/registry guards.
Confirmed completion releases backing/authority before returning peer leases.
The original sponsorship charge survives until physical release, including
deferred DMA/copy unpin after owner destruction.

Unstarted peer-admission rejection explicitly returns admitted leases and the
pin without moving mappings or changing destruction state. Detach, invalidation,
scratch failure or abandonment retains the backing pin, original charge, all
affected mapped roots and unconfirmed scratch extents. Root close returns
`MemoryCleanupFailed`; earlier completed objects stay completed. Rejected frame
release propagates the same close error after quiescence, retains its original
charge and mapped roots, and does not retry partially released backing.

Unmapped caps also wait for live revocation/transfer and DMA/copy fences before
borrower state/authority removal. Destroy-fenced objects cannot resume usable
authority and retain their independent backing pins/charges. A competing live
operation returns the closing owner as Pending for later polling. Timeout or
abandonment retains the fence/root; no force-clear or scalar recovery is added.

Exact object and namespace completion receipts gate progression. Cleanup
admission now seals **after device, IPC and owned memory completion plus zero
leases**, before remaining namespace metadata removal and final root detachment.
Incoming cleanup leases remain possible during owned memory preparation, when
registry pins and admitted mapping records protect the affected payloads. They
cannot enter after sealing. Final root/table/heap/image destruction retains its
existing detached owner. The serialized memory adapter is explicitly named
`close_address_space_fixture` and restricted to raw kernel boot fixtures.

## SEC-28: unmapped reader removal could restore stale borrower state

**Severity: medium robustness/availability, conditional on kernel-managed
non-IPC loans. Corrected.** `LoanRevocation` moves the prior read-borrower list
into its owner while publishing `Revoking`. Before this change, another reader's
namespace could close while it had no mapping: `clear_borrower` skipped the
active pin/Revoking state, but final memory-cap removal still consumed its cap.
When the first revocation finished, its owned prior state could restore that
now-dead reader's numeric ASID/capability entry. The stale reader could keep
lender restrictions active and impede future loans or normal owner close.

Final namespace memory-cap removal now returns Pending while a live revocation
or other relevant pin/transfer fence owns the object. Once prior state returns,
cleanup clears that reader and removes its capability under the same registry.
No stale borrower is restored after namespace disappearance.

The guest fixture reproduces this state sequence with real read loans, an
unmapped third reader and a real prepared revocation. It verifies Pending and
retained authority, completes the first revocation, then closes the third reader
and checks that lender restrictions clear. Normal IPC-tracked readers also have
IPC cleanup claims that can reject earlier; this fixture validates the general
memory boundary for kernel-managed loans. It is not an application-triggered
denial-of-service exploit or a demonstrated confidentiality/integrity breach.

## Validation

New boot fixtures cover local/scratch and foreign/direct mappings across three
roots, three frame batches per mapping, already-started closing peers, competing
peer close, exact scratch reuse and complete charge refunds. A stale captured
peer generation is rejected before any table walk; earlier admitted peer leases
are returned, its real successor mapping stays intact, and the closing owner
and charged object remain retained. No corrupted generation is force-restored.

Fault fixtures reject partial detach, invalidation and scratch completion, and
abandon a real prepared receipt. They assert retained backing/charges, scratch,
source root and mapped peer roots. Legacy failed-loan fixtures now expect both
roots to remain closing rather than erase authority under an uncertain
revocation. In total, this batch deliberately retains **18 additional roots**:
eight from those existing failed-loan cases, eight from the new physical failure/
abandonment cases, and two in the stale-identity case. New fixtures additionally
retain nine object pages/charges and four heap pages, beyond earlier audit
quarantines. No recovery bypass frees them.

Callbacks assert lifecycle, table, object-registry and scratch guards are absent.
A deferred production fixture closes local and peer mappings after secondary
LPs start, exercising real x86 synchronous shootdowns. These roots have no
application threads; concurrent hardware walks and failed-recipient recovery
remain separate work. Physical allocator rejection is propagated by the owning
release path, but this batch does not inject real allocator corruption.

| Check | Result |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed, including 23 slot tests, scratch ownership, runtime/services/protocol, signing and boot-result suites. |
| `scripts/build-catten-services.sh --embed` | Passed; staged/signed AArch64 bundle. |
| x86/AArch64 kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check` and `git diff --check`: passed. |
| Four-LP x86 guest | **15 passed, 0 failed, 0 pending**, including new memory preparation/failure fixtures and deferred mapped-peer cleanup. |
| AArch64 security guest | **19 passed, 0 failed, 0 pending**. Both probes reported `0xffff`, generations reached 1 and 2, cancellation traffic retired after 4,776 requests. TCP/IP clock/cycle progress held while pressure clients retired after 1,255 and 1,359 requests. |

Successful guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance memory-owned-20261006 --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18496 \
CATTEN_DEPLOY_HOST_PORT=17796 scripts/run-aarch64.sh --security-test \
  --instance memory-owned-20261006 --fresh-storage --timeout 180
```

The initial pair stopped at the older fault fixture's successful-root-close
expectation; that assertion was corrected to verify conservative retention before
the successful isolated runs. Runners rebuilt kernels, checked native assembly
permissions and reused signed service bundles. ARM forwarding ports required
approved execution. No unrelated VM or storage was stopped/reset.

## Remaining scope

SEC-18 remains partial: IPC move/copy/result attachment cleanup still retains
outer serialization. Recoverable shootdown failure, full CPU/DMA quiescence,
physical-device reset and recovery for abandoned roots remain open. An injected
false barrier is not rejected hardware IPI delivery. Final root shutdown still
depends on caller-established thread quiescence and recipient progress.

SEC-07 translation/metadata admission, peer/management authentication, production
provisioning and security-time work remain open as previously documented.
