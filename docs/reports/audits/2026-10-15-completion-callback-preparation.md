# Completion callback preparation and source disposal

Date: 2026-10-15. Baseline `6e0d8608`, following waiter source preparation.
Scoped R7-1/2 and C16/G1/G4 work; SEC-07/18 remain partial. Finding counts,
eighteen owner families and seven cleanup gates are unchanged. This batch
does not attribute historical IPC exit, stack, persistence or NIC failures.

## Observed boundaries

`observe_registered` checked the captured completion under `COMPLETIONS`, then
held its inner-state mutex while reserving a watch charge, allocating a lazy
callback list and allocating an entry. Failed storage and allocation rejection
could also destroy metadata beneath those guards. An already-terminal callback
already ran outside them; preparation needed to preserve that property without
opening a check-versus-one-shot-delivery race.

Namespace replacement/close also used either an `if let` scrutinee temporary
guard or an explicit registry guard while the detached `AsCompletions` value
was destroyed. Successful CQ replacement destroyed the old `CqState` inside
`replace_cq`, called under the registry. These payloads own source lists,
retained records, event/worker subscriptions, CQ backlog/rings and original
charges. Source destructors entering only their independent list lock did not
prove that heap metadata or callback captures could safely be destroyed under
the containing registry.

These are source observations, not historical failure attribution. Preparation,
successful detachment, failure cleanup and terminal producer cleanup are
different contexts; this batch qualifies the first two local boundaries.

## Captured callback preparation

The canonical `completion::{observe, observe_registered, CompletionObservation,
ObserveError}` paths remain, with implementation in `completion/callbacks.rs`.
There is one current admission path and no compatibility allocation branch.

Capture borrows the registry and inner lock briefly, retaining the exact
completion, namespace/root identity, original watch account, trusted captured
classification, terminal observation and any existing callback list. Cloning
those owners allocates no storage. Outside both guards, one `Preparing` owner
holds that capture plus the candidate list and admitted entry. Watch reservation
and list/entry allocation are fallible there. An already-terminal capture
prepares no entry or charge, preserving ready delivery under watch pressure.

Publication borrows the preparation. Under registry serialization it rechecks
namespace/root and account identity, retirement and exact completion identity;
under the inner lock it checks terminal state and either returns immediate
delivery or links the prepared entry. The inner lock remains shared with the
terminal transition, so a registration cannot append after one-shot delivery
has drained. A captured existing source must still match. Competing initial
captures with no list use the first published candidate; losing storage remains
with its original admitted owner until guard exit.

Ready/full/retired/replaced outcomes preserve unused preparation for post-guard
disposal. The public adapter releases that storage before an immediate callback
runs, so reentrant work does not inherit an unused watch charge or candidate.
Publication never invokes a callback or destroys losing storage under its local
guards. The returned observation retains the observer and cancellation token,
not an extra completion/root owner. Existing strong/weak record accounting and
callback-cancellation races remain in force.

Retirement can win during preparation; publication then rejects and unused
storage refunds only its original account after unlock. Namespace/account
replacement returns `UnknownAddressSpace`; a missing/replaced completion in
the same installation returns `UnknownCap`. Resource/allocator rejection stays
`ResourceLimit`. None completes the operation, observes its buffer or grants
permission to replay it.

## Complete successful detachment

`replace_address_space`, `open_namespace_with_cq` and `close_address_space`
now carry their complete old `AsCompletions` out of the registry scope.
Authority removal stays serialized with namespace publication/removal. Only
after unlock does its existing destructor cancel/discard subscriptions and
destroy source/payload fields.

`replace_cq` returns the complete old `CqState` after updating discarded detached
submission-slot accounting. `attach_cq` carries it out before destruction.
Ring/backlog backing, retained records, subscriptions, original budget owners
and charges remain together through that move. No teardown snapshot, new node,
registry, independent metadata ticket or recovery owner is added.

This changes the local disposal context, not the authority required to remove
a namespace or replace a queue. Mapped physical rings still belong to their
original root/mapping and require a quiescent consumer. A retained list identity,
an empty source, discarded callbacks or removal from the registry is not physical
quiescence. Old strong/weak completion references retain their original record
charge; they cannot refund a successor namespace's account. Discard invokes
no callback and fabricates no terminal result.

## Caller and cleanup inventory

