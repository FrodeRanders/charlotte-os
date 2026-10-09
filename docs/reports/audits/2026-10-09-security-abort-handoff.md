# Whole-domain abort completes its root operation before caller retirement

This corrects the self-request window identified in the
[published DMA creation follow-up](2026-10-09-security-dma-creation-rollback.md)
within C01/C16 and G1/G2/G7 of the
[cleanup/recovery strategy](../../reference/cleanup-recovery.md).
SEC-18 remains partial; the ledger stays **20 corrected, six partial and four
open**. The historical intermittent timeouts remain unresolved observations.

## Defect and correction

`DomainAbortSweep::run` previously treated its own executing thread like every
other thread selected by the domain scan. `abort_thread_generation` marked that
caller aborted and requested a context switch, while the sweep still owned its
`AddressSpaceOperation`. A scheduling boundary after that request could retire
the executor before its explicit root completion. Reaping its stack would not
complete the separate sweep lease, leaving root close rejected even after the
thread disappeared. This is a source-level ownership gap; the earlier captures
do not establish that it caused their failures.

The sweep now captures one executing TID/generation under a short local mask,
then defers that exact lifetime while requesting peers. The scan retains its
captured slot ceiling, exact root/generation checks and existing publication
fence. Its callback and ordinary peer claims run outside the final handoff.

A final `LocalInterruptMask` spans the exact local self-request and consuming
root-operation completion. `request_executing_abort` borrows that mask, qualifies
the current LP handle, thread generation and captured root, then marks the
request and context-switch flag. It does not scan peer LPs, stage a context,
send an IPI, allocate, notify or release backing. Scheduler/LP/thread-table
guards leave before root completion; the mask stays held through that release
and restores only the IRQ state captured on entry. The existing outgoing
handle/context and subsequent architecture switch retain their ownership.

An ordinary peer/request error still completes the sweep's operation and leaves
the terminal thread-admission fence installed. Abandonment retains its root
lease/fence. No root count is force-decremented, no physical release is retried,
and no new custody registry or owner-family row is introduced.

## Deterministic execution evidence

The scheduled EL0 verifier creates one exact root with two admitted threads:
one spins at a second mapped entry, while the other faults on a null read. A
probe armed before the faulting caller's publication selects only this root
generation; its scalar observations do not own resources or authorize cleanup.
The containing fixture retains the root through both threads' retirement.

Before the final handoff, the probe verifies:

- The caller retains its exact thread/root identity and has no abort request.
  Its peer is already requested or removed; a reused peer TID is qualified by
  its original generation.
- Publication, lifecycle and root-table guards are available. Stale caller
  generation and wrong-root local requests reject without marking the caller.
- Eight explicit cooperative scheduler boundaries return with the caller still
  present and unrequested. No guard or local mask survives those yields, and
  the fault entry's IRQ state is preserved.

After root-operation completion, the probe checks that IRQs remain masked, the
caller still owns its current thread lifetime, and its abort owner is this LP.
Arm additionally confirms architecture CPU ownership. The verifier requires
ordinary caller/peer retirement and successful close of that exact root under
the unchanged ten-second deadline. This excludes a leaked sweep lease in the
tested episode and adds no retained root or backing charge.

All three targets emitted this selected result:

```text
[domain abort] real fault: stale identity rejection, peer request, eight pre-handoff yields, masked self-request/root completion and exact root teardown passed
```

## Validation

| QEMU target, four LPs and fresh isolated storage | Authoritative result |
| --- | --- |
| Intel VT-d, default suite | 15 passed, zero failed/pending; passed bitmap `0x1bbff` |
| AMD-Vi, default suite | 15 passed, zero failed/pending; passed bitmap `0x1bbff` |
| Arm SMMUv3, security suite | 19 passed, zero failed/pending; passed bitmap `0x2003ffff`; probe `0xffff`, publication generations 1/2, 4,864 cancellation requests retired |

Both x86 runs executed kernel SHA-256
`c4e7228f3230492b1250b198e33f9b33d75977d18a335192aeeb18ca89d92a5b`.
The successful Arm run executed
`1621d637f51620ed9614d0381e21751ace386122b804d256e490a2811c2ecd53`.
Strict default-feature kernel Clippy passed on both custom targets with
`--locked -- -D warnings`. Rustfmt, diff whitespace, local documentation links
and the unchanged eighteen-family/seven-gate map passed.

An initial compile check rejected equality assertions on the scheduler's
non-`PartialEq` error; the fixture uses variant matching instead. An initial
Arm invocation could not bind its forwarding port in the sandbox and never
entered the kernel. The isolated invocation with port-binding permission passed.
Neither rejected invocation is counted as guest validation. The full host
harness was not rerun: these changes and regression checks are kernel-only.
Existing validated service bundles were reused; guest runners rebuilt kernels
and enforced assembly-section permissions. Runs were sequential after Clippy.

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --instance abort-handoff-intel-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd --instance abort-handoff-amd-20261009 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18629 CATTEN_DEPLOY_HOST_PORT=17929 scripts/run-aarch64.sh --security-test --instance abort-handoff-arm-approved-20261009 --fresh-storage --timeout 180
```

## Remaining scope

The original [Intel preparation timeout](2026-10-09-security-preparation-abandonment.md#execution-and-unresolved-progress-observation),
[Intel IOMMU recurrence](2026-10-09-security-iommu-preparation-abandonment.md#reproduced-retirement-timeout)
and [AMD recurrence](2026-10-09-security-dma-creation-rollback.md#validation-and-remaining-scope)
remain failed validation episodes. This source correction and fresh successes
do not identify which lease those captures retained or establish causation.
Their deadline, phase observations and retained-count policy are preserved.

Concurrent sweeps or unrelated remote aborts can request an executor before
this final self-request boundary. Their interaction with retained kernel
operations still needs context qualification and deterministic progress evidence.
General thread metadata/callback fallback, enclosing teardown caller contexts,
partial cleanup recovery and original failed-pair custody remain G1/G2/G7 work.
Backend initial creation/reset waits, private rollback and further hardware
phase separation remain G3 work. No complete SEC-18 or hardware-recovery claim
follows from this handoff correction.
