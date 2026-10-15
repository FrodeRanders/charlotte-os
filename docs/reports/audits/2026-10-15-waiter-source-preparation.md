# Waiter source preparation

Date: 2026-10-15. Baseline `c78d74f5`, following scheduler wait publication.
Scoped R7-1/2 and C16/G1/G4 work. SEC-07/18 remain partial; finding counts,
eighteen owner families and seven cleanup gates are unchanged. This batch
does not attribute historical IPC exit, stack, persistence or NIC failures.

## Source observations and correction

The preceding batch moved scheduler waker/source registration and cancellation
outside local LP/master-table guards. Its source inventory still found:

- Lazy `WaiterSource::register` admitted/allocated list backing while holding
  its initialization mutex.
- IPC endpoint-readiness and pending-call sources allocated waiter entries
  through `WaitSponsor::register` under the IPC registry read guard.
- Production `CqInterest` retained an exact list, but checked readiness under
  the registry and registered later without repeating that check there. Its
  fixture-only numeric `CqObservable` allocated under the registry. The IRQ
  fixture likewise registered while retaining its registry borrow.

Lazy initialization now captures any existing list under a short slot hold,
prepares a missing candidate outside it, then borrows that candidate for
first-winner publication. A competing initializer uses the installed list;
its losing candidate remains owned and charged until post-guard disposal.
Entry preparation follows outside the initialization mutex. No live source
list is replaced or resized, no allocation happens during publication, and
no new registry or initialization retry loop is introduced.

The first published candidate fixes list classification from its captured
sponsor. Losing ordinary/platform candidates retain their original charges
until deallocation; promotion does not reclassify existing storage. Concurrent
first use can temporarily consume multiple admitted candidates and can reject
under pressure. An initialized empty source remains charged if later entry
admission fails, as before. This is bounded admission, not fairness or an
essential-progress entitlement.

`PreparedWaitRegistration` contains the captured list and the existing charged
`PreparedRegistration`. It prepares the weak observer, original domain/node
charge and entry backing before entering a source registry. Borrowed publication
can move the entry into the list without destroying losing storage. Failure
or readiness leaves the complete unused entry with its caller until guard exit.
The shared ordinary registration adapter uses this same preparation; it does
not add an alternate uncharged or generation-free allocation path.

IPC receive/reply sources share exact-list capture and publication. After
preparation, the registry check matches the current list before checking
readability/result readiness, then either returns ready or links the entry.
Absent or replaced sources reject without affecting a successor. CQ does the
same with its existing `CqInterest`, checking both that captured interest and
the prepared entry against the current list. Retained strong list references
prevent address reuse during pointer-identity comparison. The original source
serialization closes the ready-check/link window; callback delivery still
occurs after producer guards leave.

The separate numeric-only `CqObservable` is removed. Its two fixture callers
use `capture_cq_interest`; the IRQ fixture captures a list before allocating.
Production and fixtures share prepared publication. No public wire/storage
grammar, runtime ABI, service bundle or application owner changes.
The IRQ fixture deliberately registers independently of the CQ's last-seen
cursor to force a waiting reader; it still prepares outside the registry.
Living waiter documentation also corrects its stale unchecked-CQ ABI wording
to the already-current checked ABI 88; this batch changes no ABI behavior.

## Ownership and context inventory

| Path | Qualified local boundary | Remaining context |
| --- | --- | --- |
| Lazy waiter list initialization | Source-slot capture/publication only clone admitted references; list allocation and losing-candidate disposal leave the mutex. | Existing caller masks/guards remain. A generic drain is a hint, not stored readiness; callers still need their durable condition/CAS recheck. |
| IPC receive/reply registration | Capture original list, allocate entry outside IPC, revalidate identity/readiness and link allocation-free. Ready/full/stale preparation disposes after the registry. | Endpoint/call construction, teardown, other observer metadata and outer callers remain separately qualified. |
| CQ waiter registration | Existing `CqInterest` and prepared entry qualify the same current source; ready check and linking share the registry. | The interest retains list identity, not ring backing or a physical root lease. Destruction/replacement still requires a quiescent consumer. |
| Direct retained-list registration | Shared preparation owns the original list/charge through independent list publication; no ASID lookup or callback. | This adapter cannot prove arbitrary callers have released their own outer guards. Other event-watch/callback allocation and field destruction remain separate. |

