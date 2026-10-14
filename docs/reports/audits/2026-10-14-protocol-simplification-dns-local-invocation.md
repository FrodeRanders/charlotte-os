# S08b2c4a: owned full-name DNS invocation (2026-10-14)

This batch follows `c35746d5` and replaces the remaining packed-name asynchronous
local invocation adapter. The [current contract](../../reference/dns-local-invocation.md)
and [simplification ledger](../../reference/api-simplification.md) distinguish
this correction from the remaining generation-unregister, raw receive/transport
and close-watch work. Existing security finding dispositions remain unchanged.

## Changes and retained boundaries

- DNS interface 4 requires `CDNSCL01` moved-memory client requests: exact total
  length, complete 1..=256-byte name, opcode and signed argument. The new pure
  `charlotte-protocol-dns` crate supplies the single kernel/service encoder and
  strict borrowed decoder. Packed-name/untagged 12-byte clients reject without
  fallback or default fields. Register ABI 3, catalog-v18, Raft pair-v3 and
  remote relay tags remain unchanged; signed variants rebuild together.
- `LocalInvocation` owns the named NS lookup, accepted invocation and returned
  connection. Both pending RPCs bind reply readiness to the same DNS CQ; the
  reactor observes them without blocking on one service. Unused returned
  capabilities use their Rust owners. Pending teardown precedes connection
  teardown; no scalar ownership reconstruction remains in this adapter.
- The client receive boundary adopts unique memory/connection/reply attachments
  once in `IncomingMessage`. Mapping and complete frame/name validation precede
  routing. Typed reply authority stays with the existing query, local or remote
  destination; ordinary begin rejection returns that destination intact.
- One kernel-monotonic deadline begins before decode/preparation. Follower query
  routing and both local phases retain it. Invocation encoders recheck expiry before
  publication. The reactor waits for the earliest pending deadline within its
  existing clock/transport watchdog. The callee captures a local deadline rather
  than comparing clocks between nodes. CQ binding rejection after accepted
  invocation is uncertain, never permission to replay.
- The existing 64-operation vector reserves fallibly before a local RPC begins.
  Runtime cancellation/close Drop policy remains, including domain abort when
  cleanup cannot safely release ownership. No new registry, custody, physical
  release proof or retry authority is introduced. Dedup, peer/session/call ID,
  expected generation and protected namespace denial remain enforced.

## Executed validation

| Check | Result |
| --- | --- |
| Final `scripts/run-host-tests.sh` | 467 passed, 0 failed, 4 existing ignored; signing, tooling, store and syscall checks also pass |
| Three new codec tests | Golden/signed argument; 255/256-byte and trailing-zero names; invalid writers; every truncated prefix, trailers, overflowing lengths, bad magic and retired record |
| Seven new production-state-machine tests with owning fake resources | Full-name success and both CQ bindings; lookup/preparation/invoke rejection; accepted bind uncertainty without replay; original deadline across unrelated turns and delayed preparation; cancellation/drop; unused returned-resource disposal |
| Signed AArch64 and x86 service builds | Both complete; all final staged/signed bundles rebuilt |
| Strict service and fixture-kernel Clippy | Both architectures pass with `-D warnings` |
| Formatting | Final `cargo fmt --all -- --check` and diff whitespace check pass |
| Linked four-LP precursor, fresh stores | ARM 22/0/0, x86 20/0/0 |
| Linked four-LP final source, fresh stores | ARM 22/0/0, x86 20/0/0 |
| Manual | Two successful sequential TeX passes; 213 pages; final changed pages 89, 90 and 106 rendered and visually inspected without new clipping/overlap |

The precursor preceded the additional post-encoding deadline checks. Its final
self-test summaries were at ARM 94.857732 s and x86 39.750902 s. Final-source
summaries were ARM 94.338942 s and x86 45.314556 s, with passed bitmaps `0xcfffff`
and `0xcfbbff`, respectively, and zero failed/pending bitmaps. Both coordinators
observed ARM boot publication through registered stdout before starting x86,
verified the real TCP listener at x86 boot, and returned `arm=0, x86=0`.
Runners checked executable/read-only kernel assembler sections.

