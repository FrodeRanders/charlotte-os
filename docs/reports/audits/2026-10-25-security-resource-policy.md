# Security audit: central resource policy and Configuration chapter

Date: 2026-10-25. Reviewed baseline: `741098ef`.

Kernel hardening capacity settings now come from one typed compile-time resource
policy. Operators can edit central defaults or select an absolute-file profile
with `CATTEN_RESOURCE_POLICY`; allocation and cleanup checks keep their existing
owners. Numerical defaults and original-charge lifetimes are preserved.

The [tracked evidence](evidence/2026-10-25-security-resource-policy.json) records
runtime/build/profile/model/artifact hashes, selected native diagnostics and
interrupted attempts separately. The finding ledger remains **29 corrected /
6 partial / 3 open**. SEC-07/18 stay partial and production trust stays disabled.

## Policy and operator documentation

The [typed policy](../../../crates/catten/src/resource_policy.rs) groups seventeen
fixed admission families plus memory-object counts, backing ceilings, discovered-
RAM ratios, ordinary shares, kernel-heap sizing and local queue/watch/waiter
ceilings. Family-local aliases forward to these fields; no new counters,
registry, cleanup family, sponsor classification or ownership mechanism is added.
Reservation, abandonment and confirmed physical release keep their prior rules.

The [Configuration guide](../../guides/configuration.md),
[policy inventory](../../reference/resource-policy.md), manual Chapter **20**
and updated family contracts document adjustment, related capacities and a
workload-driven sizing/qualification process. The manual PDF is rebuilt to
**249 pages**. Its contents page and Configuration pages **197–199** are rendered
and visually inspected. The new chapter has no overfull boxes or unresolved
references. Preexisting layout warnings elsewhere in the manual remain outside
this focused visual review.

The optional profile is trusted Rust build source, not application or deployment
input. Unset selects defaults; empty/relative/missing/directory paths reject.
Cargo tracks selection and file changes. The compiled policy remains fixed for
the boot. Boot serial output reports its name; source/artifact hashes provide
the stronger evidence identity. Production-trust installation is unchanged.

Boot-resource input is not implemented. Early heap preparation and statically
initialized counters require an ordered, authenticated, complete installation
boundary before relevant allocations. The documentation records that work;
there is no independent live-pool resize or untrusted-manifest fallback.

## Validation and sizing boundaries

Compile-time checks reject invalid domain/ordinary/total ordering, zero reserves,
machine-range overflow, invalid RAM ratios, inconsistent backing ceilings,
principal thread metadata above its shared pool, objects that cannot fit an
ordinary snapshot, local source/queue limits beyond their budgets, and invalid
heap alignment/arithmetic. The kernel also checks heap policy against the launch
VA contract and actual allocator arena. Unknown fields reject at compilation.

RAM fraction arithmetic widens before multiplication. Configuration review
identified an extreme-RAM alignment overflow when the newly adjustable kernel
heap divisor is small. The corrected calculation clamps to validated aligned
endpoints before rounding; host tests exercise divisors 1/2/64/max and RAM up to
`u64::MAX`. Default rounding and values are preserved.

The host harness passes three test groups: actual policy/counter tests under the
default, larger and optional selected profiles (**13 Rust tests per compilation**),
**11 invalid-profile compile rejections**, and selector/default/override/path
checks. The selected-profile invocation in this evidence selects the checked-in
larger profile in addition to its independent default/larger compilations.
It exercises the real lifecycle counters, not a duplicate kernel implementation.

Strict base-kernel Clippy passes for both default architectures and the larger
Arm profile. Workspace formatting and whitespace checks pass. Kernel source is
frozen before final native builds; no final source revision is used to turn a
failed run into a success.

The larger example co-scales scratch-node, snapshot-buffer and mapping-node
requested bytes/counts by **25%**. Native pressure controls now read selected
capacities instead of repeating defaults. Ordinary/total metadata counts become
**7,680 / 10,240**. Snapshot bytes become **3,932,160 / 5,242,880**; mapping-node
bytes become **983,040 / 1,310,720**; scratch-node bytes become **491,520 / 655,360**.
Typed charges still follow actual deallocation through publication, root reuse,
late promotion and existing guarded retention. Larger capacity supplies no new
physical proof, custody or replay authority.

## Native qualification

The final frozen source passes all four profile/backend combinations: **76
deferred tests**, **56 HTTP checks** and both security-service generations per
run. The default profile passes on SMMUv3, VT-d and AMD-Vi; the larger profile
also passes on SMMUv3. No final-source guest failure occurs:

| Profile / backend | Deferred passed/failed/pending | Healthy beside stalled peers | Cancellation requests |
| --- | --- | --- | --- |
| development-default / smmuv3 | 20/0/0 | 0.273–0.377 s | 19,016 |
| development-default / intel-vtd | 18/0/0 | 0.185–0.379 s | 17,840 |
| development-default / amd-vi | 18/0/0 | 0.362–0.430 s | 14,576 |
| larger-metadata-test / smmuv3 | 20/0/0 | 0.326–0.381 s | 20,112 |

Native executable assembly-section checks remain enabled. Bundled services are
reused; userspace sources and public runtime ABI/wire formats are unchanged.

Initial host metrics observations, with retry diagnostics retained separately
from guest completion:

- **development-default / smmuv3:** HTTP transport TimeoutError; retry 1/2; then PASS within the existing bound.
- **development-default / intel-vtd:** HTTP transport TimeoutError; retry 1/2; then PASS within the existing bound.
- **development-default / amd-vi:** PASS without a preceding retry diagnostic.
- **larger-metadata-test / smmuv3:** PASS without a preceding retry diagnostic.

All host transport retry contexts remain inline in the evidence. Any timeout
cause remains unestablished; successful retries do not attribute historical
failures or supply physical/replay authority.

Two preliminary attempts were intentionally interrupted after configuration
review identified the heap arithmetic edge case: **Arm had started its guest
without a complete suite result; Intel was still building and had not started a
guest**. Both runner exits were 143. These are neither completed passes nor
evidence of an observed guest arithmetic failure. Captured intermediate policy
hashes, host tails and the Arm manifest/selected serial diagnostics are retained
separately. Not every intermediate runtime source hash was captured, so those
attempts provide no qualification of the final source. Final runs use distinct
instances and the corrected frozen source; no deadline is widened. The larger
profile is a planned configuration qualification, not a passing retry obtained
by increasing a failed default's capacity.

## Formal scope and remaining work

All **23 focused harness cases** pass: three safe configurations and **20 expected
counterexamples**. DMA operation explores **29,175 distinct / 62,003 generated
states**, depth **24**; IPC transaction explores **26 / 71**, depth **11**; root
retirement explores **2,978 / 3,733**, depth **33**. Required safe actions execute;
negative configurations violate named invariants through their intended unsafe
actions.

Model inputs do not change. Existing root original-charge, IPC loan/rollback/
timeout and DMA ownership projections remain relevant. They do not represent
this concrete configuration grammar, capacities, arithmetic, workload minima or
boot installation. Host/native controls qualify those boundaries without an
executable-refinement, arbitrary-profile or sustained-pressure/fairness proof.

This advances resource-policy maintainability and R7-1/R7-3 inventory/validation;
it closes no whole audit finding. Other metadata/principal/general-heap gaps,
sustained concurrent EL0 pressure, measured essential-service progress and wider
cleanup contexts remain. Service-image/header/deployment capacities, userspace
protocol/reactor policy, recovery deadlines and structural layouts remain in
their own contexts and are explicitly inventoried for separate planning.
Historical metrics/root-close/mapping failures remain unattributed.
