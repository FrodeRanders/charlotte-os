# Owned namespace loan cleanup and assembly permissions — 2026-10-06

Baseline: `74dec1c8`, following
[explicit endpoint close](2026-10-06-security-endpoint-close.md).
This advances SEC-18 for whole-domain IPC loans and corrects a newly reproduced
assembly-section defect, SEC-26. SEC-18 remains partial.

## SEC-18: whole-domain IPC loan cleanup

Root retirement previously retained lifecycle and IPC serialization while
revoking queued/delivered loans. Failure preserved ownership, but x86 mapped-loan
invalidation could still rendezvous under interrupt-masking guards.

`ClosingAddressSpace` now retains the exact root through three stages:

1. After existing operations drain, retire backing/capability sponsorship and
   device resources under lifecycle. Retire IPC record sponsorship and claim all
   owned endpoints before unlocking, fencing new incoming calls without reporting
   terminal endpoint closure. Device retirement still retains its existing guard
   and precedes memory loan cleanup.
2. Process admitted token storage one call at a time. `PreparedCancellation`
   borrows the closing owner and retains exact peer roots before IPC revalidation,
   bounded loan preparation and token claim. Detach, invalidation and scratch
   completion run outside lifecycle/IPC, object-registry and table guards. No
   namespace/token/capability snapshot is allocated.
3. Remove confirmed token/IPC authority, drain cleanup leases, and permanently
   seal cleanup admission before remaining memory cleanup and root detachment.
   Final root invalidation/destruction retains its existing detached owner.

Cleanup leases require the exact linear closing owner. They retain a live or
already-closing peer before its sealed backing-teardown phase; they never reopen
ordinary operation admission. Lifecycle precedes the table, never IPC. Both
generations are validated and inline counts checked for overflow. Successful
sealing requires zero counts; staged detachment additionally requires that seal.
An abandoned lease retains its count/root, and a dropped close retains its fence.
There is no scalar reconstruction, forced decrement or recovery bypass.

Self-calls borrow one owner for both roles. The permanent kernel root is handled
explicitly rather than attempting a user-root lease. Synthetic boot namespaces
can carry scalar calls; loans require captured user roots or permanent kernel
identity. Preparation revalidates the token and exact namespace handles under
IPC before claiming.

Each confirmed loan is recorded before consuming token/capability ownership.
Queued server-exit calls publish `REPLY_ENDPOINT_CLOSED`; delivered calls preserve
the existing `REPLY_CANCELLED` result. Caller-domain close removes its pending
record and queued ownership. Peer readiness notifications occur after unlock;
owned endpoint close watches/readiness remain pending until namespace drain.

A competing completion returns the closing owner as Pending for later polling,
without waiting under a guard. Preparation rejection restores only unstarted
receipts and completes admitted peer leases. Physical failure retains the failed
pin/token fence, queue, authority and original closing root/charges without
publishing that call's terminal result. Abandonment additionally retains the
claim and peer leases. Earlier confirmed calls/loans remain completed.

Production root retirement uses this composed path. The old serialized namespace
loan adapter is explicitly named `close_address_space_fixture` and confined to
raw kernel boot fixtures. Move/copy/result attachment cleanup and whole-domain
memory/device cleanup retain their existing serialization and remain separate
SEC-18 work. Immediate close still rejects initially busy roots before mutation;
a peer overlap after its closing fence can return busy with that fence retained.
Staged/supervisor close preserves its owner across pending polls.

## SEC-26: assembly section inheritance caused x86 boot failure

**Severity: medium, availability. Corrected.** The register-state probe selected
`.rodata` without restoring the assembler's prior section. Global assembly blocks
share section state within a codegen unit. This build placed
`reload_segment_regs` in read-only, non-executable data following the probe.
Kernel bootstrap then attempted to execute it before installing the IDT.

