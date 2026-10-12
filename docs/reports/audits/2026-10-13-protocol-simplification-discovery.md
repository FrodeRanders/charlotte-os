# Current-only discovery advertisements

Second batch of the [API/protocol simplification sweep](../../reference/api-simplification.md),
following deployment-descriptor removal in `aa8c7ae4`.

## Implementation and boundary

The discovery protocol exposes one borrowed `PeerAdvertisement`, one complete
builder/parser pair and a `DiscoveryFrame` that validates header and payload
together. Base-only response helpers, optional extended payloads and the
`ROLE_UNKNOWN` fallback are removed. Invalid or oversized builds return
`None` without modifying their output; they cannot silently publish a prefix.
Both service probes and replies use this same contract. Complete decode
precedes peer learning and probe response, with current version, one known
flag, declared size, UTF-8 IDs, known role and duplicated Ethernet source
validated centrally. Cluster-ID and self-MAC filtering remain in the service.

The former reserved header word now declares the exact payload length. Old
zero-length headers reject, even with a full cluster suffix. This is required
to distinguish a real empty cluster block from Ethernet zero padding on a
short retired base-only frame. The canonical version number stays 1; old
interpretations do not remain accepted. Only zero padding to Ethernet's exact
60-byte minimum is allowed outside the record. Every node's signed discovery
service must rebuild together; mixed old/new bundles are unsupported. Discovery
cache state is transient and needs no persisted migration.

No IPC resource owner, cleanup receipt or wait contract changes. The peer-list
and cluster-status IPC layouts remain unchanged. Routing hints remain
unauthenticated; accepted syntax/MAC consistency does not prove peer identity,
membership, deployment authorization or hardware quiescence. Periodic probing,
Raft refresh, scalar reactor ownership and existing readiness gaps remain
separate work. No security finding is closed by removing compatibility alone.

The [wire reference](../../reference/discovery-protocol.md), networking
architecture, LaTeX/PDF manual and sweep ledger describe the new boundary.
Historical reports are preserved.

## Regression evidence

Five new host tests cover all four roles in probes and responses; every
truncated payload/frame prefix; old base-only and zero-length headers;
invalid UTF-8, roles, flags, version, EtherType and source duplication;
maximum-size success; oversized/invalid builder rejection without prefix
publication; and exact, partial, nonzero and excessive Ethernet padding.
The padded retired base-only fixture rejects even when its padding could
otherwise look like three empty cluster fields.

## Validation

Validated on 2026-10-13 from `aa8c7ae4` plus this batch:

- `scripts/run-host-tests.sh`: 397 passed, zero failed, four ignored
  documentation examples. The five new protocol tests account for the change
  from 392. Signing-policy, boot-result and syscall-emitter probes and signing
  tool self-tests also passed.
- `scripts/build-catten-services.sh --embed` rebuilt and signed 41 ARM ELFs.
  The x86 runner's build-only mode rebuilt and signed 37 ELFs and compiled the
  DNS/discovery kernel. The ARM DNS/discovery kernel also compiled. Strict
  AArch64 service Clippy and `cargo fmt --all -- --check` passed.
- The tracked PDF rebuilt successfully. Its final two LaTeX passes follow a
  path-wrapping correction. The 208-page output was rendered and visually
  inspected on pages 102–103; the discovery text and reference are readable
  without clipping. Existing unrelated manual layout diagnostics remain
  outside this batch's scope.
- `git diff --check` passed, and local links in changed Markdown resolved.

## Initial failed pair and execution limitation

The first ARM launch attempt rejected before building/booting because the
runner inherited `CODEX_SANDBOX_NETWORK_DISABLED=1`. Its preflight diagnostic
was `two-QEMU socket networking is unavailable in this sandbox`. The already
authorized outside-sandbox rerun used the runner's explicit network override;
this preflight rejection is separate from the guest failures below.

Both initial guests used `--dns-test`, fresh storage, distinct MACs and a
180-second limit, on socket port 19470. x86 booted while ARM compiled, so
their startup was staggered. The x86 kernel hash was
`f09ffb370e0f993464ca60ef5a302f6528dcc4cb30bb91cb56e939eb93450436`;
ARM used `3f0b2ca21a88b43ebff6838216ceeed83313b69ab02878839477b42372d44958`.

Around 8.18 seconds, ARM panicked at `self_test/cq_wait.rs:268:9`:

