# Retained executing-thread ownership for stack growth

Date: 2026-10-15. Baseline `3db606bc`, following the committed exit-watch
preparation batch. Scoped C16/C17 and G1/G2 work; SEC-07/18 remain partial.
This corrects a source-reviewed context gap, not an attributed historical panic.

## Gap and containing owner

`grow_current_user_stack` held the master thread-table write guard while growing
the current user stack. That included physical allocation/zeroing, page-table
mapping and ordinary rejected-map rollback. Releasing only the inner mapping
guard did not remove the enclosing scheduler guard.

Taking a mutable pointer to the boxed context and then dropping the table guard
would not be a sufficient correction: preemption still uses that context, and
remote abort could otherwise retire its containing stack. The correction keeps
the original thread/context/user/kernel stack pair in its existing master-table
owner and retains that exact executing lifetime through the operation.

The existing abort-sweep executor is shared as `ThreadExecution`, with the inline
field renamed to `operation_executor_lp` throughout scheduler admission, wake,
abort, migration and tests. No second fence, per-instance registry or callback
registry is added. Admission captures current TID/generation and LP, rejects a
pending abort or nested owner, and precedes stack-plan capture. While held,
abort requests remain recorded but cannot make that lifetime retirement-ready;
placement preserves its LP and migration rejects it. Abandonment leaves the
same inline fence installed. The existing whole-domain abort and concurrent
executor tests continue to use this shared mechanism.

## Growth transaction

`CurrentGrowth` retains execution ownership and one `Growth` episode:

1. Under a short thread-table guard, qualify TID/generation and caller ASID,
   capture the original root/slot/capacity/committed prefix, and fence that
   stack's existing slot. Its original root lease and complete maximum
   user-plus-kernel reservation remain in the thread. No context is detached.
2. Leave the thread table before allocation, zeroing, mapping or rollback.
   The episode owns provisional backing and exact new-page progress. Mapping
   validates the captured root generation under the address-space table; its
   guard leaves before physical rollback. Work starts once, and an interrupted
   mapping cannot be interpreted as rejection or replayed.
3. Under an IRQ-preserving handback mask and short thread-table guard, revalidate
   the captured lifetime/root, slot base/capacity and original committed prefix.
   Only a completed episode with no provisional frame or unconfirmed mapping
   can publish its new committed count. Leave the table before consuming the
   execution fence. A pending abort becomes retirement-ready afterward.

During growth, other readers see the old committed-prefix observation. The
complete original stack and charge remain present. Sampling still uses that
same context; no mutable context borrow or raw pointer escapes the table guard.
The count is published once at handback and is not a quiescence proof.

| Phase/outcome | Ownership and continuation |
| --- | --- |
| Invalid range, no user context, rejected execution admission | No growth episode is published. Ordinary unused execution ownership is explicitly completed after local guards. |
| Allocation rejection or confirmed rejected-map rollback | Completed handback preserves the exact old prefix and clears this episode's temporary slot fence. No page/request replay occurs inside the operation. |
| Success or known partial mapped prefix | Handback records that prefix before execution release. The fault adapter receives the same optional low-address result as before; a later fault is a separate captured operation. |
| Physical rollback rejection | The single attempted provisional release is not resubmitted. Completed handback keeps the original slot/root/reservation uncertain and records the known committed prefix. Ordinary thread retirement retains admission if cleanup cannot confirm it. |
| Abandonment, interrupted publication or rejected handback | Provisional backing is quarantined without allocator/registry/logging work. The original slot and executing-thread fences remain retained. There is no retry owner, custody ticket or force-clear operation. |

The physical growth reserve, initial-page floor, configured capacity and original
sponsorship are unchanged. No live timer sponsor is replaced, no IRQs are enabled
under an unknown caller mask, and no polling/readiness contract is introduced.
Address-space walker allocation and its own serialization, heap/image faults,
constructor/loader callers and arbitrary callback/destructor contexts remain
separate G1/R7 work. This is not a general kernel-operation cancellation proof.

## Regression evidence