Two x86 boots reproduced the failure. QEMU's exception trace showed an
instruction-fetch page fault, error `0x11`, at `0xffffffff8027b421`, followed by
double/triple fault. The exact ELF classified `reload_segment_regs` as a read-only
data symbol at that address, outside `__text_end`. This was a layout-dependent
build defect exposed by the remediation's changed codegen layout. It does not
establish an application-triggered exploit or confidentiality/integrity impact.

Kernel and probe assembly now select/restore sections with
`.pushsection`/`.popsection` on both architectures. Inline x86 segment-reload and
syscall trampolines explicitly select executable text. Probe bytes remain in
their intended sections; executable permissions are not added to `.rodata`.

`scripts/check-kernel-asm-sections.py` checks the built ELF's section and load
segment permissions for required native entries and x86 ISR/dynamic gates. Boot
image reporting rejects a missing entry, writable/non-executable section, or
absence of an executable read-only `PT_LOAD`. The saved failing ELF is rejected
specifically for `reload_segment_regs`; corrected images pass, verifying 249 x86
native entries and the AArch64 vector table.

## Validation

New boot fixtures cover mapped read/write loans in queued and delivered calls,
caller/server domain retirement, scalar self-calls, both roots already in logical
cleanup, competing peer polls/replies, preparation rollback, partial physical
failure and abandoned token/namespace ownership. Kernel-caller fixtures check
queued/delivered mapped loans, result codes, restored authority, complete object
charge refunds and preservation of the permanent root. Four new direct host
tests cover closing-peer admission, permanent sealing, exact identity/overflow
rejection and abandoned lease retention; the slot suite now has 23 tests.

A deferred fixture closes a server with delivered and queued mapped loans from
two callers after secondary LPs are online. It verifies terminal results and
restored lender authority, then releases both callers. On x86 this exercises real
synchronous cross-LP shootdowns through the unlocked namespace path. These roots
have no application threads; it is not a concurrent hardware-walk stress test or
an injected rejected-IPI campaign.

| Check | Result |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed, including 23 direct slot tests, scratch ownership, runtime/services/protocol, signing and boot-result suites. |
| `scripts/build-catten-services.sh --embed` | Passed; rebuilt/staged/signed the AArch64 bundle. |
| x86 and AArch64 kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check` and `git diff --check`: passed. |
| ELF permissions | Saved failing ELF rejected; corrected x86/AArch64 kernels accepted. Each final guest runner checked its exact kernel before image creation. |
| Four-LP x86 guest | **15 passed, 0 failed, 0 pending**, including new namespace/kernel-peer boot fixtures and deferred mapped-loan cleanup. |
| AArch64 security guest | **19 passed, 0 failed, 0 pending**, including new namespace/kernel-peer fixtures. Both scoped probes reported `0xffff`, generations reached 1 and 2, cancellation traffic retired after 4,796 requests. TCP/IP clock/cycle progress held while clients retired after 1,648 and 1,762 requests. |

Final commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance namespace-security-20261006-kernel-peer --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18492 \
CATTEN_DEPLOY_HOST_PORT=17792 scripts/run-aarch64.sh --security-test \
  --instance namespace-security-20261006-kernel-peer --fresh-storage --timeout 180
```

Both runners rebuilt kernels and reused signed service bundles. An initial ARM
launch could not bind forwarding ports under the sandbox; approved isolated runs
followed. One intermediate ARM run timed out in firmware before kernel entry.
Intermediate guest failures also caught result-code/fixture assumptions, which
were corrected before final runs. No unrelated VM or storage was stopped/reset.

## Remaining scope

SEC-18 remains partial: whole-domain memory/device teardown, IPC move/copy/result
attachment cleanup, full CPU/DMA quiescence, recoverable shootdown failure and
physical-device reset remain open. The failure adapter abandons real prepared
pins; it does not simulate rejected hardware IPI delivery. Failed/abandoned
fixtures deliberately retain roots and charged backing.

SEC-07 translation/metadata admission, peer/management authentication, production
provisioning and security-time work remain open as previously documented.
