# Object-store readiness and unreadable mount metadata — 2026-10-12

This follow-up to `2b07d93d` investigates the retained Intel failure in the [controller deadline report](2026-10-12-security-controller-deadlines.md). It corrects source-confirmed readiness and mount-decision defects. The underlying historical block-read rejection is not reproduced or attributed here; SEC-06/07/18 remain partial.

## Historical observation and source findings

The previous final-artifact Intel run panicked at `supervisor.rs:637`: `local object store failed before node readiness`, observed diagnostic **209 (`0xd1`)**. Last status at guest 17.736503 s: 15 passed, 3 pending, passed bitmap `0x33bff`, pending `0x1208000`; HTTP did not validate and the runner exited 1. Kernel SHA-256 `ad4a38fc0841c651fa920e603dfdd8e54dfff7879147f9037fd13986b7586a75`; runner-output hash `3484ebe5efe4be7cd63fe2867757155608b3f6cdde2ad69b4b1144e1a3ecec37`. The store logged ordinary operator-scope operations at 15.911757/16.330579 s. Its exact read, caller, rejection phase and device result were not captured. The earlier same-artifact passing repeat remains a separate observation.

Source inspection establishes three defects independent of attribution of that read:

* Readiness accepted any diagnostic `STAGE >= 4`, although initialization itself writes stages 10–42 before endpoint publication. A diagnostic number was not proof that initialization finished.
* Every block read/write/flush failure latched `ERROR`, including ordinary serving requests. Readiness asserted that this field was zero, treating a request-local I/O diagnostic as a fatal startup result.
* Superblock allocation, IPC read and mapping failure all returned `None`, the same result as successfully read invalid/absent metadata. Two such failures could select formatting. An unreadable redundant slot could also be silently ignored in favour of another slot.

`BLOCK_SIZE` and `BLOCK_OP` additionally shared offset 16, so ordinary I/O overwrote the published geometry diagnostic. Their offsets are now disjoint.

## Explicit state and first-failure record

`objstore_status::STATE` (offset 12) is a release/acquire publication: Starting 0, Serving 1, Draining 2, Failed 3. Serving is published only after mount, scope initialization and endpoint registration succeed. Handled startup failure records `STARTUP_ERROR` at offset 8 and publishes Failed before domain abort. Cooperative shutdown publishes Draining before endpoint close/flush. Diagnostic stages remain separate.

Kernel readiness borrows the exact service pages through the existing admitted root operation and acquires this state. Starting is pending; only Serving with zero startup error qualifies. Draining, Failed, unknown state or startup error reject. The publisher revalidates after network verification, before publishing its local marker. This is a point-in-time boot milestone, not a continuing root, device-health or hardware-quiescence lease. Existing earlier dependency/RPC waits and their broader progress gaps remain.

Ordinary I/O failures no longer mutate startup outcome. In this single-threaded service, an immutable first-failure record stores service state, block operation, LBA/count, phase and available IPC status/device result before release-publishing `FIRST_IO_ERROR`. Later errors cannot overwrite those coordinates. The first failure logs once; diagnostics neither replay I/O nor authorize cleanup/recovery.

Record offsets: first error 40, phase 44, status 48 (u64), result 56 (i64), LBA 64 (u64), count 72, state 76, operation 80. The last block operation is at 84; geometry stays at 16/20. Phases distinguish submission 1, observation 2, returned failure/unexpected attachments 3, metadata allocation 4 and metadata mapping 5. `u64::MAX` means unavailable status, and `i64::MIN` unavailable result. Borrowed-call admission still has its existing opaque ABI; a submission trace may identify phase/coordinates without the exact IPC rejection. A phase-3 zero result can identify rejected unexpected attachments rather than a device error.

## Fail-closed mount decision and ownership

`read_superblock` now returns `Result<Option<Superblock>, ()>`: allocation/read/mapping failure is `Err`, while successful decode retains the existing optional candidate. The production `objstore_mount::select` propagates either slot's read failure before selecting a candidate or absence; even a valid peer slot does not turn unreadable metadata into permission to continue/format. Mount aborts on that failure before formatting. Successfully read blank/invalid candidates, generation selection and the existing incompatible-layout formatting policy remain separate; this change is not a complete corruption-recovery or sudden-power-loss contract.

The same typed block connection, borrowed-memory pending call and transient mapping remain the operation owners. The pending call terminates before page reuse; returned unexpected attachments receive typed disposal. No failed operation is retried, reconstructed from an address or represented by a new recovery registry. Ordinary unused resources use their existing ownership cleanup; uncertainty still follows the existing abort/retention policy. C18 in [cleanup/recovery](../../reference/cleanup-recovery.md) records this scope without wider allocator/IRQ/Drop qualification.

