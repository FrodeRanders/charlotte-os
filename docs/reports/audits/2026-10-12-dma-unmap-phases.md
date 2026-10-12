# DMA unmap rejection and completion phases — 2026-10-12

Scope: SEC-18 / C13, C14, C18 / G2, G3, G7. Baseline: `8e41f7cd`.

DMA unmap now preserves the distinction between rejection before backend work
and an ambiguous failure after claiming the actual domain/command engine.
The owned runtime records the attempt before kernel entry and does not submit
another unmap after ambiguous failure, including from Drop. This is a
prerequisite for extending contention readiness beyond scheduled grant creation.
It adds no wait, polling loop, cleanup registry or physical retry owner.

## Finding and correction

At the baseline, the unmap syscall returned `1` for every kernel error. The
public operation adapter also collapsed backend admission contention into
DmaInvalid. `DmaTransfer::finish` and `SharedDmaMemory::finish` retained their
memory owners on failure, but left DMA marked active. A second finish or Drop
could therefore issue another unmap after a hardware timeout. A host test even
modeled a generic MAP_FAILED response followed by a successful resubmission.

The existing kernel quarantine still prevented uncertain pin release; this
finding is an invalid runtime replay contract, not an observed early backing
reuse. A failed unmap can already have detached its leaves and moved the pin
into the domain's admitted quarantine. The same IOVA is not a completion receipt
or a retry owner, and another call need not describe the original cleanup.

All three backends now retain typed `UnmapError::Admission` versus `Completion`
through the production unmap adapter. Taking the actual domain/engine establishes
the boundary. Only Admission(OperationInFlight) becomes the public busy status.
An error after that boundary remains DmaInvalid, even if a future underlying
cause happens to look busy. Rejection while completing the public claim/root
after backend work is also terminal; its error cannot escape as admission busy. Raw serialized fixture adapters expose the original
backend cause while production retains the phase distinction.

The syscall returns the existing device-status ABI rather than a boolean:

| Outcome | Register status | Runtime treatment |
| --- | --- | --- |
| Confirmed unmap/pin release | OK, 0 | Record completion; permit CPU memory return |
| Capability claim or backend admission busy before work | OPERATION_IN_FLIGHT, 20 | Retain the same owner; another unstarted submission is possible |
| Error after actual backend claim | DMA_INVALID, 14 | Retain original status/owner; never resubmit that transfer's DMA unmap |
| Other rejection or unknown status | Existing specific code or unknown value | Conservatively retain status/owner without DMA resubmission |

The syscall crate now names existing status codes ADDRESS_SPACE_CLOSING (19)
and OPERATION_IN_FLIGHT (20). No compatibility fallback or new syscall number
is introduced. Only the DMA unmap contract qualifies status 20 here; other
device operations retain their own contracts.

`DmaTransfer` and `SharedDmaMemory` share local Active/Complete/Retained progress.
They arm Retained before entering the kernel, record actual success, and rearm
only on explicit pre-work busy rejection. Subsequent finish on Retained returns
the stored error and the same owning value without another kernel call. Drop
also skips uncertain DMA resubmission. Confirmed DMA completion remains recorded
when shared-memory CPU unmap later fails, preserving the existing no-repeat
behavior for that step.

Best-effort CPU unmap and memory-capability close remain the existing local
fallback policy. A close attempt is not physical release: uncertain DMA pins,
domain quarantine, root dependencies and original backing charges remain with
the kernel's complete owners. No caller may infer quiescence from the stored
status or retry physical teardown by IOVA/address.

## Executed evidence

Five additional host tests cover:

- Exclusive transfer ambiguity, including DMA_INVALID, UNKNOWN_CAPABILITY and
  an unknown status: a second finish returns the original error without a call
  or CPU-owner return, and Drop issues no second DMA unmap.
- Shared transfer ambiguity: repeated finish preserves both the error and owner;
  Drop attempts existing CPU/memory cleanup without DMA replay.
- Drop after explicit busy rejection: one best-effort unstarted submission is
  allowed, and a subsequent ambiguous result is not repeated.
- Shared busy rejection followed by success: CPU unmap occurs only after the
  later successful DMA submission.
- Injected kernel-entry interruption for exclusive and shared owners: progress
  is already armed, and unwinding performs no second DMA call. The mock injects
  the panic after recording entry and outside its state lock.

The former generic-failure retry test now requires explicit OPERATION_IN_FLIGHT.
Existing successful teardown ordering, failed map ownership and shared CPU-unmap
failure tests remain enabled. Mock close events are cleanup attempts, not proof
that a real kernel released uncertain backing; guest evidence supplies the pin
retention checks separately. Host panic interception is not executed kernel
unwinding evidence.

Kernel synthetic containing-owner tests check Admission(OperationInFlight),
Completion(OperationInFlight), Completion(HardwareTimeout) and other admission
rejection. Only the first returns OperationInFlight. Each invokes work once,
keeps the original capability cell and fences nested unmap/root close through
the exact public operation owner. Synthetic trap frames execute the actual
syscall dispatcher and assert busy 20, invalid-capability 1 and DMA_INVALID 14.

