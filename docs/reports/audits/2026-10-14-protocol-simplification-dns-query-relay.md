# S08b2c1: complete correlated DNS query identities

Date: 2026-10-14. Baseline: `932420a1` (S08b2b). This batch continues
internal protocol simplification with the correlated follower query relay only.
It does not close the remaining remote invocation, registration or raw adapter
inventory. See the [current contract](../../reference/dns-query-relay.md) and
[batch ledger](../../reference/api-simplification.md).

## Change and review boundary

The old query request/reply tags `0x12/0x13` shortened caller, name and owner
identities to a 255-byte prefix. Current tags `0x24/0x25` require complete
little-endian u32 lengths with a 4,096-byte bound per identity and exact frame
termination. Retired tags reject; relabelled old u8 payloads cannot decode as
current records. Request caller/name must be nonempty. Explicit missing or
read-barrier rejection replies may still carry an empty owner and generation
zero. Those are existing outcomes, not parser defaults.

Encoders validate all lengths before fallible allocation. Decoders validate the
complete frame before fallible identity copies. Followers capture the deadline
before preparation and publish pending state/send only after successful
encoding. The original name moves into that pending record instead of being
cloned. Preparation rejection returns the existing oversized/resource-limit
status. Rejected reply preparation logs once and sends no partial reply; the
caller retains its original correlated query and deadline. There is no replay
of an accepted invocation.

Authenticated peer/caller equality, DNS session, query ID, expected leader and
generation checks remain required. Catalog-v18, Raft pair-v3, DNS interface 3
and runtime ABI 3 remain current and unchanged. The codec owns bounded ordinary
byte vectors; it creates no capability, cleanup receipt, wait source or registry.
The existing pending-query/reactor family and C18 retain their qualification
limits. See [cleanup/recovery](../../reference/cleanup-recovery.md).

## Host, build and static verification

- `scripts/run-host-tests.sh`: **449 passed, 0 failed, 4 ignored** Rust tests,
  including the three new query codec tests. Existing signing, boot-input,
  secure-boot, syscall, tooling and object-store Python controls passed too.
- Final services host repeat after test-only decoder-bound refinements:
  **117 passed, 0 failed, 0 ignored**. Production sources were unchanged after
  the signed builds and guest execution.
- Golden request/reply layouts and explicit missing/read-barrier outcomes;
  distinct identities sharing a 255-byte prefix through exactly 4 KiB;
  oversized writers and complete oversized frames; empty request fields;
  wrong/retired tags, complete old grammars and relabelled old bodies; every
  truncated prefix, overflowing declared lengths and trailers all passed.
- Both signed service builds passed through the architecture-specific build
  scripts. Strict service and kernel Clippy (`-D warnings`) passed on ARM and
  x86; kernel checks used the same discovery/DNS/deployment/cluster-control
  feature set as the linked guests. Native runners also enforce executable,
  read-only assembly sections.
- Workspace rustfmt and diff whitespace checks passed. Markdown, contributor
  instructions and the LaTeX manual were updated together. Two successful
  PDF passes produced 212 pages; changed pages 89–90 were rendered and inspected
  for legibility and layout. This is not a whole-manual layout repair claim.

These host controls do not inject allocator exhaustion or every live
query/transport failure. Native node identities were short; the 4 KiB identity
case is codec evidence, not a long-identity cluster execution claim.

## Failed runner preflight, retained separately

The first linked attempt stopped before QEMU boot with ARM runner exit 1;
x86 never started. The runner reported:

```text
error: two-QEMU socket networking is unavailable in this sandbox
       CODEX_SANDBOX_NETWORK_DISABLED=1 prevents the listener/connect path.
       Re-run outside the sandbox (or with network permission).
       Set CATTEN_ALLOW_SANDBOX_NETWORK=1 only if the sandbox is known to allow it.
```

This was an inherited environment-flag preflight rejection, not a guest test or
executed network failure. The same authorized localhost fixture was rerun using
the explicitly approved unsandboxed tool invocation with
`CATTEN_ALLOW_SANDBOX_NETWORK=1`. The subsequent actual listener and linked
guest results below are separate evidence; they do not erase this rejection.