| Entry | Boundary now qualified | Remaining qualification |
| --- | --- | --- |
| Numeric/captured completion observation | One exact capture; admitted preparation outside registry/inner guards; borrowed publication and post-guard unused disposal before immediate delivery. | Root-acceptance checks still enter the existing address-space/budget lookup under the registry. Arbitrary caller masks/guards and callback captures remain separate. |
| Namespace close and two replacement paths | Whole detached payload leaves `COMPLETIONS` after serialized authority removal, before source/field destruction. | Authority-record metadata, registry-map allocation/deallocation, outer lifecycle/root/IRQ contexts and producer quiescence remain separate. |
| Successful CQ replacement | Whole queue returned by the existing splice; detached slot counts update under serialization; payload disposal afterward. | `stage_cq` still allocates and can destroy partial state under the registry. Physical ring mapping/consumer policy is unchanged. |
| Terminal completion and observation setters | Existing behavior retained. | `complete_registered` calls `Completion::complete` under the registry; that method drops event/worker observations after releasing only its inner guard. Observation setters can replace fields under the inner guard. These are concrete next disposal contexts, not covered by this batch. |

This extends existing C16 metadata/context evidence without a new cleanup
family or physical owner. Caller IRQ policy and accepted operation ownership
are preserved; no forced IRQ enabling, ABI/wire/storage change or physical
retry/custody authority is added.

## Focused verification

The new composite native probe executes in the existing callback-admission
fixture:

- Actual list allocation/rejection reenters both registry and inner locks.
  Injected list-preparation failure refunds the original watch charge without
  installing a source. Existing entry-allocation rejection tests remain enabled.
- Two captures prepare independent first-list candidates. Actual heap-held
  publication links both callbacks through the winner without destroying the
  loser. Cancellation preserves the in-flight operation and owned buffer.
- A 128-entry full source rejects the prepared 129th entry while heap-held;
  its original charge survives until post-guard disposal. Domain capacity is
  256 in this fixture; the source ceiling still bounds linked entries, not all
  privately prepared entries.
- A terminal transition after preparation returns immediate delivery without
  linking or disposing storage beneath guards. The public late callback then
  reenters both guards exactly once; readiness does not take the result/buffer.
- Namespace replacement and retirement after capture reject heap-held
  publication. Old charges release after unlock, and a fresh namespace/record
  is left unchanged. Private/raw fixture namespaces are not physical root proof;
  the retirement control uses an actual generation-owned user root.
- Three public namespace actions (close, plain replacement and replacement
  with CQ) discard a real stored event observer whose destructor reenters the
  registry and original inner lock. Original record/buffer ownership remains
  charged while a retained reference survives; no notification/terminal result
  is fabricated.
- A private complete namespace stages two real admitted queues against its
  original account. The production replacement splice runs under the actual
  registry and heap guards, returning its old whole queue still charged. After
  unlock, disposal cancels its subscription and returns the original queue
  charge. Normal public CQ replacement is checked separately.

The entry IRQ state is preserved. No live sponsor/account/root is substituted
or saturated. These are forced orderings and real metadata owners, not
exhaustive cross-LP exploration, physical-OOM injection or evidence that every
outer cleanup context is safe. Existing 512-cycle cancellation/rearm,
128-entry notification, ready/reentrant callbacks, shared admission, scheduled
workers, real EL0 growth, wait/abort/reap and source-identity fixtures remain on.

Host validation: **494 Rust tests passed, 0 failed, 4 ignored**, plus signing,
boot/diagnostic/runtime-input/tooling/object-store checks and cluster-sign
self-tests. Strict Clippy passed for both custom kernel targets with
`--no-default-features --features acpi -- -D warnings`. New guard probes execute
in guests, not a host mock of the kernel. Formatting, whitespace and 462
local documentation-link checks passed.

## Native matrix

The planned matrix was fresh four-LP ARM/SMMUv3, Intel VT-d and AMD-Vi
no-network runs, followed by ARM and Intel security profiles, each bounded to
180 seconds with existing automatic all-LP failure capture. Runtime Rust and
native probes were frozen before execution. Every failed sample, if any, is
retained separately from passing repeats.

| Profile | Composite callback/disposal probe at | Exact close/exit episodes at | Final passed / failed / pending at |
| --- | --- | --- | --- |
| ARM/SMMUv3 | 1.582048 s | 64 at 8.484998 s | 17 / 0 / 0 at 10.465110 s |
| Intel VT-d | 1.267337 s | 64 at 19.132431 s | 15 / 0 / 0 at 20.255475 s |
| AMD-Vi | 1.359852 s | 64 at 14.019239 s | 15 / 0 / 0 at 14.815291 s |
| ARM security | 1.328588 s | 64 at 8.914838 s | 19 / 0 / 0 at 55.997496 s |
| Intel security | 1.231054 s | 64 at 13.754571 s | 17 / 0 / 0 at 61.444077 s |