In the final pair, native retired/trailing invocation rejection occurred at ARM
79.844080 s and x86 31.372847 s. Echo returned 42 at ARM 80.183858 s and x86
31.378057 s through the named-memory lookup operation. The hosting x86 node
retracted generation 1 after endpoint death at 32.010708 s and rejected stale
replay. Both guests completed the three exact signed variant-pinned rollout
generations, cooperative retirement and zero-grace forced retirement while
protected greet forwarding remained denied. Each of four guests across the two
pairs completed 64 distinct scheduled claim/endpoint close episodes with exact
exit watches: 256 in this batch. Those passing repeats do not reproduce or
explain the historical IPC exact-exit timeout or subsequent persistence errors
retained in [registration-relay evidence](2026-10-14-protocol-simplification-dns-registration-relay.md).

An initial strict Clippy check rejected the new generic poll signature for
`type_complexity`; a shared `PollResult` alias removes that warning. Final
strict checks pass. A final format check caught one new match-arm indentation;
`cargo fmt` corrected it. Neither diagnostic was a runtime failure. Both linked
runs passed; no failed native run is omitted from this batch.

## Exact artifacts

SHA-256 hashes identify the complete artifacts used by the runs. Final stores
were read with the strict read-only v4 inspector, which verified each stored
DNS object's FNV checksum; complete bytes exactly matched the corresponding
signed staged ELF at object `0xfffe2f18f4769a5c`. Ignored stores/logs are not
required evidence: their relevant results and hashes are recorded here.

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| Final signed ARM DNS | 914408 | `3b14a8b912f4aacfdd0c1df54290a384c1d3342fcad61991576a53c5cf6effb6` |
| Final signed x86 DNS | 937120 | `0d20c6718afd55f9f2af430d42e5b3394232b80f19ccc234acacc40563c209fa` |
| Precursor signed ARM DNS | 914408 | `a0d1b14492f841f915d6c4206869e82ea5b97d805f7ae890d9c2324ac6d02347` |
| Precursor signed x86 DNS | 937120 | `7f4cff7d3d6eb55d26103567fe7c4bcccc780c3fa17deb47450af5488663a6a2` |
| ARM kernel, both runs | - | `7a654ab470e8ff882350d61339c8cefbf48e8183016b08fdcf957792814c8f1d` |
| Final x86 kernel | - | `70a3752be40bdf6540487c9f7595b96f2f76d8777e2601b2133d0df3a6522b07` |
| Precursor x86 kernel | - | `16ec473eb0b47d388cebcc884c164db2d61815f1c228853ef0f2d116f6cb4db3` |
| Updated manual PDF | 1017916 | `9bdc293ac7b9cadf5d68f2f5dba3b2f70b53749f9d7291533e509f3509bf4bc5` |

The kernel digests come from each runner's boot-time captured ELF digest;
Clippy metadata checks do not replace that execution identity. Signed builds,
fixture kernels and object-store bytes correspond to runtime ABI 3.

## Limits and next batch

The native positive invocation name is short echo, carried through the full-name
memory path. Full 256-byte/trailing-zero identity and delay/rejection controls
are host codec/state-machine evidence, not native long-name publication and
retraction or live allocator exhaustion. Actual mapping/unmap failure, rejected
kernel cleanup, physical backing release and outer IRQ/guard contexts require
separate qualification. The whole DNS reactor is not event-only: its existing
watchdog still covers raw receive/transport interests not wired to this CQ.

Next replace node-local generation observation/unregister together with a
mandatory full-name generation record and owning cleanup transaction, preserving
registrar/manager authorization and exact generation fencing. Raw local
unregister completion handles, close-watch readiness, transport queue/dedup
allocation and other packed/scalar entry points remain in S08b2c4. This batch
updates C18's existing row and evidence; it closes no broader cleanup-recovery
or production trust boundary.