Existing real mapping checkpoints additionally call the typed unmap entry while
the actual engine/domain is detached, requiring Admission(OperationInFlight)
without entering completion. The real failed-unmap fixture now executes its
injected hardware completion rejection through the syscall dispatcher, requiring
register status 14. A final public-claim fault on a second empty fixture cap
adds one retained operation count to the existing abandonment fixture root,
without a new root or physical obligation. Its empty cap is disposed through
the explicit fixture close adapter; this cannot refund that failed count.
Original table charges remain unchanged and memory close
still rejects with LendingActive. Only confirmed real domain retirement releases
that pin and allows final memory/root cleanup. These checks execute on VT-d,
AMD-Vi and SMMUv3; no new abandoned root or physical retry is added.

Every final guest emits the assertion-backed phase marker:

```text
[DMA unmap phase] only pre-work busy returns OperationInFlight; completion busy/timeout and other rejection remain DmaInvalid; exact public root/claim and original capability cell preserved
[DMA unmap phase] final public-claim rejection remains terminal after work; failed completion retains its exact count in the existing abandonment fixture root
```

The resource-ownership guide, LaTeX memory chapter, AGENTS.md and existing
cleanup-owner rows now describe the same conservative replay boundary.

## Validation and artifacts

Both signed bundles were rebuilt with `scripts/build-catten-services.sh --embed`
and `scripts/build-catten-services-x86_64.sh`. Host suite: **389 passed, 0 failed,
3 ignored**. Strict `-D warnings` Clippy passed for bundled services and both
final-source kernel profiles. Formatting, diff and changed Markdown-link checks
passed.

The earlier four-profile matrix passed before final public-claim rejection
hardening. The final-source matrix below was rebuilt and rerun with that
additional terminal-status assertion; those earlier passes do not substitute
for final artifacts. No guest failure was captured in either matrix.

Final-source fresh-storage guests used the 180-second runner deadline:

| Profile | Passed / failed / pending | Final bitmap | Completion time |
| --- | --- | --- | --- |
| ARM SMMUv3, security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | 57.445743 s |
| x86 VT-d, HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 23.113962 s |
| x86 AMD-Vi, HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | 27.807787 s |
| ARM shutdown ingress | 19 / 0 / 0 | `0x103ffff` | 10.531122 s |

All four runners exited 0 with zero failed/pending bitmaps. Synchronous phase
and syscall assertions are additional to those deferred totals. HTTP/deployment
ingress controls passed; shutdown requested PSCI power-off at 13.858284 s.
No guest failure was captured in this matrix. Concurrent x86 samples are not a
production contention soak or reliability claim.

Commands used `CATTEN_SKIP_EMBED_BUILD=1` after both bundle builds:

```sh
scripts/run-aarch64.sh --security-test --http-test --dhcp-test --fresh-storage --timeout 180 --instance dma-unmap-arm-final
scripts/run-x86_64.sh --http-test --dhcp-test --iommu intel --fresh-storage --timeout 180 --instance dma-unmap-intel-final
scripts/run-x86_64.sh --http-test --dhcp-test --iommu amd --fresh-storage --timeout 180 --instance dma-unmap-amd-final
scripts/run-aarch64.sh --shutdown-ingress-test --fresh-storage --timeout 180 --instance dma-unmap-shutdown-final
```

HTTP/deployment ports were 19340/18640, 19341/18641, 19342/18642 and
19343/18643. Immutable per-boot hashes precede subsequent profile builds:

| Artifact | SHA-256 |
| --- | --- |
| ARM combined kernel | `e84555351b1db7d14d2758bf519f60bf1e799c6a88f102b172856e3d14717cc6` |
| Intel / AMD kernel | `4a33897f94702826b57fed6e31a9d710ca8aaf9bf1421e57401162bfe7a17aa2` |
| ARM shutdown kernel | `71e0ddf4c519673100286068cedb5d523cf4579b7c2d6fcc7771a6a4cdff83d8` |
| ARM bundled service ELFs | `f74e73eeae72102ea2b0b43837c11934151df5beb57698af4e61bc549a76f644` |
| x86 bundled service ELFs | `90e43c8e691e01e151898a1caf337e22d9f7141744dbc3054d9c4424d0e69ccd` |

Bundle hashes use sorted newline-terminated `basename:ELF-sha256` lines hashed
with SHA-256. Results, hashes and relevant diagnostics are inline; no temporary
log or ignored build artifact is an evidence dependency.

## Remaining limits

The [scheduled grant readiness contract](2026-10-12-dma-unit-readiness.md) is
unchanged. Public DMA map/unmap/close and grant cancellation still reject
contention rather than registering interest; their real caller/IRQ contexts
need qualification before parking. This correction adds no polling/backoff
and gives busy rejection no completion or physical retry authority.

Other userspace CPU/MMIO cleanup retry contracts, raw driver adapter teardown,
indefinite claimed-unit retention, sustained admission pressure and broader
outer Drop/IRQ contexts remain separate. Retained runtime status has no recovery
adapter, even if a later domain operation establishes quiescence. The
[historical Intel startup failure](2026-10-12-ipc-close-readiness.md) remains
causally unresolved. SEC-18 and SEC-07 remain partial, with existing production
enrollment restrictions and physical retry prohibitions intact.
