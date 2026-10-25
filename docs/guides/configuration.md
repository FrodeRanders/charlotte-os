# Configuration

Configuration records the workload and capacity a node is intended to support.
Security limits need an owner and an enforcement rule; their selected capacities
also need an explanation, a way to adjust them, and qualification evidence.
This guide matches the manual's **Configuration** chapter.

## Sources and lifetimes

| Configuration | Current source and lifetime |
| --- | --- |
| Kernel resource capacity | One trusted Rust build profile, compiled into the kernel and fixed for the boot. |
| Bootstrap service assignments | `CATTEN_CLUSTER_SERVICES` at build time; runners accept repeatable `--cluster-service [NAME=]IPv4:PORT`. The first committed signed ingress policy replaces bootstrap assignments. |
| Launch/grant trust | One prepared kernel boot-trust policy. Resource capacity settings grant no signing authority or platform classification. Production trust remains disabled pending protected provisioning/custody. |
| Application limits and capabilities | Signed deployment plus kernel admission, including stack/thread limits, executable variants and protocol-specific launch profiles. Requests do not enlarge node capacity. |
| Persistent state | Explicit store formats and committed state. Capacity changes authorize neither reset nor silent reformatting. |

Protocol layouts, address ranges, page granularity and hardware register widths
remain structural contracts. Changing them requires coordinated implementation
and format work, separate from tuning resource capacity. See
[tooling inputs](../reference/tooling-inputs.md),
[boot trust](../reference/bootstrap-trust-policy.md) and
[resource policy](../reference/resource-policy.md).

## Adjusting compile-time capacity

The central defaults are
[resource_policy/defaults.rs](../../crates/catten/src/resource_policy/defaults.rs).
The [typed policy and validation](../../crates/catten/src/resource_policy.rs)
define dimensions and relationships. Numerical defaults are preserved by this
centralization.

Either edit the central defaults or set `CATTEN_RESOURCE_POLICY` to an existing
**absolute file path**. Unset selects the defaults. Empty, relative, missing and
directory paths reject. The file is trusted Rust build source, compiled in the
policy module's scope; never accept it from an application manifest or upload.
It defines `pub(super) const POLICY: Policy` and can derive selected fields from
`DEFAULT`. The checked-in
[larger profile](../../crates/catten/src/resource_policy/larger.rs) co-scales
scratch-node, snapshot-buffer and mapping-node byte/count budgets by 25%, keeping
their reserve proportions.

For example, copy that profile to `ops/resource-policy.rs`, review its settings,
then run:

```sh
CATTEN_RESOURCE_POLICY="$PWD/ops/resource-policy.rs" \
  python3 scripts/tests/test-resource-policy.py

CATTEN_RESOURCE_POLICY="$PWD/ops/resource-policy.rs" \
  scripts/run-aarch64.sh --security-test --http-test
```

The same input works with `scripts/run-x86_64.sh` and direct kernel Cargo builds.
Runners pass it to the kernel build. Cargo tracks selection and file changes;
changing or unsetting selection rebuilds the kernel. These policy fields require
no userspace rebuild. Boot serial output identifies the selected policy name.
Preserve the exact profile source and kernel artifact hashes; names alone do not
identify artifacts.

There is currently **no boot-time resource-policy input or live resizing**.
Profile selection does not install trust or provide a fallback for invalid policy.
An already-running node keeps its compiled policy and original charges.

## Related limits

Review these groups together when changing workload targets:

| Target change | Related capacity and consequences |
| --- | --- |
| Larger objects | Per-object/domain/principal backing, snapshot bytes, mapping work and pin lifetime. A maximum-sized object must fit an ordinary snapshot. |
| More mappings | Mapping nodes, scratch extents/namespaces and concurrent/retained snapshots. Preparation and uncertain cleanup also consume capacity. |
| More outstanding RPCs | Capabilities, connections, pending/reply records, endpoint counts/slots, CQs, waiters, timers and observer lists. Queue backing rounds up to a power of two. |
| More service publications | Lifecycle watches, observer lists and replacement overlap. A replacement admits its new watch before dropping the old one. |
| More domains/threads or restarts | Root/principal metadata, thread bundles, stacks, tables and per-principal backing; retiring generations can remain charged. |
| More metadata bytes | Allocator overhead, kernel heap and physical progress reserves. Pool layouts exclude some backing; summed logical ceilings do not establish RAM fit or fairness. |

Within fixed pools, each ordinary dimension must be positive and below total;
each domain dimension must fit ordinary. The reserve is `total - ordinary`, not a
guaranteed allocation share for one essential service. RAM-derived pools use
explicit ratios of usable pages, and the kernel heap has a RAM divisor plus
aligned minimum/maximum reserve. Ratios widen before multiplication; heap growth
clamps before rounding. The current launch ABI still bounds heap virtual space.

No profile changes sponsorship or cleanup. Moving a resource retains the creating
sponsor's charge; retired generations, ASID reuse and late promotion cannot refund
live allocations. Failed cleanup keeps its complete owner and original charge.
Raising capacity supplies no physical completion or accepted-work replay proof.

## Planning and qualification

1. State the workload: RAM, principals/domains, threads, service graph, object
   sizes, RPC concurrency, restart overlap and failure expectations.
2. Inventory each consumed family, unit, sponsor, lifetime owner, rejection
   result and trusted reserve. Include unused candidates, detached work, weak
   retention and uncertain cleanup; classify structural limits separately.
3. Measure occupancy and rejection under that concurrency. Select ordinary
   capacity and trusted headroom together, recording relationships and expected
   behavior when a limit binds. A previously passing default is not a new
   workload's sizing calculation.
4. Run the policy harness, `cargo fmt --all -- --check` and strict kernel builds.
   The harness exercises actual policy/counter code with default, larger and
   optional selected profiles, and compile-rejects invalid policies. Static
   consistency does not establish boot viability or essential-service progress.
5. Boot the exact selected profile on supported backends. Exercise success,
   exhaustion, allocation rejection, cancellation, restart/generation reuse,
   retained charges and confirmed cleanup. Measure management, timer/scheduler
   and teardown progress against deadlines fixed before the run.
6. Preserve profile/artifact hashes, occupancy/latency and failure diagnostics.
   Keep failed or interrupted attempts separate from passing repeats. Requalify
   after changing the profile or workload.

The [configuration evidence](../reports/audits/2026-10-25-security-resource-policy.md)
qualifies selected profiles and boundaries, not arbitrary capacities. Sustained
concurrent pressure and essential-service fairness remain SEC-07/18 work.

## Toward boot resources

Boot input can later provide the same typed policy, but requires an explicit
authenticated, complete grammar and a single installation point before the first
relevant allocation. Kernel-heap sizing and static counters currently precede
such an installation point. That work must order initialization, validate
discovered RAM and structural constraints, and publish one immutable policy before
schedulers and services consume it. Independently resizing live pools would need
a separate account-generation/original-charge contract. Until then, capacity
changes mean a reviewed profile, a kernel rebuild and a new boot.