## Regression and execution evidence

Four new host tests execute shared readiness classification, field-width non-overlap and the production superblock selector. Selector fault schedules reject unreadable slot 0 or slot 1, including a valid peer, for allocation/submission/observation/device/mapping error labels; they preserve the error and stop further reads. Successful absent and redundant candidates retain their distinct outcomes. These are controlled host read-result schedules, not actual hardware/mapper fault injection in a booting store.

Both complete signed bundles were built before guest validation. Every run below exits 0; failed historical runs are not folded into the passing counts:

| Configuration | Passed / failed / pending | Passed bitmap | Guest completion | Controls |
| --- | --- | --- | --- | --- |
| Intel fresh A | 18 / 0 / 0 | `0x123bbff` | 23.213286 s | Complete deployment/node/cluster recovery |
| Same kernel, Intel fresh B | 18 / 0 / 0 | `0x123bbff` | 19.018761 s | Same controls |
| Same kernel, AMD fresh | 18 / 0 / 0 | `0x123bbff` | 18.268639 s | Same controls |
| Same Intel A disk, explicit reuse | 18 / 0 / 0 | `0x123bbff` | 18.302447 s | Mount and same controls on retained disk |
| ARM security/HTTP/DHCP | 21 / 0 / 0 | `0x2123ffff` | 56.413046 s | Both signed pressure probes; full HTTP/deployment/node/cluster controls |
| Separate ARM shutdown ingress | 19 / 0 / 0 | `0x103ffff` | 10.144914 s | Cooperative drain and PSCI power-off |

Trickled deployment rejection/recovery takes 30.18/30.22/30.11/30.17/30.16 host seconds respectively for Intel A/B, AMD, reuse and ARM. Existing bounded reset/timeout retries remain visible; their healthy controls pass. Only ARM runs the older HTTP EOF/reset/idle script.

None of these guest captures emits the new first-block-failure trace. They validate normal published-state consumption, storage round trips, both caller-pressure generations and ordinary reuse/shutdown; they do not establish the original read's cause or exercise a real store's new failure-record/failed-mount branches. No failed guest run occurred in this follow-up. Fresh/reused passes are not proof of physical power-loss durability or retained data under an injected read error.

Shutdown acknowledges ObjectStore at guest 14.495784 s (one acknowledged, zero unacknowledged/forced, lifecycle status 2) and requests PSCI power-off at 14.506534 s. It is ordinary cooperative shutdown, not an uncertain device/loan cleanup test.

Kernel SHA-256: Intel A/B/reuse and AMD share `56587fb8a80eecbf3882966438f41591c587dee07e1e2238e374f6c255c8faca`; ARM combined `7db8967356b6f5f30f979745c8a666534ffa2e6267ff989e2d846253b5e7b12f`; separate ARM shutdown `87db62400b6e95fc1b5a31a4cf7905408a5acba31786e33b8134a19068abafd2`.

Complete sorted `filename:ELF-digest` bundle SHA-256: ARM `d3df1203b9c939669c1007f7b0a86a88f9a521d236bbb078db3ce44218eaf17e`; x86 `74d34df0bfb4807c9759cc4d9926acc90f4d683bd03c1de6990b5e3497c49d36`. Signed object-store ELF SHA-256: ARM `9114e02bf2600eee9e6905c699be15a167a190ecc19ee75ffd249efc37e3103c`; x86 `18260d0a272bdb68040cc9bb5abab9bd9a4695c27f1c4634d44afca5ca36aad4`.

Final host suite: **378 passed, zero failed, three ignored**. Strict service and ARM/x86 kernel Clippy, workspace formatting and diff whitespace pass. No syscall ABI changes were made in this follow-up.

## Reproduction and remaining gates

Build with `scripts/build-catten-services.sh --embed`, then `scripts/build-catten-services-x86_64.sh`. x86 uses `--http-test --dhcp-test --iommu intel` or `amd --fresh-storage --timeout 180`; preserve instances and use `--reuse-storage` explicitly for the reuse case. ARM uses `--security-test --http-test --dhcp-test --fresh-storage --timeout 180`; shutdown uses its separate `--shutdown-ingress-test --fresh-storage --timeout 180` profile. Give simultaneous guests distinct instances/forwarding ports; reuse a kernel only with identical profile/bundles.

The historical Intel read rejection still needs an actual causal capture, preferably the new first-failure fields; borrowed admission may require further status exposure. Continuous storage health, successfully read corrupt/incompatible metadata policy, real allocation/device/mapper failure injection, power-loss durability, aggregate sponsor/heap limits, concurrent client fairness and broader cleanup/IRQ contexts remain open. The separate historical AMD NIC exit remains unattributed.
