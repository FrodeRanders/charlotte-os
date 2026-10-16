# Checked completion operation-ID exhaustion

Date: 2026-10-16. Baseline `82a9cc21`, following detached submission preparation.
Scoped R7/C16 identity-admission work. SEC-07/18 remain partial; the numbered
findings remain 29 corrected within scope, six partial and three open. No new
cleanup family, registry, physical completion proof or retry owner is added.

## Source observation and change

The global completion operation-ID counter used wrapping `AtomicU64::fetch_add`
despite the documented nonreuse contract. After exhaustion it could issue zero
and eventually alias a predecessor. It could also issue `u64::MAX`, which the
detached timer syscall reserves for submission failure. This is a source-proven
boundary defect, not attribution of an observed panic or evidence that a real
deployment exhausted 64-bit identities.

One boot-lifetime `OperationIds` counter now records the last issued identity.
Its checked atomic update issues `1..u64::MAX` exactly once; zero and the syscall
failure sentinel are excluded. The last successful update leaves the counter
at `u64::MAX - 1`. Every subsequent update rejects without changing it.
Relaxed ordering provides unique atomic admission; existing registry/source
publication still orders the corresponding payload and authority.

Capability-backed plain/buffer and event-watch submission, capability timers,
and detached plain/timer submission use this counter. They return the existing
`SubmitError::WouldBlock` / syscall submission-failure result on exhaustion,
before reserving new record/watch/timer admission, capability metadata or a
producer. Normal namespace validation, capacity checks and existing CQ backlog
reconciliation may precede that check. No exhaustion-specific cleanup or
timer enqueue is needed. Existing operations retain completion, cancellation
and close semantics; this rejection does not promise retry progress.

Later preparation rejection consumes its ID permanently. Namespace teardown,
replacement and numeric ASID reuse do not reset the global counter. There is
no reset, refund or free-list interface. Private implementation paths accept
an allocator reference so fixtures can exercise the same submission code with
independent boundary counters. Public entry points always use the static
boot-lifetime allocator; the live counter is never reseeded for testing.

The Markdown admission/ABI contracts and LaTeX completion chapter record the
same range, sentinel and permanent rejection semantics. No register or wire
format changes.

## Controls and evidence

Five standalone host tests exercise the production allocator directly:

- First issuance excludes zero, and discarded identities are not refunded.
- The final two usable IDs are issued once, followed by 128 stable rejections.
- An isolated sentinel-state counter cannot wrap to zero.
- Eight synchronized host threads issue 8,192 distinct contiguous identities.
- Eight synchronized threads race for 31 remaining IDs across 512 attempts;
  exactly those 31 are issued, without a duplicate or reserved value.

A separate temporary source copy replaces only the checked update with a
wrapping atomic increment. It fails three of the five tests: the boundary
returns the failure sentinel, sentinel state returns zero, and the concurrent
boundary admits 512 operations instead of 31. The unchanged production source
passes all five. The negative control is never installed in a kernel image.
The checked-in evidence records its exact replacement and concise failures.

The synchronous native fixture uses independent counters and the actual
submission implementations. A buffer-bearing capability receives the
penultimate usable ID; a detached operation receives the last. Sixteen rounds
reject all five paths (plain, watch, capability timer, detached plain and
detached timer), leaving the two live operations, original record charges,
watch/timer usage and CQ unchanged. Timer observer allocators must not run.
The original capability returns its exact buffer and the detached producer can
still publish its result. After namespace replacement, all five paths reject
with the same exhausted counter and fresh accounts remain empty. A public
submission using the untouched global counter succeeds.

A second local counter consumes its penultimate ID in injected timer-observer
allocation rejection, refunds unused record/event admission, then issues only
the last ID to a detached operation. Five later submissions reject. Each guest
therefore checks 90 exhaustion rejections plus the separate injected preparation
failure; no production global counter is advanced to the boundary.

The full host runner passes: **499 Rust tests passed, zero failed, four
ignored**, plus its signing/tooling/Python controls. Strict kernel Clippy passes
on both custom AArch64 and x86 targets with `-- -D warnings`. Workspace
`cargo fmt --all -- --check`, host-script syntax and `git diff --check` pass.

Runtime Rust and native fixtures were frozen before the following completed
runs. Both use four LPs, fresh storage, a 180-second budget, denied Rust warnings,
assembly-section checks and automatic all-LP failure capture:

| Profile | Isolated exhaustion control | Deferred summary | Runner/result |
| --- | --- | --- | --- |
| ARM / SMMUv3, no network | Passed at 1.601312 s; 90 rejected submissions | 17 passed, zero failed/pending at 11.196228 s | 0 / complete |
| x86 / Intel VT-d, no network | Passed at 1.214551 s; 90 rejected submissions | 15 passed, zero failed/pending at 14.822754 s | 0 / complete |

Each also completed 64 scheduled IPC claim/endpoint-close episodes with exact
exit watches. There was no failed native sample, panic or Raft persistence
fail-stop in these two completed runs. This focused cross-architecture check
does not repeat the prior AMD/security matrix or qualify integrated pressure.

Commands, with common environment `CATTEN_SKIP_EMBED_BUILD=1
CARGO_BUILD_RUSTFLAGS='-D warnings'`:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance operation-id-arm-final
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance operation-id-intel-final
```

Two earlier sandboxed launches (`operation-id-arm` and `operation-id-intel`)
failed before guest execution: QEMU could not bind its diagnostic Unix socket
(`Operation not permitted`). They used superseded development images and are
recorded separately as preflight failures, not passing or failing kernel tests.
The completed runs above use the corrected sentinel limit and required socket
access. During source review the initial checked implementation was corrected
to exclude `u64::MAX` before any successful native execution.

[Version-controlled evidence](evidence/2026-10-16-operation-id-exhaustion.json)
records reviewed Rust/host-runner hashes, exact negative-control replacement
and failures, separate preflight outcomes, kernel/boot-image/initial-disk/final
UART hashes, concise native observations and complete unchanged service-bundle
inventories. These inline results and the JSON are the published evidence;
temporary logs and ignored diagnostic directories are not required to read it.
Documentation/evidence was finalized after execution. Per-file hashes cover
the new untracked Rust modules as well as tracked changes; this is not a
full-tree or physical-platform attestation.

## Remaining boundary

This closes the recorded operation-ID wrap/sentinel gap. Capability-backed
record/timer preparation and staged unified authority, registry/backlog
metadata, outer guard/IRQ contexts, aggregate admission and integrated hostile
pressure/progress remain separate work. Existing historical IPC exit, stack,
persistence and NIC failures remain unattributed. Passing bounded identity
controls do not establish hardware quiescence, physical backing release or
whole-node fairness.