This is subscription metadata, not a new cleanup/recovery family. A prepared
entry is unstarted storage whose unused release cancels no accepted operation.
A linked token retains its existing cancellation policy. Readiness grants no
returned capability, borrow release, root quiescence, hardware completion,
operation replay or physical retry authority. Pending-call results remain
unobserved until their existing explicit observation. Caller IRQ policy and
original deadlines are unchanged; no path enables interrupts beneath a mask.

## Focused native verification

Three probes execute in the existing serialized boot fixtures:

1. **Lazy source:** allocation and injected preparation rejection reenter the
   actual initialization mutex. Two real admitted candidates straddle sponsor
   promotion; heap-held first-winner/loser publication retains both allocations.
   Post-guard losing disposal refunds the original ordinary charge, and final
   release restores the exact list-account baseline. Live and fresh retired
   sponsor cases reject without entry leakage. A pre-publication drain is
   explicitly treated as an empty hint, not completion or durable readiness.
2. **CQ:** real prepared-entry success, full-source rejection, producer-ready
   recheck and numeric ASID/CQ replacement rejection run while the actual heap
   is held. Full/ready/stale entries retain the private original sponsor charge
   until post-guard disposal. Callbacks can reenter the registry. Replacement
   is a raw isolated namespace fixture, not a physical address-space proof.
3. **IPC:** real endpoint and reply-list publication run heap-held. Message
   readiness, a completed unobserved reply, full source, wrong captured list
   and closed source preserve their intended result and owning charge. A wrong
   list rejects even when the target is readable. Callbacks reenter IPC after
   producer guard exit, while reply readiness does not observe its resources.

The probes use private fixture sponsorship and normal node admission; no live
shared account is saturated or replaced. Entry quotas, detached-batch lifetime,
512-cycle cancellation/rearm, source close, loans, scheduled waits, watchdogs,
abort/reap and the previous early/stale-waker controls remain enabled. These
forced orderings are not exhaustive cross-LP exploration or physical-OOM
injection. The new lazy-source rejection adapter exercises that preparation
boundary; existing list-allocation rejection separately verifies charge rollback.

Host validation: **494 Rust tests passed, 0 failed, 4 ignored**, plus the
signing/boot/diagnostic/runtime-input/tooling/object-store checks and cluster-sign
self-tests. New source probes run in the guests rather than a host mock of
the freestanding scheduler. Strict Clippy passed on both custom kernel targets
with `--no-default-features --features acpi -- -D warnings`. Formatting,
whitespace and 531 local documentation-link checks passed.

## Native matrix

The planned matrix was fresh four-LP ARM/SMMUv3, Intel VT-d and AMD-Vi
no-network runs, followed by ARM and Intel security profiles. Each retained a
180-second bound and the existing automatic all-LP failure capture. Runtime
Rust and native probes were frozen before execution. All samples, including
any failure, are to be retained separately from passing repeats.

| Profile | List / IPC / CQ probes at (s) | Exact close/exit episodes at | Final passed / failed / pending at |
| --- | --- | --- | --- |
| ARM/SMMUv3 | 1.339600 / 3.465976 / 6.562586 | 64 at 8.102010 s | 17 / 0 / 0 at 10.069446 s |
| Intel VT-d | 1.297407 / 6.706076 / 17.173269 | 64 at 18.690463 s | 15 / 0 / 0 at 19.726428 s |
| AMD-Vi | 1.286501 / 3.806062 / 14.496966 | 64 at 16.081533 s | 15 / 0 / 0 at 17.126710 s |
| ARM security | 1.280236 / 3.653098 / 7.256270 | 64 at 8.955308 s | 19 / 0 / 0 at 56.103744 s |
| Intel security | 1.231625 / 3.466119 / 12.279334 | 64 at 14.256229 s | 17 / 0 / 0 at 62.031130 s |

All five runners exited **0** with diagnostic result `complete`. No panic or
Raft persistence fail-stop was observed; there was no failed native sample to
replace or omit. Total: **15 new probe executions, 320 exact close/exit episodes,
20 first-entry controls, 20 forced Ready-producer park controls and 15 real EL0
growth modes**. Every profile reported 192 timer wakes retaining established
LP affinity. These bounded results do not prove whole-node progress or explain
historical intermittent failures.

