# Resource and cleanup closure inventory checkpoint

Baseline: `b15250b6`, 2026-11-04. No kernel behavior, resource limits, finding
dispositions or production-trust setting changes in this checkpoint.

The result is a reproducible review census and a
[finite eight-package closure plan](../../reference/security-closure-plan.md).
The next implementation batch is Q1: five syscall snapshot inputs plus one
decrypted-profile buffer. They share primitive-byte lifetime mechanics and one
final qualification campaign. Store caching and active supervisor transactions
are separated into Q2 because their ownership/lifetime changes are consequential.

## Captured evidence

[Version-controlled inventory](evidence/2026-11-04-resource-inventory.json)
contains the 592 kernel Rust source hashes, input MIR hashes, compiler pin,
extractor identity, source/body origins, candidate callees/types and profile
membership. The full MIR captures contain embedded data and are not needed to
read the selected review records; no temporary log is a durable evidence link.

| Profile | Function bodies | Call occurrences | Selected occurrences | Coalesced records in profile |
| --- | --- | --- | --- | --- |
| ARM, headless `acpi`, no kernel tests | 5491 | 24439 | 3152 | 2549 |
| x86, headless `acpi`, no kernel tests | 5730 | 25415 | 3211 | 2614 |

The union has **2799 coalesced review records**, all with source spans and
containing-body origins. Calls include constant/static initializers; the
function count is listed separately. Records include 2783 function, 15 static
and one constant-body operation. This is not 2799 defects, allocations or reachable
application paths. Repeated lowered operations with the same location, function,
callee and type coalesce; architecture-specific bodies remain distinguishable.

| Candidate kind | Records | Interpretation |
| --- | --- | --- |
| Allocator, collection/growth/copy and factory operations | 163 | Can allocate or use already-prepared capacity; review sponsorship and the containing owner. |
| Other owned-result producers | 507 | Can allocate, return shared ownership or move existing storage; follow the producer before assigning a disposition. |
| Empty constructors and reference clones | 98 | Normally no new backing allocation at this site. |
| Explicit and implicit Drop operations | 2031 | Many are guard releases or generic operands; inspect actual containing locks/IRQ state and child destruction. |

## Source reconciliation and selected work

- CQ ring/backlog storage already reserves kernel backing bytes in
  `completion::delivery::preparation` through `cq_budget`. Its ordinary vector
  spelling does not create a new admission finding.
- IPC buffers, frame catalogs, IOMMU ledgers, thread/root tables and observer
  factories have existing admitted owners. Their caller contexts remain subject
  to the current C01–C18 review; this checkpoint does not undo their local proof.
- Six non-test source call sites still use default `snapshot_bytes`: five in
  syscall handlers and one store boundary. Their reachability also depends on
  the configured trusted actor; compiled presence does not establish it. The operational connector also allocates
  plaintext scratch. Q1 groups the six syscall input/scratch sites; Q2 owns the
  store caller's potential permanent-cache lifetime.
- `PreparingUpgrade` and `DomainLaunchTransaction` have active rollback Drop
  behavior. Q2 must review them and their profile/upgrade callers as complete
  operations, not merely add a vector charge.
- PCI traversal, per-LP construction, launch encoders, library producer edges
  and optional profiles remain Q3 review obligations. A boot-looking filename
  does not prove boot-only reachability. The generic closure IPI API has only
  self-test callers in the current source search; compiled presence alone does
  not prove application reachability.
- Root custody exists; other continuation categories, supervisor correlation,
  real outstanding device I/O and combined pressure/fault qualification remain
  Q5–Q8 work. The original R7/R18 acceptance requirements are preserved.

Sizing review found a concrete Q1 constraint: the current 3 MiB ordinary
frame-snapshot byte pool cannot admit the legal 4 MiB artifact maximum. A maximum
pickup plus plaintext requests 4334788 bytes. Q1 records a headroom proposal and
its cost; no protocol ceiling is reduced and no limit is changed by this report.

## Capture and validation

Using the pinned `nightly-2026-07-27` toolchain and existing signed service
bundles, both production profiles completed the following MIR-only compilation:

```sh
CARGO_NET_OFFLINE=true CATTEN_AARCH64_SERVICE_BUNDLE="$PWD/target/embedded-services/aarch64-unknown-none" \
  cargo rustc --locked -p catten \
  --target target_specs/aarch64-unknown-none-catten.json \
  --no-default-features --features acpi -- \
  -Zunpretty=mir -Zmir-include-spans=yes -Zdump-mir-exclude-alloc-bytes=yes \
  > target/resource-arm-production.mir

CARGO_NET_OFFLINE=true CATTEN_X86_64_SERVICE_BUNDLE="$PWD/target/embedded-services/x86_64-unknown-none" \
  cargo rustc --locked -p catten \
  --target target_specs/x86_64-unknown-none-catten.json \
  --no-default-features --features acpi -- \
  -Zunpretty=mir -Zmir-include-spans=yes -Zdump-mir-exclude-alloc-bytes=yes \
  > target/resource-x86-production.mir

python3 scripts/audit-resource-inventory.py \
  --mir arm-acpi-minimal=target/resource-arm-production.mir \
  --mir x86-acpi-minimal=target/resource-x86-production.mir \
  --date 2026-11-04 --output target/resource-review-inventory.json
```

Eleven extractor controls cover generic function/const syntax, empty versus growing
collections, reference clones, explicit/implicit drops, macro and missing-span
origins, static/function separation, opaque owned results, allocator function items, incompatible input and
duplicate coalescing/profile membership. They run in the host suite. A test
failure exposed missing body origins in the initial extractor. Final review also
caught static initializers inheriting a preceding function label. Scope-comment
and item-boundary handling were corrected and regression-tested before the
version-controlled capture.

No new guest campaign is claimed for this tooling/documentation checkpoint.
The preceding implementation's native results remain their own qualification.

## Limits and acceptance

Primary-crate MIR includes unused generic bodies and omits dependency bodies;
it is not a monomorphized reachability graph. These two profiles omit optional
display/devicetree and other architecture configurations. API-pattern extraction
cannot certify allocator completeness, dynamic callback capture, destructor
safety or physical completion. A source search supplements the census but does
not replace the remaining manual/dependency/profile reviews. Q3/Q5 must account
for opaque, excluded and newly discovered paths before inventory acceptance.

No Q1–Q8 package or combined closure gate is accepted by this capture. The
eight packages describe the remaining workload, not equal effort or a readiness
percentage. Existing unresolved incidents keep their recorded status. SEC-07/18
remain partial; physical-platform qualification and production trust, enrolled
node authentication, security time and credential custody remain separate gates.
