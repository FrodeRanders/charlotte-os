# Pending reply CQ readiness — 2026-10-12

This follow-up to `289825c3` addresses the multi-source controller/probe gap in the [weekly wait sweep](2026-10-12-wait-readiness-sweep.md). `grantctl` and the security probe now wait on registered endpoint/reply/lifecycle interests, with a kernel watchdog only for the earliest remaining operation deadline. Their normal loops no longer wake every five milliseconds. Cleanup-claim, pin, reaper and shootdown notification work remains open; SEC-18 is still partial.

## Notification and ownership contract

New syscall **87, `IpcReplyBindCq`**, is exposed through `PendingCall::bind_cq`. One owning interest resides in the existing admitted pending-call record. The current caller thread sponsors its registration. Observer/entry preparation happens outside IPC; publication revalidates the exact pending-call ID/capability and rejects a second binding without replacing the first. A reply winning registration or publication causes a post-guard wake. Ordinary interest rejection leaves the pending capability and any Rust memory borrow owned. Closing that owner cancels its subscription; no separate scalar watch registry exists.

The interest uses the call's existing observer list, so ordinary replies, reply-token cancellation and terminal endpoint results share existing readiness publication. It does **not** observe a result or deliver hidden returned connection/memory authority. Those transfers still require the owned call's first result observation. No readiness notification authorizes physical release, loan reuse or replay of an accepted request.

`completion::CqInterest` captures the admitted waiter-list identity of one exact CQ lifetime. Before changing work generation it verifies that identity under `COMPLETIONS`, then notifies detached waiters outside the registry. A late callback cannot wake a successor at the same numeric ASID/CQ. This retains original list metadata/admission, not ring backing or a root/physical completion receipt.

New syscall **88, `CqWaitChecked`**, has timed and event-only modes. It returns work, timeout, wait-admission rejection, or invalid queue distinctly, using completion status codes in the second return register. Timed mode borrows the scheduler's existing owned watchdog machinery, captures the thread generation, preserves `CompletionQueueWait` migration constraints, cancels unused registrations/watchdogs and reparks after unrelated wakes within one absolute deadline. Event-only mode admits an owned source subscription without a timer. The wire reserves `u64::MAX` for event-only waiting. Existing explicitly fail-soft CQ wait APIs retain their boolean/void contracts; they are not used to collapse rejected admission into timeout in the corrected reactors.

`grantctl` binds its endpoint and every accepted lookup to the default CQ. It keeps each existing pending transaction and five-second deadline intact, computes the earliest remaining deadline before parking, and retries rejected wait admission with an explicit ten-millisecond admission backoff. That exceptional backoff is not a normal readiness wait or a cleanup completion. Interest rejection cancels the exact accepted lookup and replies unavailable, without replay or presumed remote rollback. A sixteen-message intake limit still permits reply/lifecycle progress; reaching that limit continues productive intake before parking, because a coalesced wake need not provide another edge for remaining queued messages.

The probe binds the actual endpoint it must serve while awaiting each call, including its later record-pressure endpoint. It drains productive work before parking on the same CQ and uses a checked remaining-deadline watchdog. Its idle loop also parks on that CQ. A 32-request unauthorized batch tests controller drain beyond one intake cycle; it does not deterministically freeze the producer to prove all 32 were simultaneously queued.

`ServicePages::write_request` still borrows its exact admitted root and fixed image mappings. After the request's release publication and local table-guard exit, it prepares and publishes default-CQ work while that root prevents namespace reuse. Rejected page access publishes no request/wake. It adds no cleanup-lease drain subscription and does not advance acknowledgement or device gating. The signed probe's cooperative retirement exercises its idle lifecycle wait; the ordinary node shutdown roster excludes the grant controller, so power-off is not a grant-controller acknowledgement proof.

The existing C02/C07/C08/C18 rows in [cleanup recovery](../../reference/cleanup-recovery.md) record this scope. Broader implicit metadata destruction under IPC and outer guard/IRQ contexts remain G1/G4. No uncertain physical cleanup receives a retry owner, force-clear or terminal result from these notifications.

## Failed executions retained separately

The first service build failed because the endpoint wrapper was called `bind_cq`; its actual API is `bind_completion_queue`. Correcting that test/controller source error permitted compilation. A preliminary kernel build also warned about an unused returned-memory fixture counter; adding its assertion removed the warning. Preliminary normal passes are not substituted for final-artifact execution below.

Two ARM security executions of the earlier implementation failed. It allocated an extra `Completion::timer` solely to drive each probe wait's deadline:

| Earlier artifact | Observation | Result |
| --- | --- | --- |
| Kernel `a4c09668231193623b6d720445b91e2bc65de3d76dd56c8d552c432fc929ded2`; ARM bundle `14a2b622d584bd97ed43577d127cdf91cdb9355149cf8ffb3c7baccdfbe46b3e` | 32 denials pass at 12.087319 s; RPC pressure checks pass at 33.958233 s. Probe aborts at 34.333205 s without a precise helper diagnostic. Kernel `el0_security.rs:106` reports exit before adversarial checks. | 20 passed, 1 failed, 0 pending at 34.778954 s; passed `0x123ffff`, failed `0x20000000`. HTTP keyhole not validated; runner exits 1. Output SHA-256 `05658a1e2401f651acf1d2be5537cb652acdd4d4894628549564db434bbd0c5d`. |
| Diagnostic kernel `be7279dda83a097c20b6c5a8d04c98c15045342042448c148eac66c6ae64af7e`; ARM bundle `8b8fe67dbe9c199f0f45c519921cd657342931836b623b7682aa9cfcc5f2e710` | RPC pressure checks pass at 32.806768 s. At 33.146004 s the probe records `wait deadline signal rejected: SubmissionFailed`, then failed check **104** at 33.146420 s. Kernel `el0_security.rs:100` reports that exact failed check. | 20 / 1 / 0 at 33.698954 s, same bitmaps; runner exits 1. Output SHA-256 `d5b40de608846efcbec3b8f37d44505d81065165c3afae8cbe1994a06266ee23`. |