- **ARM/SMMUv3:** 0 boot Ready moves; sustained runtime counter 1 at 10.062506 s.
- **Intel VT-d:** 0 boot Ready moves; started-context runtime migration explicitly skipped under the existing x86 gate.
- **AMD-Vi:** 1 boot Ready moves; started-context runtime migration explicitly skipped under the existing x86 gate.
- **ARM security:** 0 boot Ready moves; sustained runtime counter 1 at 11.009078 s; policy generation 1 fenced at 33.816410 s; policy generation 2 fenced at 56.095162 s; 14,476 cancellation requests retired.
- **Intel security:** 0 boot Ready moves; started-context runtime migration explicitly skipped under the existing x86 gate; policy generation 1 fenced at 39.461167 s; policy generation 2 fenced at 62.013073 s; 14,052 cancellation requests retired.

SHA-256 identities of the executed kernel ELF and final UART bytes:

- **ARM/SMMUv3** (`aarch64-ilvf1rl4`):
  kernel `bff4e5fefc6c0cca7020c737a41c86cfce57c0fc3fd676191444835a7256cb1e`;
  UART `eee2b7bb7fbd79fa52ecfb9c44016ffb9a8abd84fc22aaf954eac77b082c5598`.
- **Intel VT-d** (`x86_64-czxbjala`):
  kernel `9dfc5d3872648a5aa566bd726a18982573fca7e942680ad50def84f7ed78ceb6`;
  UART `59c820ea7a813c5ca451ae69196b4c884e5ab997a08759efd35f38943124eaed`.
- **AMD-Vi** (`x86_64-a512xrt1`):
  kernel `9dfc5d3872648a5aa566bd726a18982573fca7e942680ad50def84f7ed78ceb6`;
  UART `dae6105ef9534e79812d10fba7ffa5f794fb2b437578f71424cbc2034321c498`.
- **ARM security** (`aarch64-7pmrifdg`):
  kernel `e5327c1ddd2d9de7d5becbb89b266a6c481eab7ea3b45ea7a4f265bd7d05cf0a`;
  UART `24284bf1543e002b5175c4e2baf526f6838124d4262e6f2d687716e313d03e35`.
- **Intel security** (`x86_64-lmax3rwb`):
  kernel `d760aa9cbda53d086cff5eb8ca2edd8634b121bbbf9b9a34be30a40a2a695c97`;
  UART `5f2d91e75a08362eaf6c9ce3d5cc50ee33e6d3e5d3954f7670ec0f5e43b31f83`.

## Reproduction and artifact identity

Use `CATTEN_SKIP_EMBED_BUILD=1 CARGO_BUILD_RUSTFLAGS='-D warnings'` with:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance source-waiter-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance source-waiter-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance source-waiter-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance source-waiter-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance source-waiter-x86-security
```

AMD reuses the Intel no-network kernel with `CATTEN_SKIP_KERNEL_BUILD=1`.
Security profiles use `CATTEN_ALLOW_SANDBOX_NETWORK=1`, HTTP/deploy host ports
19080/19081 for ARM and 19180/19181 for x86. Runner checks require executable,
read-only native assembly sections. There are no assembly or userspace changes;
no linked-pair, reset or outstanding-I/O campaign is claimed.

The [version-controlled evidence](evidence/2026-10-15-waiter-source-preparation.json)
records reviewed source, unchanged complete service bundles, actual kernel and
boot-image identities, initial disks, final UART hashes and selected observations.
Run manifests' dirty tracked-diff hashes omit then-untracked new Rust files;
those hashes alone are not complete source identities. Committed source and
the preserved artifact hashes anchor the result without temporary-log evidence.

## Remaining work

Next review: source teardown and implicit observer/list/weak-reference
destruction under registry or timer guards, then remaining callback/watch and
record preparation contexts. Concrete source observations in the
[completion registry](../../../crates/catten/src/completion/mod.rs) include
`observe_registered`, which prepares lazy callback lists/entries beneath both
registry and completion-inner holds, and `CqState::drop`/`AsCompletions::drop`,
which discard waiter/callback metadata. Their containing callers and implicit
field destruction need qualification; this report does not declare every
invocation unsafe or authorize detaching ring/root backing. IPC source Drop
and timer-owner contexts remain part of that inventory.

Outer scheduler/park masks, broader metadata and
principal byte admission, integrated hostile-pressure progress and the full
R18 call graph remain open. There is no new recovery registry, physical retry
owner or historical-failure attribution. SEC-07/18 remain partial.
