# Observer-list backing admission

Date: 2026-10-07. SEC-07 continuation after timer backing admission at
`bcdaa56`. Independent list/control-block storage now has node-wide admission;
general metadata and heap findings remain partial.

## Finding and correction

Waiter and event-watch entry charges returned when their individual entries
were freed. Their source lists were independently allocated Arcs. An empty list
could remain with a source, or survive source destruction through registration
tokens, without consuming entry admission. Weak references could retain backing
after payload destruction too. Existing documentation already identified this
gap; source review and controlled lifetime fixtures confirmed it. No hostile
application exploit or whole-node exhaustion was demonstrated.

All `ObserverList` construction now reserves from an independent pool of 8,192
node slots, with 6,144 available to ordinary allocation and the remaining shared
pool available to kernel-designated platform sources/sponsors. Reservation
precedes both auxiliary holder and list allocation. `ListRef` wraps the charged
Arc and is retained by source fields and registration tokens. Final strong/weak
backing release and private charge-holder deallocation precede the one refund.
There is no uncharged constructor or bare global-allocator list owner.

Classification comes from the existing captured completion/call/watch charge,
generation-qualified CQ/endpoint admission, or the first lazy waiter's captured
generation-owned sponsor. It is retained unchanged through promotion and later
subscribers. Lazy sources check sponsor retirement before first initialization.
No source lock performs an address-space lookup; no application role/name grants
reserve access. This is a node account, not a new namespace/principal account.

Completion and CQ waiter lists, IPC endpoint/call lists, thread-exit subscriptions,
lazy completion callbacks, blocking locks, timer waiters and both boot-status
sources all use the admitted owner. Existing outward failure shapes are retained.
CQ preparation currently maps list quota rejection to its allocation-failure
variant. Rejected initialization publishes no source and can retry after release.
Existing entry, record, timer and CQ limits remain separate and unchanged.
No userspace ABI or wire format changes.

The new counter guard neither allocates nor enters another subsystem. It is
released before allocation, destruction or callbacks. Allocation under existing
registry/source guards is otherwise unchanged and remains SEC-18 work.

Contract: [observer-list admission](../../reference/observer-list-admission.md).

## Regression evidence

- A real entry is detached/discarded, returning its watch charge. Its retained
  registration token keeps the empty source list charged. Dropping the token
  destroys the last strong list payload; 128 weak aliases and a final original
  weak owner keep allocation admission until final release. Exact counters
  reconcile to baseline throughout.
- Injected list-allocation rejection after reservation returns unused admission.
  This is a controlled factory failure, not actual physical OOM. Existing entry
  allocation rollback remains tested separately.
- First ordinary lazy-source initialization remains ordinary after its sponsor
  is promoted. A second fresh source consumes platform capacity. Retirement
  rejects another fresh initialization without allocating a list.
- Counter-only ordinary saturation rejects direct list creation and lazy waiter
  registration, with no waiter-entry charge retained. Completion submission,
  CQ attachment, callback installation, endpoint creation and scalar-call
  preparation reject through their real production paths. Existing record and
  endpoint charges, the operation's buffer and endpoint queue remain unchanged.
- A real platform source and waiter initialize under ordinary pressure without
  increasing ordinary list usage. Total saturation rejects platform creation
  too. Releasing reservations permits lazy-source retry, completion submission
  and CQ attachment; normal IPC/completion/root cleanup restores exact baseline.
- Existing callback/watch/waiter, cancellation, timer, IPC and generation-reuse
  fixtures remain enabled. The shared allocator's six host regressions pass,
  including allocation failure destruction order and concurrent final release.
  The new list pressure tests run serialized before AP schedulers, not as an
  EL0 multi-client flood or exhaustive cross-LP race exploration.

## Validation

| Check | Result |
| --- | --- |
| Host allocation-owner regressions | **6 passed, 0 failed**. |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; scoped probes `0xffff`, publication generations 1/2, concurrent cancellation retired 4,728 requests. |

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance observer-list-intel-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance observer-list-amd-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance observer-list-arm-20261007 --fresh-storage --timeout 180
```

The host command is `cargo +nightly-2026-07-27 test --locked --manifest-path
/path/to/charlotte-os/crates/charlotte-lifecycle/Cargo.toml --test
charged_allocator`, invoked outside the repository's kernel build-std config.
Guests rebuild kernels and enforce assembly section permissions. This kernel-only
change reuses validated embedded services. Arm uses required local port-binding
permission.

## Remaining scope

List quantities and their auxiliary owners are bounded, not total kernel heap
bytes or allocator overhead. Per-domain/principal list attribution, arbitrary
callback captures, sponsor backing, registry nodes and other weak-only Arc
allocations remain separate work. One generation can still compete for shared
list admission; there is no per-subscriber fairness or per-platform-service
progress guarantee. Maximum pool backing was not physically allocated by the
saturation fixture. SEC-07 remains partial.

SEC-18 remains partial for allocation/destruction under existing subsystem
guards, physical-platform quiescence, broader reset support and abandoned-owner
recovery. Authentication, production provisioning and security-time work are
unchanged. Passing the scoped suites does not certify whole-node containment.