## Linked ARM/x86 execution

Fresh stores, four LPs per guest, `--deploy-test`, and a 300-second guest budget
were used. ARM listened on TCP 19136; x86 connected to `127.0.0.1:19136`.
The coordinator registered stdout readiness with a selector, started x86 after
ARM boot publication, and confirmed QEMU PID 1759 actually listening on that
port before the x86 boot. Coordinator terminal exits were ARM 0 and x86 0.

| Observation | ARM | x86 |
| --- | --- | --- |
| Final selftests | **22 passed / 0 failed / 0 pending**, 105.636886 s, bitmap `0xcfffff` | **20 / 0 / 0**, 40.337126 s, bitmap `0xcfbbff` |
| DNS success | 105.635676 s | 40.336033 s |
| Signed deployment success | 105.634842 s | 40.335264 s |
| Controller controls | 16.510846 s | 19.913713 s |

ARM explicitly reported follower state at 91.783282 s; x86 reported the follower
remote invocation at 27.890000 s. DNS success recorded replicated catalog,
remote invocation and automatic endpoint-death retraction fenced by generation.
Controller controls passed upload, retired-opcode and malformed-query rejection
and the fixture key ceremony.

Both nodes observed these exact signed replica pins:

| Generation | Node | Variant | ARM status time | x86 status time |
| --- | --- | --- | --- | --- |
| 1 | `0x42e278ea` (ARM) | 0 | 96.709922 s | 31.542752 s |
| 2 | `0x42e27737` (x86) | 1 | 101.680284 s | 36.400410 s |
| 3 | `0x42e278ea` (ARM) | 0 | 105.501328 s | 40.118651 s |

Agent diagnostics selected Aarch64 variant 0 and x86 variant 1 with runtime
ABI 3 from the locally verified signed artifact cache. Deployment success
covered readiness across all three generations, cooperative and zero-grace
forced retirement, and rejection of legacy forwarding. No DMA startup failure
or guest panic was observed in this actual linked run.

## Artifact identity

SHA-256 values below identify this execution's artifacts; paths name artifact
roles, not required ignored evidence files. The read-only v4 store inspector
verified staged DNS object `0xfffe2f18f4769a5c` in each guest data image and
matched its complete bytes/hash to the corresponding signed DNS ELF.

| Artifact | Bytes | SHA-256 |
| --- | --- | --- |
| ARM debug kernel | 61,800,744 | `13f2d708076bcc9a34fa1a7952d915e23cb2cd2a2b72be48254a97fd91b71bee` |
| x86 debug kernel | 58,673,912 | `7f8f1d4d1eee535824301e503a1761df74d71a92e268c77212df2eccd52f028a` |
| Signed ARM DNS ELF | 898,704 | `75dab288773fcbad5a6527489a7b5c1e08a970332614ea9b2dd8ecc7ec133786` |
| Signed x86 DNS ELF | 926,656 | `515857474157e409672f771b4919b1623d79f2eda9e7486880159e0dce2eda89` |
| Local greet fixture bundle, identical on both runners | 1,896 | `c8bceb2e21d7a5cd231465507f4a9b520c381234abfb5777ab64b3d302c85ba3` |
| Tracked manual PDF | 1,014,516 | `488fa897c69ae2a328bd2c7203e07edad3f3722af8ea8dab69ea1d62f9168170` |

## Remaining qualification and next batch

Remote invocation `0x10/0x11` and registration/retraction `0x14/0x15/0x16`
still need coherent full-identity grammars and caller updates. Scalar/named and
packed-name adapters, operational release representations, physical-address
queries and mailbox word/capability contexts remain in S08b2c2. This batch
does not infer safe deletion of current boot/IRQ or connector-binding contexts.

No HTTP, S3, Kafka or new admission-pressure guest fixture ran in this batch.
Earlier intermittent ARM stalls, DMA-map admission contention (including the
first x86 S08b2b run), HTTP reset/EOF failures, shared-CQ per-waiter readiness
and production trust/custody qualification remain open. A passing linked run
provides no new hardware, root, stack or backing quiescence proof and identifies
no cause for those historical failures.
