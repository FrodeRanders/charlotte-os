# One-shot kernel boot trust handoff

Date: 2026-10-07. Baseline: `00b01202`, following
[signed policy and revision lineage](2026-10-07-security-signed-trust-policy.md).
**SEC-04 remains partial; no finding is closed by this batch.**

## Change

Kernel launch previously selected DNS's public trust independently from the
deployment plane and final kernel gate. Its operational configuration function
could overwrite an existing policy/key/name-service tuple. These were trusted
kernel APIs; this report does not claim a demonstrated EL0 replacement exploit.

`service::admission::PreparedBootTrust` now retains public policy, signed-fields
digest and a zeroizing recipient-key owner. Signed preparation consumes a
verified public receipt, checks its launch cluster and derives/checks the
recipient public key before publication. Mismatch rejects with no fixture
fallback. Development preparation is a separate, explicit constructor.

Publication consumes this owner into a single immutable kernel slot. Every
later attempt rejects and returns its entire preparation after unlocking;
there is no replacement/reset API. The successful private-field `BootTrust`
view also captures the supervisor's exact name-service domain and endpoint.
It borrows the existing kernel bootstrap grant rather than adopting ownership
of a scalar capability.

DNS, clusterctl and agent manifest construction now borrows that same published
view. The final scoped-artifact, operational-connector and shutdown gates read
the immutable slot. A callback receives a zeroizing temporary recipient copy
after the policy guard is released. The old independent/raw-key configuration
entry points are removed rather than retained as compatibility paths.

`launch_steady_state_with_trust` consumes preparation and rejects a foreign
cluster or occupied slot before service composition. Today's boot wrapper
explicitly passes development trust through that common entry point. A future
protected boot adapter can pass signed preparation through it only after
providing executable authentication, committed acceptance state and custody.
Initial trusted platform spawning retains its existing mandatory-fixture panic
policy; this change does not introduce transactional rollback of the whole
steady-state service set.

Contract: [bootstrap policy and kernel handoff](../../reference/bootstrap-trust-policy.md).

## Evidence

The kernel's deferred boot assertions use local policy slots, leaving live boot
trust unchanged. They check signed verification/preparation, foreign cluster
and mismatched recipient rejection, unpublished-owner drop, empty-slot access,
public manifest/gate agreement, exact captured registry identity, callback
execution outside the policy guard, returned preparation on replacement,
signed/development replacement rejection in both directions and retained
installed digest. The actual composition entry point also rejects foreign
cluster and duplicate publication without replacing live boot trust.

A new host test regenerates the checked-in 328-byte public kernel fixture from
documented, publicly known deterministic test seeds and verifies it. It contains
no private material. Existing signature tamper/lineage tests continue to run.
Zeroizing owners cover their owned buffers; this does not close SEC-15's broader
inventory of compiler/cryptographic temporary copies.

## Validation

- `scripts/run-host-tests.sh`: passed, including nine signed-policy tests,
  13 signer tests, existing host crate tests and signing/boot-result checks.
- Host Clippy (`charlotte-launch`, `cluster-sign`, all targets) and both custom
  kernel targets: passed with `--locked` and `-D warnings`.
- QEMU Intel VT-d and AMD-Vi, fresh-storage/no-network suites: each passed
  **15/15**, zero failed/pending. The new boot-trust assertions passed on both.
- Arm SMMUv3, fresh-storage adversarial security suite: passed **19/19**, zero
  failed/pending. Boot-trust assertions and ordinary network/deployment startup
  passed; scoped probe checks were `0xffff` across publication generations 1/2,
  with cancellation traffic retired after 4,704 requests.
- Direct `CATTEN_TRUST_MODE=production` kernel check: rejected with the expected
  build-script diagnostic.
- `cargo fmt --all -- --check`, `git diff --check` and local documentation-link
  checks: passed.

Kernel-only changes reuse the previously built embedded service bundles.

## Remaining boundaries

There is no protected firmware installer, rollback-resistant state commit,
authenticated executable chain, sealed recipient custody or production boot
selector. Initial platform services still use compiled development roots.
The handoff is a trusted kernel API, not evidence of platform authentication;
a receipt verified against substituted root/state cannot establish it.
Runtime policy rotation and protected restart/recovery remain future work.
Captured registry identity is not a shutdown/root lease and does not resolve
SEC-18's cooperative-status ownership gap. Production remains disabled and the
current 30-finding ledger remains **20 corrected, six partial, four open**.