The existing collision/partial-prefix, allocation rejection, guarded abandonment
and physical-rejection fixtures now use the same `Growth` phase owner. They
retain the existing sixteen initial/growth roots/slots, 272 reservation pages
(136 ordinary) and ten provisional frames; no historical retained owner is
recovered. A completed failed physical rollback is handed back while lifecycle,
both address-space tables, the physical allocator and original stack-admission
pool are held. Handback performs only captured scalar mutation and preserves
the original fence. The separate six failed stack cases remain enabled.

New serialized checks reject unfinished or foreign-root handback without
mutating either parent. Both completed original owners restore usability and
their original charges. These are never-scheduled fixture owners, not authority
to extract a running context.

Three new real EL0 fixtures run on both architectures:

- A four-page stack begins with one committed page. A store three pages below
  its aligned top faults and returns with three committed pages. The user
  publishes a sentinel; the verifier checks exact thread/root identity, the
  committed count and completed execution fence before releasing its bounded
  fixture gate. Normal exact-root teardown then succeeds.
- The same fault records a self-abort request while growth owns the execution
  fence. The lifetime stays owned through work and committed-prefix handback;
  normal scheduled retirement and root close succeed afterward.
- A trusted foreign leaf at the next growth page forces actual map rejection
  and physical rollback. The original one-page prefix remains; normal fault
  containment/retirement closes the root afterward.

An exact-root fixture hook is armed before thread publication. It reenters the
master table immediately before actual allocation, mapping and physical rollback;
it does not assume global try-lock availability. The pre-handback check verifies
the original root/generation, committed count, execution fence and deferred
abort state. Enabled callers yield eight times while retained; masked callers
do not yield or enable IRQs. The x86 fault entry supplies enabled IRQ context,
whereas the observed ARM fault entry stays masked. These differences are
reported separately. Fixture gates use the existing ten-second bounded
observation policy; they are not production resource waits.

User instruction payloads remain scoped `.pushsection`/`.popsection` read-only
byte blobs copied into admitted user-code pages. Native kernel trampolines keep
their executable/read-only contract. No syscall/runtime ABI, signed service,
storage format or production admission classification changes.

## Bounded execution

Plan: fresh four-LP ARM/SMMUv3, Intel VT-d and AMD-Vi no-network guests, each
with a 180-second host window; ARM/x86 security profiles at the same bounds
after those pass. Automatic failure capture remains enabled. All five runners
return 0, with no kernel panic, Raft fail-stop or authoritative failed/pending
result. No failed native sample is replaced by a passing repeat.

The first three guests pass:

| Guest | Handback rejection probes | Three real EL0 growth modes complete | Authoritative result |
| --- | --- | --- | --- |
| ARM/SMMUv3 | 1.191952 s | 6.344860 s; zero forced yields in each masked mode | **17/0/0**, 10.191320 s, bitmap `0x1ffff` |
| Intel VT-d | 1.224729 s | 18.223335 s; eight yields per mode | **15/0/0**, 21.746734 s, bitmap `0x1bbff` |
| AMD-Vi | 1.446489 s | 18.295803 s; eight yields per mode | **15/0/0**, 21.772565 s, bitmap `0x1bbff` |
| ARM/SMMUv3 security | 1.260928 s | 7.924456 s; zero forced yields per masked mode | **19/0/0**, 56.843622 s, bitmap `0x2003ffff` |
| Intel VT-d security | 1.135271 s | 13.206517 s; eight yields per mode | **17/0/0**, 61.783691 s, bitmap `0x2003bbff` |

Thus **15 real EL0 growth episodes** complete, including five successful
growth/return controls, five retained self-abort episodes and five actual
foreign-leaf rejection/rollback episodes. The three x86 guests execute **72
qualified yields** across these modes; the two ARM guests supply masked-context
coverage, not forced-preemption evidence. The original concurrent abort-sweep
fixtures also remain enabled under the shared execution type.

