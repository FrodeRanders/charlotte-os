# Q4 diagnostic buffer admission

Baseline `2cb0a539`, following the shared-heap capacity assessment. This batch
corrects diagnostic thread/domain/encoding scratch admission. **Q4 remains in
progress**: root frame-tracking capacity, wider metadata/principal coverage,
allocator overhead and joint heap planning are still required. Q5–Q8 and all
aggregate gates remain open. The [qualification evidence](evidence/2026-11-05-q4-diagnostic-buffers.json)
records frozen sources, artifacts, relevant diagnostics and preserved failures.

## Correction and scope

Previously each diagnostic request prepared fallible buffers within one 1 MiB
working bound but used the global allocator without aggregate or principal byte
admission. The syscall now supplies a captured allocator to the same snapshot
budget and capture helpers. The thread snapshot, domain snapshot and encoded
bytes retain that allocator through their complete allocation lifetimes.
Wire version 11, self/observer visibility, complete-source rejection and the
existing `(0, 0)` unavailable result stay unchanged.

`ByteAllocator` accepts an optional typed sponsorship adapter. Its existing
family-only callers keep their contracts; diagnostic buffers use both the
existing `SnapshotBuffers` family and the verified principal. Both reservations
precede backing allocation, no locks are held across the next counter or
allocator, and either rejection refunds only its unused prefix. Default grow
admits the complete new allocation while the old one remains charged. Actual
deallocation precedes family and principal refunds. Allocator clone/move alone
reserves and refunds nothing. No second principal registry or metadata pool is
introduced.

The syscall captures trusted class and the root's prebound signed principal
under one root-table hold before source observation or allocation. Later root
retirement, promotion and reusable ASIDs cannot redirect an allocation's
sponsor. The existing principal record is boot-lifetime; holding this token
requires no root lookup at refund and grants no cleanup authority. Unidentified
kernel/raw fixture roots keep family admission only; verified application launch
prebinds the principal before execution. `SystemObserver` widens observation,
not the caller's backing entitlement.

The new configurable principal kernel-buffer ceiling is **16 MiB**, currently
used only by diagnostic scratch. It admits sixteen 1 MiB maximum requests in
isolation and allocates no backing in advance. The existing shared 16/12 MiB
total/ordinary snapshot family can bind earlier, alongside frame snapshots and
primitive inputs. Neither the per-request bound nor family/heap limits are
reduced. The principal record's added two-u64 budget is included automatically
in its existing requested-layout node admission. This supplies no general
allocator-overhead or joint capacity proof; the 96/128 MiB heap-reserve options
from the preceding baseline remain open.

Scratch elements are `Copy` observations. Preparation and disposal occur after
thread/usage/physical/read-only observation guards have left. Failure drops the
whole local prefix; it contains no active capability or physical cleanup owner.
The final response object retains its existing independent admission/close path.
General syscall IRQ latency and other active containing destructors remain Q5
work. No physical release, recovery policy or root-quiescence condition changes.

## Controls and qualification

The production allocator host tracer covers full old/candidate sponsor overlap,
principal denial preserving pointer/capacity/prefix, family rejection rolling
back principal admission, backend rejection, zero storage and deallocation
before refund. Existing byte/family, bounded-snapshot and policy tests remain.
Policy controls reject an undersized or out-of-range principal buffer setting.

Native serialized boot controls allocate real maximum-request and sibling-root
buffers. A private principal ceiling produces 128 denials while another sponsor
allocates successfully. Existing ordinary buffers stay ordinary after promotion;
new trusted buffers may use family reserve but retain principal admission.
Root close/ASID reuse preserves the first sponsor until its buffer is freed.
Counter-only family saturation tests rollback and recovery; this is explicitly
not a real heap-pressure or fairness campaign. The existing syscall controls
also reconcile snapshot bytes across all 256 preparation/publication failures.

| Final frozen-source qualification | Result |
| --- | --- |
| Formatting and full host suite | Passed |
| Strict ARM/x86 test and minimal profiles | All four passed |
| Four-CPU ARM SMMUv3 security | 19/19, unchanged 240-second budget |
| Four-CPU Intel VT-d and AMD-Vi security | 17/17 each, unchanged 240-second budgets |
| Four-CPU ARM and Intel minimal operational boot | Both passed, 90-second budgets |

All five native campaigns used the exact previously qualified signed Q3 service
bundle identities; no userspace source changed. The current kernel sources and
regression controls were frozen for final qualification. Diagnostic byte admission
is accepted in this scope; no package-wide or pressure gate is accepted.

## Preserved failed attempts

The first new host denial control requested one extra element with length two
and capacity three. That correctly did not grow and the assertion failed. The
fixture now requests two extra elements, forcing the intended allocation. No
production fix was needed for that failure. The preserved diagnostic identifies
the assertion separately from the final passing control.

Initial sandboxed attempts also encountered a registry DNS denial and four
OpenSSL interop tests failing to open local sockets (`Operation not permitted`).
The full suite was rerun outside the sandbox with cached dependencies. These
environment failures are separate from both the fixture correction and native
campaign results; no deadline was extended to obtain a pass.

Earlier ARM display, Intel retirement-pin and ARM exit-watch incidents remain
unresolved. This batch neither explains them nor changes finding dispositions,
production trust or physical-platform qualification.