```text
assertion `left == right` failed
  left: 4
 right: 3
```

This is the timer-sponsor equality assertion after the saturated watchdog
admission probes. The exact matching symbol capture resolves return address
`0xffffffff801a439c` to `catten::self_test::cq_wait::cq_driver+0x4074`;
the panic path is `__rustc::rust_begin_unwind+0x74`, assertion/panic helpers,
and `kernel_thread_trampoline+0x8`. The last pre-panic aggregate observation
was 11 passed / 9 pending, bitmaps `0x47fd` / `0xfb802`. There was no terminal
self-test result. ARM's serial progress later stopped after DNS echo launch;
its fixture was stopped after capturing the failure and exited 1.

Discovery itself observed one peer on both nodes; ARM later reported final
two-member membership at epoch 6, `joint=false`, commit 7. The paired x86 run
ended with 17 passed / 1 pending (`0x7bbff` / `0x80000`), with DNS pending,
and exited 1: `authoritative self-test result was not produced within 180s`.
It repeatedly reported candidate state after its initial join exchange.
These observations do not establish why progress stopped, or that the timer
count mismatch caused the later DNS state.

The aggregate mismatch identifies no specific retained timer and is not proof
of a production leak. It recurs after the prior
[completion-wait fixture corrections](2026-10-12-completion-wait-outcomes.md);
the fixture's accounting/context qualification remains open. No test, timeout
policy or kernel wait implementation was changed to make the repeat pass.

## Passing identical-artifact repeat

Both kernels were prebuilt and verified against the initial hashes before
reuse. A new paired run used fresh disks, the same signed bundles and MACs,
socket port 19471 and a 180-second limit:

```text
scripts/run-x86_64.sh --dns-test --net-listen 19471 --mac 52:54:00:12:34:61 --fresh-storage --timeout 180 --instance disco-cleanup-x86-repeat
scripts/run-aarch64.sh --dns-test --net-connect 127.0.0.1:19471 --mac 52:54:00:12:34:62 --fresh-storage --timeout 180 --instance disco-cleanup-arm-repeat
```

The fixture reused the verified kernels/bundles through the runner's explicit
skip-build settings and ran outside the sandbox with ARM's network override.
Both runners exited 0. ARM discovered its peer at 15.342624 seconds and
completed 20 passed / 0 failed / 0 pending at 24.816984 seconds
(`passed_bitmap=0xfffff`). x86 discovered its peer at 21.306320 seconds and
completed 18/0/0 at 30.264352 seconds (`passed_bitmap=0xfbbff`). Both reported
final two-member membership at epoch 5, `joint=false`, commit 7, and DNS success:
replicated catalog, remote invocation and generation-fenced endpoint-death
unregister. This validates current discovery interoperability between ARM and
x86; it does not erase the initial failed pair or establish its cause.

Captured SHA-256 identities:

| Artifact | SHA-256 |
| --- | --- |
| ARM initial/repeat kernel | `3f0b2ca21a88b43ebff6838216ceeed83313b69ab02878839477b42372d44958` |
| x86 initial/repeat kernel | `f09ffb370e0f993464ca60ef5a302f6528dcc4cb30bb91cb56e939eb93450436` |
| ARM signed bundle, 41 ELFs | `db100e7ea48a9593f2ceacb6db27e06279d8431e42f405fa3ba3c0bbe7e42ec4` |
| x86 signed bundle, 37 ELFs | `b9b49511d49a1cbff833d9a72d66189a07c6686ac14eede5d36861dcc71fe084` |
| ARM signed discovery ELF | `229475caf696c416500bb48df982a29bfcc4307d44b3e7265a5c4b708508cd05` |
| x86 signed discovery ELF | `c32124ea469d569f80b6b0761c397373fb2518fa4534ce797f296d45d220ded6` |
| Rebuilt manual PDF | `11ea46ba636467ecb27e0d90aaec9604a026da29612f4dbe066aa518cd0a1e1d` |

Bundle identity hashes sorted, newline-terminated `basename:ELF-SHA256` lines.
Relevant failures, results, symbols and hashes are recorded inline; temporary
logs, symbol sidecars and ignored build outputs are not published evidence
dependencies. These guests use public development trust, not production
enrollment. Security finding counts remain unchanged.

## Remaining sweep

Raft persistence migration (S03) is next, followed by duplicate deployment,
IPC and wait APIs. This batch is not a completed whole-codebase sweep.