All five guests complete their existing 64 IPC close/exit episodes, four
first-entry controls and four forced Ready-producer controls: **320 / 20 / 20**.
No historical exit-watch/stack-lease timeout recurs. ARM security passes scoped
policy generations 1/2 at 34.537548 / 56.837218 s and retires 14,860 concurrent
cancellation requests. x86 passes at 39.517172 / 61.770632 s and retires 14,204
requests. Independent-root/raw-frame denial and existing exact-generation
observation remain part of those authoritative suites. No linked deployment
pair or new device-outstanding-I/O campaign is claimed in this batch.

## Checks and artifacts

The full host suite passes **488 Rust tests, zero failed, four ignored**, plus
existing boundary checks and thirteen automatic-diagnostics tests. New growth
fixtures are native kernel execution, not added host test counts. Strict kernel
Clippy passes both custom targets with `--no-default-features --features acpi
-- -D warnings` and their staged bundle variables. Workspace rustfmt, whitespace
and local documentation targets pass. Native runners verify executable/read-only
assembly sections. Early fixture checks caught an address-space generation
atomic typed as `u64` rather than `usize`, and Clippy required `expect_err`;
these were corrected before native execution and final checks.

Environment: `CATTEN_SKIP_EMBED_BUILD=1 CARGO_BUILD_RUSTFLAGS='-D warnings'`.
Both no-network kernels are rebuilt. Intel/AMD share that x86 profile after a
`--build-only` preparation, using `CATTEN_SKIP_KERNEL_BUILD=1` for execution.
Security profiles are rebuilt separately with their current signed fixture
inputs and `CATTEN_ALLOW_SANDBOX_NETWORK=1`. ARM security uses host HTTP/deployment
ports 18680/18681; x86 uses 18780/18781. All stores are fresh and instance-local.
Service source/signatures are unchanged, reusing the signed bundles identified
in the [persistence diagnostics batch](2026-10-15-raft-persistence-diagnostics.md#artifact-identity).
No new service build/Clippy result is claimed.

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance growth-owner-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance growth-owner-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance growth-owner-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance growth-owner-arm-security
./scripts/run-x86_64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance growth-owner-x86-security
```

| Executed artifact | SHA-256 |
| --- | --- |
| ARM no-network kernel | `011ada380bec3205e3d991e15da0fdeffe137668c6aa4710c4add9efb454b5a1` |
| Intel/AMD no-network kernel | `2669dbcf5a06113f4256f1f7a8a7a6b5f488e91e3314c27c6c915ab1401880de` |
| ARM security kernel | `421624a5661d23922f13c6a7d644006836b63da2ab8ad8b70731a6a43c6f0c82` |
| x86 security kernel | `f21aaa87efb8d3c587492b2993b1d48804c1fe342db2f600a9a9ca7f07be2b0a` |
| ARM no-network final UART | `52d9d35b4af4b18861f60ae1282cf712c9e9e7a5da1848b899b3cc57bf6b7ee4` |
| Intel no-network final UART | `91a447053dc5259c74f8a753be6e9c4f7dcce4c97033b96be1668c87d2534763` |
| AMD no-network final UART | `97e5d85e9ca5522f950d40d7528bcf1d61a971057b181b7b745e04ce85dc3873` |
| ARM security final UART | `33a416695f6c79ab97bc34fec20dc1113600bab04b09958d8db04b4377374823` |
| x86 security final UART | `253e86a230e8160a67810657207d694b76ca6f7f7beb5f9d05eb9e2e2b0255ae` |

Run manifests retain the actual boot-image ELF and input identities. Relevant
results/qualifications are inline here, without dependency on temporary logs
or ignored artifacts. Passing runs do not attach LLDB and are not snapshots
of a historical failed episode.

## Disposition and remaining work

The enclosing thread-table allocation/rollback boundary is corrected with
retained execution ownership. It does not explain or erase historical IPC
exit-watch, stack-lease, persistence or NIC failures. No original failure is
claimed reproduced. Full outer-context/destructor qualification, kernel-arena
and broader walker metadata, general heap/aggregate-principal limits,
outstanding-I/O recovery and integrated pressure/progress remain open.
The eighteen owner families, seven cross-category gates and numbered SEC
accounting are unchanged. No physical cleanup, replay or restart authority
follows from the new observations.