The diagnostic establishes completion-timer submission rejection at the deliberately full-budget probe boundary. It does not identify which internal submission quota/allocation rejected, or prove the earlier undiagnosed exit had the identical cause. The correction removes that extra completion-capability dependency, uses the owned CQ watchdog instead, and preserves the original pending owner/deadline. Subsequent full-budget controls pass. This is not evidence that all completion/timer admission pressure is resolved.

## Final validation

Both complete signed bundles were rebuilt before the final executions, using `scripts/build-catten-services.sh --embed` and `scripts/build-catten-services-x86_64.sh`. Every final run below exits 0; failed runs remain separate:

| Configuration | Passed / failed / pending | Passed bitmap | Guest completion |
| --- | --- | --- | --- |
| ARM security/HTTP/DHCP, fresh disk | 21 / 0 / 0 | `0x2123ffff` | 57.767475 s |
| Intel HTTP/DHCP, fresh disk | 18 / 0 / 0 | `0x123bbff` | 20.012932 s |
| AMD HTTP/DHCP, same x86 kernel, fresh disk | 18 / 0 / 0 | `0x123bbff` | 23.523842 s |
| Separate ARM shutdown ingress, fresh disk | 19 / 0 / 0 | `0x103ffff` | 10.954785 s |

Kernel fixtures verify reply/cancellation/late binding, duplicate-binding rejection, unchanged observation state and rejected source admission. A full source rejects both a ten-second checked wait and an event-only wait without blocking or losing its original registrations. Sixty-four real checked timeouts leave zero source waiters. Timed and untimed scheduled callers are confirmed parked before a producer replies; both wake without a completion-ring entry (ARM marker at 8.830822 s). Returned-memory tests retain hidden authority after CQ notification until observation, and preserve cleanup for both observed/unobserved results. A stale captured CQ interest cannot increment or wake a successor queue; detached callbacks remain tied to their original source. Exact-root page fixtures verify default-CQ generation advances after valid request publication and retain existing rejection/abandonment checks.

Both signed ARM probe generations drain 32 controller denials at 12.746994 and 35.386465 s. They pass full-queue publication deadlines and eight-slot recovery at 24.523706/47.099085 s, then scalar/moved RPC deadlines and original page-charge/slot controls at 34.700909/57.244201 s. Cooperative probe retirement completes with retired policy fenced and publication generation 1/2 at 35.227188/57.757710 s. This exercises the checked CQ ABI and self-serving/idle waits from EL0, including deliberately full memory/record/observer/timer budgets; it does not inject uncertain hardware/loan cleanup or measure CPU savings.

HTTP/deployment/node/cluster controls pass on ARM/Intel/AMD. Trickled deployment rejection/recovery takes 30.16/30.11/30.13 host seconds. Expected reset/timeout retries remain visible; healthy controls succeed. Shutdown acknowledges ObjectStore at 14.690288 s and requests PSCI power-off at 14.702004 s, without claiming coverage for the excluded grant controller.

Host suite: **382 passed, zero failed, three ignored**. The new owned-runtime test rejects reply interest while preserving its pending loan owner, then binds successfully and verifies cancellation/cleanup. Strict service and ARM/x86 kernel Clippy, workspace formatting, diff whitespace and syscall emitter coverage pass; the latter checks **90 wrapper sites across nine arities**.

Kernel SHA-256: ARM combined `e9f70c9bf9e63d0184a7723fe8472a7a24ab7e849fcf1e16c5488d80bce1636d`; Intel/AMD `291013b790215bb60b22272830487d72e5b8096c7457c11c3e48f67f9264a232`; ARM shutdown `618c7326a54ac8218b3571a6392e831ea28c619ba63919fb360a1964c8c3b199`. Sorted `filename:ELF-digest` bundle hashes: ARM `e8bf681a4f01d71c30cecfe9d13f1362e3b7e9f8c79a04e078d3befbcd915621`; x86 `2bf773dfb18cbd457ddbe31a7c2d81816e5d8d147d9e57a6425f11f4ee5bc999`. Grant-controller ELF hashes: ARM `90994410648e6e04cee0c9fae925a6710d47d90971dfb393f4dadea34cf0a351`; x86 `94aac770cb0325b9492738d7f0007036debc4b0429244f43a4677bbbf47aa56d`.

Reproduce host checks with `scripts/run-host-tests.sh`. ARM uses `--security-test --http-test --dhcp-test --fresh-storage --timeout 180`; Intel/AMD use `--http-test --dhcp-test --iommu intel` or `amd --fresh-storage --timeout 180`; shutdown uses its separate `--shutdown-ingress-test --fresh-storage --timeout 180`. Give concurrent guests distinct instances and forwarded ports, and reuse kernels only for identical profiles/bundles. The retained cleanup/reaper/shootdown gaps and historical Intel object-store/AMD NIC failures remain separate work.