All five runners exited **0** with diagnostic result `complete`. No panic or
Raft persistence fail-stop was observed, and there was no failed native sample
to replace or omit. The five composite probes include **15 registry-reentrant
namespace disposal controls and five heap-held complete CQ detachments**.
The matrix also completed **320 exact close/exit episodes, 20 first-entry
controls, 20 forced park controls and 15 real EL0 growth modes**. Every profile
reported 192 timer wakes retaining established LP affinity. These are bounded
fixture observations, not whole-node progress or historical-causation proof.

- **ARM/SMMUv3:** 1 boot Ready moves; sustained runtime counter 2 at 10.461874 s.
- **Intel VT-d:** 0 boot Ready moves; started-context runtime migration explicitly skipped under the existing x86 gate.
- **AMD-Vi:** 0 boot Ready moves; started-context runtime migration explicitly skipped under the existing x86 gate.
- **ARM security:** 2 boot Ready moves; sustained runtime counter 3 at 10.878702 s; policy generation 1 fenced at 33.700098 s; policy generation 2 fenced at 55.992174 s; 14,664 cancellation requests retired.
- **Intel security:** 0 boot Ready moves; started-context runtime migration explicitly skipped under the existing x86 gate; policy generation 1 fenced at 38.925104 s; policy generation 2 fenced at 61.425187 s; 13,992 cancellation requests retired.

SHA-256 identities of the executed kernel ELF and final UART bytes:

- **ARM/SMMUv3** (`aarch64-vkl8jmgy`):
  kernel `7d51d53cfbab5b14841ff04439961ce89f0212960eb27447708f203b380f0852`;
  UART `c5102bab0d67d74df1392255fe94778931a6142379e6a14d7841cd786148ec84`.
- **Intel VT-d** (`x86_64-ti9w_7ia`):
  kernel `fab26ae73d825cf613476a19314018925f1275a6f6064d3eb635ee5fd642f12b`;
  UART `aca478786103d80ec1905b9c249997d3754812f5ee15d02b7f763d1d59ac4743`.
- **AMD-Vi** (`x86_64-h5iypwwh`):
  kernel `fab26ae73d825cf613476a19314018925f1275a6f6064d3eb635ee5fd642f12b`;
  UART `baa6c513932aeafe849032a93273a70af9bb55aac40a729b32538844365e8bce`.
- **ARM security** (`aarch64-xr382o01`):
  kernel `ac34bcd7d2a062f9a68af14e9416da265e4ea552f4cc0af7ab61fc85615f7829`;
  UART `64ccd385fbb692c21adbaaa028b8511d014f2686e2446c20bf40485191e6c071`.
- **Intel security** (`x86_64-krclp3hc`):
  kernel `a790fb507248c7318dad472a4b11282f8719d3e0892665a9289a4b64b7d291d1`;
  UART `77705a397f3f8ab355e47f2d223c6af0e7c3724a126e21a6f1665cf79715a018`.

## Reproduction and artifact identity

Use `CATTEN_SKIP_EMBED_BUILD=1 CARGO_BUILD_RUSTFLAGS='-D warnings'` with:

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance callback-source-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance callback-source-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance callback-source-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance callback-source-arm-security
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --security-test --fresh-storage --timeout 180 --instance callback-source-x86-security
```

AMD reuses the Intel no-network kernel with `CATTEN_SKIP_KERNEL_BUILD=1`.
Security uses `CATTEN_ALLOW_SANDBOX_NETWORK=1`, HTTP/deploy host ports
19080/19081 for ARM and 19180/19181 for x86. Assembly-section runner checks
require executable, read-only trampolines. No assembly/userspace change or
linked-pair, outstanding-I/O or reset campaign is claimed.

[Version-controlled evidence](evidence/2026-10-15-completion-callback-preparation.json)
preserves changed-source and complete unchanged service-bundle hashes, actual
kernel/boot images, initial disk identities, final UART hashes and selected
observations. Dirty tracked-diff hashes omit then-untracked callback files;
they alone are not a complete source identity. Committed source and the
preserved artifacts anchor this record without temporary-log citations.

## Remaining work

Next: terminal event/worker-observer disposal and observation setters under
their containing registry/inner guards. Continue CQ/record preparation and
partial-error cleanup, authority metadata and other source/field teardown
contexts. Outer lifecycle/IRQ qualification, metadata/principal byte admission,
integrated hostile-pressure progress and the full R18 call graph remain open.
SEC-07/18 remain partial; no historical-failure cause, recovery registry or
physical retry owner is inferred.
