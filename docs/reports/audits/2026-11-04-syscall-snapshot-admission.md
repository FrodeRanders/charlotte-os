# Six-site syscall snapshot admission — Q1

Date: 2026-11-04. Base: `32109149`. This checkpoint accepts the bounded Q1
package in the [SEC-07/18 closure plan](../../reference/security-closure-plan.md).
It leaves seven packages and all four aggregate gates I/C/P/M open; package
counts are not a weighted readiness percentage. SEC-07/18 remain partial.

## Result and capacity decision

Upgrade ELF, scoped image/descriptor, operational pickup/plaintext and shutdown
envelope storage now share `ByteAllocator<SnapshotBuffers>` and the existing
checked copy boundary. Class comes from the exact authenticated executing root,
not a manifest, actor name or later reusable ASID. Scoped and operational
parents retain all private prefixes. Actual free precedes original-class refund.
Plaintext allocates once, rejects reallocation after initialization and erases
initialized/spare capacity before backend free.

Default snapshot bytes rise from 3/4 to **12/16 MiB ordinary/total**; frame-buffer
counts remain 6144/8192. Primitive bytes contribute zero frame-buffer counts.
The maximum scoped pair needs 4194304 + 3584 bytes. Two maximum pickup/plaintext
pairs need 2 × (4269180 + 65536); with the former 3 MiB allowance, the total is
**11815160 bytes**, leaving 767752 ordinary bytes before other retained usage.
The native control allocates all these byte buffers simultaneously.

The cost is another 9/12 MiB permitted heap retention, with no preallocation.
The minimum configured heap envelope is 8 MiB initial + 64 MiB growth reserve;
this family may consume 16 MiB while other families and allocator overhead
compete for that envelope. Preserving existing protocol maxima avoids parser
splitting and extra lifetime machinery. This is a reviewed local capacity
increase, not the Q4 aggregate capacity plan or a Q7 progress guarantee. The
23-pool catalog, statistics version 10, SDK and signed service bundles remain
unchanged.

## Acceptance evidence

The [final evidence](evidence/2026-11-04-syscall-snapshot-admission.json) binds all
ten frozen changed/new source and host-script hashes, relevant dependencies,
commands, kernels, boot/disk/service artifacts, results and inline diagnostics.
All final campaigns use four CPUs, fresh storage and a 240-second guest bound:

| Target | Passed | Failed / pending |
| --- | --- | --- |
| AArch64 / SMMUv3 | 19 | 0 / 0 |
| x86 / Intel VT-d | 17 | 0 / 0 |
| x86 / AMD-Vi | 17 | 0 / 0 |

The full host suite, strict Clippy test/minimal checks for both architectures,
formatting and diff checks pass. The new optimized owner harness passes 46 tests
(four new controls plus 42 shared controls), and deliberately removing wiping
fails its live-storage tracer. Each native runner enforces the boot profile and
executable/read-only assembly-section checks.

Focused controls establish:

- Actual maximum scoped copies and two maximum pickup/plaintext lifetimes plus
  3 MiB actual private mixed storage. That mixed storage represents the old frame
  allowance; separate existing frame-catalog tests exhaust real 12/16 MiB byte
  and 6144/8192 count limits.
- Ordinary/trusted immutable classification through promotion, source-root close,
  ASID reuse and final disposal. Stale captures reject successor-generation caps;
  copied bytes retain original charges independently of the closed source root.
- Ordinary/total quota denial under a held heap guard before allocation. An
  allocated image prefix is disposed after descriptor quota/error rejection.
  Counter reservations and the second-copy error are explicit adapters.
- A real host backend allocation rejection after a live admitted package prefix;
  only unused candidate admission refunds until the complete prefix is dropped.
- Full initialized/truncated plaintext capacity is zero while still live at the
  backend deallocation boundary, with its original charge still held. Refund
  follows actual free. No freed-memory read is used.
- All four public handlers retain generic failure statuses and source-cap
  consumption on invalid content and quota denial; no scoped receipt publishes.
  Existing signature/freshness/parser controls and native scoped-launch security
  remain enabled. The new maximum-copy fixture uses byte contents, not a
  maximum-sized authenticated connector deployment.

## Failed preflight and limits

Both initial ARM and Intel runs failed the same new fixture assertion: expected
11815304, actual 11815160. The copies had allocated successfully. The planning
arithmetic overstated maximum pickup by 72 bytes, hence 144 bytes for two pairs.
The [separate failed evidence](evidence/2026-11-04-syscall-snapshot-preflight.json)
retains source hashes, artifact identities, results and panic diagnostics.
Only that native arithmetic expectation changed before the final source freeze;
the sizing documentation was corrected in place. Passing repeats do not erase
those failures or explain the older Intel retirement-pin/ARM exit-watch incidents.

These are serialized native and tracing-host qualifications, not hostile
concurrent EL0 pressure, arbitrary concurrent root retirement, general native
heap OOM or abort erasure. The executing thread retains the production syscall
root; the class token grants no root/cleanup authority. Byte owners have no
active physical cleanup, but their surrounding supervisor transactions and
broader caller/IRQ/Drop contexts remain Q2/Q5. Persistent `service::store` image
snapshots stay outside Q1. Original-input, crypto/compiler copies and forced
termination retain separate SEC-15 limits. Aggregate original-principal/heap
coverage, retained-owner recovery, saturated management/timer/teardown progress
and the combined Q8 matrix remain open.
