# Renewed-audit remediation — SEC-31–35 (2026-10-10)

This records corrections to the
[2026-10-10 renewed security audit](2026-10-10-security-audit.md), based on
revision `2221b5bfa0ce6f303cfe4e07f6c8602060adfbfa`. The audit remains a
historical record of the reviewed code; this report accompanies the corrected
worktree. SEC-36, SEC-37 and SEC-38 are not addressed here and remain open.

## Disposition

| Finding | Disposition | Result |
| --- | --- | --- |
| SEC-31 | Corrected | Reassembly state creation, buffering and assembly are gated on an admitted total; assembly reserves fallibly |
| SEC-32 | Partly corrected | Transmit cannot hand out a token without reserved backing and allocation failure drops the frame; retained cross-domain receive backing stays open under SEC-07 |
| SEC-33 | Corrected | `SPAWN_ARTIFACT` uses the fallible launch wrapper and rejects without stopping an LP |
| SEC-34 | Corrected | AArch64 `svc #77` emitter restored; an automated check now covers every wrapper emitter |
| SEC-35 | Corrected | Platform reserve follows explicit launch provenance; descriptor-less artifacts are ordinary-classified |

## SEC-31: reassembly admission before state and allocation

The reassembly struct, the admission predicate and the assembler now live in
the host-tested [`relmsg_reassembly`](../../../crates/catten-services/src/lib.rs#L1422)
module. [`fragment_within_ceiling`](../../../crates/catten-services/src/lib.rs#L1451)
requires `total <= MAX_MSG` and `offset + payload_len <= total`, and
[`assemble`](../../../crates/catten-services/src/lib.rs#L1463) rejects any
total above the ceiling before reserving its output with `try_reserve_exact`.

The service
([relmsg.rs:364](../../../crates/catten-services/src/bin/relmsg.rs#L359))
computes the predicate first; only an admitted fragment creates or updates
reassembly state, and the assembler is reached only under that gate. An
over-ceiling fragment is ignored without touching existing state, so a
spoofed fragment cannot clear a legitimate in-progress reassembly.

Tests: `oversized_totals_are_never_admitted` covers the `0xFFFF_FFFF` wire
value, `MAX_MSG + 1`, gaps and the exact ceiling;
`assembly_requires_an_admitted_total_and_every_fragment` covers complete
assembly, a missing fragment and an over-ceiling state.

## SEC-32: fail-safe transmit reservation

[`CharlotteEthDevice`](../../../crates/charlotte-smoltcp/src/lib.rs#L99) now
reserves one persistent mapped page at first transmit/receive use. `transmit`
and `receive` return `None` when the reservation is unavailable, so smoltcp
drops the packet instead of receiving a zero-length buffer; a queued receive
frame stays queued. The advertised IP MTU is clamped to
`TX_SCRATCH_BYTES - ETHERNET_HEADER_LEN`, making a frame larger than the
reserved buffer unreachable, and the token consume path clamps defensively.
The device releases the page in `Drop`.

The consume path no longer allocates before invoking the smoltcp closure. It
builds the frame in the persistent scratch, then tries a fresh moved object
for the driver; allocation or mapping failure drops that frame and leaves the
service running
([lib.rs:288](../../../crates/charlotte-smoltcp/src/lib.rs#L300)).

Tests: `missing_tx_scratch_applies_backpressure_without_a_token` verifies no
token is handed out without backing, the queued frame survives and is
delivered once reservation succeeds; `advertised_mtu_never_exceeds_the_reserved_frame`
covers the clamp. Queue, ordering and bounded-ingress tests continue to pass.

**Retained boundary:** a client can still retain moved receive objects charged
to the TCP/IP creator until its own caps close or it exits. While that
backing is retained, TCP/IP's own transmit objects cannot be reserved and
packets are dropped (degraded network availability, recoverable on release)
instead of aborting the service. Kernel cross-domain charge transfer and a
service-side retention bound remain SEC-07 R7-3 work; this report does not
claim that the starvation path is closed.

## SEC-33: fallible runtime artifact launch

[`sys_spawn_artifact`](../../../crates/catten/src/syscall/mod.rs#L2248) now
calls `try_start_domain_with_limits(loaded, limits, false)` and returns
failure to the caller; the launch transaction tears down the unstarted
domain. The panic wrapper remains confined to the ambient boot launch path
([supervisor.rs:397](../../../crates/catten/src/service/supervisor.rs#L397)).

The existing kernel fixture
([thread_admission.rs:204](../../../crates/catten/src/self_test/thread_admission.rs#L204))
already forces the same fallible rejection and asserts the pre-existing
mapping is preserved and the unstarted root is reclaimed; it passes with the
new signature.

## SEC-34: AArch64 syscall emitter coverage

The `svc3` table now emits
[`svc #77`](../../../crates/catten-syscall/src/lib.rs#L445). A new host check,
[test-syscall-emitters.py](../../../scripts/tests/test-syscall-emitters.py),
parses the declared syscall list, each AArch64 `svcN` table and every
`svcN(SyscallNumber::Variant, ...)` call site, and fails when a wrapper has no
matching arm. It is wired into
[run-host-tests.sh](../../../scripts/run-host-tests.sh#L8) and reports
`68 call sites across 4 arities checked`. Removing the 77 arm makes the check
fail with `svc3(SyscallNumber::SpawnOperationalConnector)`, confirming it
detects the audited regression class.

The AArch64 service bundle was rebuilt with the emitter; the deployment-agent
path can now reach the kernel gate instead of panicking in the wrapper. The
connector path itself was not deployed end to end in this pass.

## SEC-35: provenance-based platform classification

`try_start_domain_with_limits` takes an explicit `platform` flag. The ambient
boot wrappers pass `true`
([supervisor.rs:397](../../../crates/catten/src/service/supervisor.rs#L397));
scoped deployment-descriptor launches and descriptor-less `SPAWN_ARTIFACT`
pass `false`
([supervisor.rs:925](../../../crates/catten/src/service/supervisor.rs#L925),
[syscall/mod.rs:2248](../../../crates/catten/src/syscall/mod.rs#L2248)).
Profile-backed platform connectors and the observability service pass `true`.
Upgrades use
[`try_start_upgrade_domain_with_limits`](../../../crates/catten/src/service/supervisor.rs#L403),
which preserves the replaced service's existing classification via
`memory::budget::platform_identity`, so an ordinary service cannot promote
itself by being replaced.

The old inference from `launch_descriptor_digest.is_none()` is removed from
the launch path; the digest remains for the controller's descriptor match
query. The live-upgrade and scoped-launch guest paths continue to pass.

## Validation

- `scripts/build-catten-services.sh --embed`: passed; the AArch64 bundle was
  staged and signed with the fixed `relmsg`, `tcpip`, `agent` and
  `catten-syscall` code.
- `scripts/run-host-tests.sh`: passed, **340 tests, zero failures** (was 336;
  the new reassembly/backpressure/MTU tests and the emitter check are
  included). The emitter check reports 68 call sites across four arities.
- `cargo fmt --all -- --check` and `git diff --check`: passed.
- Clippy with `-D warnings` passed for the AArch64 kernel (default features)
  and the x86_64 kernel (`--no-default-features --features acpi`).
- Fresh-storage AArch64 security guest: **19 passed, 0 failed, 0 pending**,
  `passed_bitmap=0x2003ffff`; scoped probe `checks=0xffff` at publication
  generations 1 and 2; concurrent cancellation retired after 4888 requests.
- Two-guest AArch64 `--relmsg-test`: **both guests 18 passed, 0 failed,
  0 pending** (`0x3ffff`), with
  `[relmsg] SUCCESS: two guests exchanged sequenced, acknowledged Ethernet
  messages`. This is the 70,000-byte payload that crosses the 65,535-byte v3
  boundary and exercises fragmentation, reassembly and acknowledgement end to
  end on the changed code. Two earlier attempts failed for harness reasons,
  not code reasons: concurrent runners shared the service bundle (producing a
  mixed `rng.elf` that panicked before the fixes executed), and a 25-second
  start stagger put one guest past its peer deadline. The passing run
  pre-built the bundle and staggered by five seconds.

Guest commands:

```sh
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18837 CATTEN_DEPLOY_HOST_PORT=18137 \
  scripts/run-aarch64.sh --security-test \
  --instance fixes-final-20261010 --fresh-storage --timeout 180

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18835 CATTEN_DEPLOY_HOST_PORT=18135 \
  scripts/run-aarch64.sh --relmsg-test --net-listen 12052 \
  --instance relmsg3-a --mac 52:54:00:12:34:01 --fresh-storage --timeout 120
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18836 CATTEN_DEPLOY_HOST_PORT=18136 \
  scripts/run-aarch64.sh --relmsg-test --net-connect 127.0.0.1:12052 \
  --instance relmsg3-b --mac 52:54:00:12:34:02 --fresh-storage --timeout 120
```

Validated artifact SHA-256 values:

- AArch64 security-guest kernel:
  `a100c1f39726d00d51634cc38a789ee0551204b89c848ecdebd60d10d38dbe37`
- Two-guest relmsg build kernel:
  `d6d5e31ce8442382d5dda03a997a7debebab3771159bbaa753b8e49d38f65b77`
- `relmsg.elf` (identical in both bundles):
  `c8cb511eae25b94dfac510b0e135c5cb90f1e08ac82ef08d2795ab2daebd27ce`
- `tcpip.elf`:
  `556c7f7011cdfbdc28c4f826f70e873dd55ac7e7792c80b9375b3d0802394826`
- `agent.elf`:
  `c646e8f312ffeab5fe35e1078ab6ce230087c2bad916ecb188859183f6f2cab4`
- `Cargo.lock` (unchanged):
  `6237dbf75d0e29bb7fe1c0dba4301d12d5a298d626fdc89bd05e5d766c50aa76`

## Limits and remaining work

The two-guest test validates the legitimate fragmented reassembly path end to
end, but no guest sends the adversarial over-ceiling reassembly header, no
guest exhausts the TCP/IP object budget, and no guest drives the
descriptor-less launch rejection or the discovery/name-service paths.
SEC-31 and SEC-32 are covered by the extracted host-tested logic, the
end-to-end fragmented exchange, and the source-confirmed call graphs; the
remote/malicious-client scenarios were not reproduced as end-to-end attacks. SEC-33/35 use the existing kernel rejection fixture rather
than a new platform-classification probe; upgrade-classification preservation
is covered by source review and the live-upgrade guest suite, not a
classification assertion.

SEC-32's retained cross-domain receive backing, SEC-36 (unbounded discovery
peer state), SEC-37 (legacy lookup of policy-published registrations) and
SEC-38 (`SPAWN_UPGRADE` snapshot bound) remain open. The standing SEC-07/18
and production trust/authentication criteria in the
[remediation ledger](../../reference/security-remediation.md) are unchanged.


## 2026-10-14: owned DNS local invocation follow-up

[S08b2c4a](2026-10-14-protocol-simplification-dns-local-invocation.md) removes the
packed-name local invocation adapter and scalar call/connection cleanup stages.
Its unique incoming transaction and typed destinations retain reply authority;
both RPC phases bind readiness to the shared CQ and retain one kernel-monotonic
deadline. Rejected reply binding after accepted invocation remains uncertain,
without replay. This is scoped C18 ownership/readiness progress, not closure of
SEC-36/37/38, kernel physical cleanup, full-name local unregister or all raw DNS
reactor contexts. Existing numbered finding dispositions remain unchanged.


## 2026-10-14: full-name NS generation and cleanup follow-up

[S08b2c4b](2026-10-14-protocol-simplification-name-generations.md) replaces packed
publication observation/retirement and raw pending cleanup handles with exact
CNSGEN01 records, owned CQ-bound calls and original deadlines. Registrar/manager
checks remain; a DNS lookup does not confer retirement authority over somebody
else's binding. Raft-commit and publication-scan cleanup share one submission
claim. Host tests and strict Clippy pass; native new DNS controls and three
signed rollout generations execute on both architectures, with x86 20/0/0.
The broader ARM suite retains the now-reproduced historical IPC exact-exit
failure, also captured on x86 in the prior run. No numbered finding disposition
changes; third-party dead NS binding producers, wider raw contexts, physical
quiescence and production trust remain open. Failed runs and current hashes are
self-contained in the batch report.
