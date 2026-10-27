# Configuration

Configuration records the workload and capacity a node is intended to support.
Security limits need an owner and an enforcement rule; their selected capacities
also need an explanation, a way to adjust them, and qualification evidence.
This guide matches the manual's **Configuration** chapter.

## Sources and lifetimes

| Configuration | Current source and lifetime |
| --- | --- |
| Management TLS | One complete signed/encrypted `CMGTENC1` resource contains credentials, exact leaf/role enrollments and all eleven `Limits` fields. `CATTEN_MANAGEMENT_TLS_RESOURCE` selects its absolute ciphertext path when building; boot verifies it against the same selected admission policy and supplies read-only launch profiles to both services. Replacement currently requires rebuild/restart; see [management TLS](../reference/management-tls.md). |
| Domain TLS entropy | Startup consumes `EntropyConfig` with an owned `rng` connection (or explicit architectural-only source) and a positive request budget. The compiled default is `5000` ms; admission retains that complete configuration through each operation; see [TLS configuration](../reference/tls-client.md). |
| Dependency advisory policy | `.cargo/audit.toml` and the scanner pin in `.cargo/audit-version`, evaluated by CI/local validation. They do not configure a running node; see [the advisory gate](../reference/dependency-advisories.md). |
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

Cryptographic backend flags are a separate build contract. Freestanding x86
services select the erasing software POLYVAL owner through target settings in
`.cargo/config.toml`. An overriding `RUSTFLAGS`/`CARGO_ENCODED_RUSTFLAGS` that
loses that selection rejects the TLS destructor guard. Resource profiles do not
override it. Re-enabling authentication acceleration requires a separately
qualified erasing owner; see the [backend evidence](../reports/audits/2026-10-25-security-x86-gcm-owner.md).

TLS record and certificate capacities are grouped compile-time constants in
`crates/catten-services/src/tls_client.rs`, separate from kernel capacity profiles.
The [TLS configuration reference](../reference/tls-client.md#compile-time-capacities)
records their defaults, upstream framing/layout requirements and joint stack/heap
qualification. Backend/source pins and erasure feature changes also require
both service variants to be rebuilt; they are not live resource-policy settings.

The entropy request budget is a startup API input in `EntropyConfig`, alongside
its owned source. Bundled consumers use `EntropyLimits::DEFAULT`; changing this
central `5000` ms default or their Rust startup values requires rebuilding the
services. There is no manifest/config-page or live-resizing input for it. A
longer observation budget retains the source lease, pending/output borrow and
possible returned backing longer; requalify timeouts, rejection and pressure
without granting replay authority. See the
[deadline contract](../reference/tls-client.md#request-deadline-and-operation-owner).

RNG protocol storage groups `entropy::BUFFER_PAGES`, `BUFFER_BYTES` and
`MAX_REQUEST` (currently one 4096-byte page). These compile-time dimensions
control caller reply allocations and the driver's private DMA buffer together;
they are separate from the request observation budget and kernel capacities.
Rebuild all callers/driver and qualify maximum sizes, retention and DMA before
changing them. DNS nonce lookup/fill shares its existing 100 ms collection
budget, named `CAPACITY_NONCE_TIMEOUT_MS` in its capacity-report context. See
[entropy configuration](../reference/entropy.md#client-policy-and-related-dimensions).

RNG publication shares the compile-time policy in `publication.rs` with other
platform services: `RETRY_BUDGET_MS = 1000`, `OBSERVATION_BUDGET_MS = 5000`,
and `RETRY_PAUSE_MS = 10`. The admission and accepted-observation phases have
separate budgets. Tune all helper consumers, boot ordering and readiness
deadlines together; this is not live per-instance configuration. RNG idle
request/lifecycle observation has an event-only CQ wait, with no timer setting.

RNG hardware polling is separate: `rng_device::Limits::DEFAULT` permits
`1000000` completion-index reads per descriptor; `Limits::new(n)` requires a
positive bound. The bundled driver selects the central default at construction.
This is an iteration ceiling, not a wall-clock deadline or replay allowance.
Its compile-time `QUEUE_SIZE = 8` determines page-rounded ring geometry;
backing size derives from it, while the entropy bounce buffer follows the
protocol dimensions above. Larger queues do not increase outstanding work.
Rebuild and qualify layout/capacity, CPU cost, short completions, original caller
deadlines, timeout/late completion and retained teardown together. See the
[descriptor contract](../reference/entropy.md#descriptor-ownership-and-terminal-failure).

## Related limits

Management TLS groups enrollment/certificate counts, record buffers, fragmentation,
handshake/session budgets and reactor work in one `Limits` value. Reducing fragment
size increases aggregate wire overhead; increasing concurrency multiplies retained
buffers and crypto state. Validate those relationships and service memory/progress
budgets together. The [reference](../reference/management-tls.md) lists exact
defaults, structural protocol bounds, file-based enrollment commands and remaining
qualifications. `management-limits-default` writes every adjustable boot field;
missing or inconsistent settings reject. Central build-time knobs also bound
the profile to `65536` bytes and reserve `32` stack pages for both services,
including the missing-profile startup path. The native
[startup campaign](../reference/management-tls.md#execution-qualification-and-remaining-gates)
checks valid, missing, malformed and substituted resources without altering the
selected policy or turning transport timeouts into startup rejection proof.

Review these groups together when changing workload targets:

| Target change | Related capacity and consequences |
| --- | --- |
| Larger objects | Per-object/domain/principal backing, snapshot bytes, mapping work and pin lifetime. A maximum-sized object must fit an ordinary snapshot. |
| More mappings | Mapping nodes, scratch extents/namespaces and concurrent/retained snapshots. Preparation and uncertain cleanup also consume capacity. |
| More memory capabilities | Unified authority plus shared payload/namespace/read-borrower node bytes and counts. Each overlapping preparation needs two temporary node slots, or three for a read loan, even for an established namespace; retained metadata stays charged. |
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
