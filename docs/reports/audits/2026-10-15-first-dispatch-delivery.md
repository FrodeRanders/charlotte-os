# First-dispatch retirement and readiness delivery

Date: 2026-10-15. Baseline: `6867bdb2`.

This investigation corrects a concrete context-switch ordering gap. It does
not establish the cause of the historical IPC close-worker exit timeouts or
the later Raft persistence errors. C16/G1/G7 and those historical observations
remain open; see the [retained investigation](2026-10-14-ipc-close-exit-investigation.md).

## Finding and correction

Both architecture `cond_yield_lp` implementations ran requested-thread staging
and producer-LP deferred delivery after `switch_ctx` returned. A resumed context
returns through an older yield continuation and reaches that code. A fresh
kernel or userspace context instead returns into its initial trampoline, skipping
the continuation entirely. An outgoing abort request or queued wake could
therefore depend on a later scheduling/idle/park boundary. The earlier
[park-publication fix](2026-10-14-park-deferred-delivery.md) covers a different
boundary and does not make the fresh trampoline return through yield.

Both switch assemblies now call the same `finish_context_switch` on the
incoming stack, after outgoing save/Arm ownership release and incoming
address-space installation, before restoring its register frame. Scheduler and
thread-table guards have left; IRQs remain masked. The helper stages requested
off-CPU owners and drains the existing producer-LP metadata transport. No-switch
passes call the same helper directly. No physical reaping moves into assembly
or IRQ tails: the original pinned, IRQ-enabled reaper and its complete owners,
charges, quiescence checks and terminal release fence remain authoritative.

Restoring incoming FP/SIMD state after callback work preserves initial userspace
isolation. x86 saves its exact incoming-frame pointer and aligns only the helper
call, handling both initial user and saved kernel frames. Existing user entry
GPR clearing and TLS/FPU isolation remain enabled. There is no protocol/runtime
ABI change, added polling, timeout increase or new recovery registry.

## Targeted regression and preserved negative control

The scheduled exit-watch fixture prepares one bounded callback batch and arms
it for one captured incoming generation before submission. `record_dispatch`
enqueues it under the existing LP/thread-table guards at that exact dispatch,
after all earlier delivery opportunities. The fresh worker requires delivery
before entry. The callback checks IRQs remain masked and reenters the thread
table after dispatch guards leave. Each selected LP gets a distinct fresh
thread; its original exact-generation exit watch must still complete within
five seconds. The source/token/sponsor remain retained through observation and
the test checks the registration charge returns to zero. This is fixture-only
metadata injection, not a replacement wait source or cleanup authority.

With this regression added and the original switch implementation unchanged,
four-LP ARM failed before fresh worker entry:

```text
panicked at crates/catten/src/cpu/scheduler/deferred.rs:149:9:
assertion `left == right` failed:
fresh context stranded its dispatch-boundary readiness delivery
  left: 0
 right: 1
```

The last preceding timestamp was **7.785886 s**. Six raw backtrace frames
survived. The runner exited **1** at its unchanged 120-second window with no
authoritative completion; the last coordinator observation was **13 passed,
4 pending**, bitmaps `0x6ffd` / `0x19002`. This failed run is separate from the
passing repeats below and is not a reproduction of the original IPC timeout.
Its executed ARM kernel SHA-256 was
`c822b7f9f71bc5ec7c0e7b6be0ffe369340642917f9ae29921236624421e02ec`.
The targeted negative control ran on ARM; x86 shares the static bypass but was
not run with the correction removed in this batch.

## Native validation

All guests use fresh stores, four LPs and existing signed bundles, rebuilt and
verified during the preceding batches. The current Sitas pin is `d27f7e7`.
No service source/signature, test deadline or cleanup policy changes here.
The security runs use `scripts/run-aarch64.sh` / `scripts/run-x86_64.sh debug
--smp 4 --security-test --fresh-storage --timeout 180`, with
`CATTEN_SKIP_EMBED_BUILD=1`, `CATTEN_ALLOW_SANDBOX_NETWORK=1` and
`CARGO_BUILD_RUSTFLAGS='-D warnings'`. The negative control instead uses ARM's
ordinary feature set and a 120-second runner window; its new pre-entry assertion
is independent of the scoped security fixture.

| Run | First-dispatch control | IPC close/exit episodes | Authoritative result |
| --- | --- | --- | --- |
| Unmodified baseline ARM, `--security-test`, 180 s | Not yet present | 64 passed at 8.793460 s | **19/0/0** at 55.683388 s, bitmap `0x2003ffff`, runner 0 |
| Corrected ARM, `--security-test`, 180 s | All four LPs passed at 7.668147 s | 64 passed at 8.403429 s | **19/0/0** at 55.246955 s, bitmap `0x2003ffff`, runner 0 |
| Corrected x86, `--security-test`, 180 s | All four LPs passed at 16.778845 s | 64 passed at 17.613805 s | **17/0/0** at 65.646911 s, bitmap `0x2003bbff`, runner 0 |
| Corrected linked ARM, `--deploy-test`, 300 s | All four LPs passed at 7.956847 s | 64 passed at 8.554570 s | **22/0/0** at 126.476931 s, bitmap `0xcfffff`, runner 0 |
| Corrected linked x86, `--deploy-test`, 300 s | All four LPs passed at 14.631237 s | 64 passed at 15.211761 s | **20/0/0** at 41.310319 s, bitmap `0xcfbbff`, runner 0 |

The corrected guests pass initial GPR/FP/SIMD isolation, natural return and
self-exit ownership, remote abort, scheduled reapers, completion waits and scoped
security controls. These results do not prove universal bounded reaper progress.
Independent security fixtures differ across runs; this is not an identical-input,
single-variable comparison of their complete signed launch sets.

Two early corrected ARM invocations failed before guest execution because host
port 8081 was already occupied by another test guest. The final ARM run used
ports 18080/18081. A mixed-test setup initially rejected its missing explicit
network setting before build/boot. An initial ARM mixed guest used a MAC outside
the fixture's signed node pins and was deliberately stopped before peer startup;
it supplies no completed deployment result. Its kernel was
`7a01ad6ad364efdd2408cccef5a4b0a96493ab3e0ed2dcdb977409892b73e428`.
The corrected mixed pair uses the fixture identities `52:54:00:12:34:01/02`
(`0x42e278ea` / `0x42e27737`) on socket port 19153.

The linked pair uses each runner's `debug --smp 4 --deploy-test --fresh-storage
--timeout 300`, the same build/network environment above, ARM `--net-listen
19153 --mac 52:54:00:12:34:01` and x86 `--net-connect 127.0.0.1:19153 --mac
52:54:00:12:34:02`. Before starting the x86 runner, the real ARM QEMU listener
was confirmed (PID 94119). Catalog replication, remote invocation, endpoint-death
retraction and all three signed architecture-pinned deployment generations pass
on both guests, including cooperative and forced retirement. Across the four
corrected guests, **256 distinct IPC close/exit episodes and 16 first-entry
controls pass**. The baseline's 64 episodes remain separate. No historical IPC
timeout or Raft persistence failure recurred in these completed suites.

## Checks and artifact identity

Host suite: **483 passed, 0 failed, 4 ignored**, plus the existing signing,
boot-input, secure-boot, syscall, CI-runtime-input, tooling and store-format
checks. Strict kernel Clippy passes for both custom targets with
`acpi,virtio_net_test` and the corresponding runner bundle environment.
The initial x86 check omitted that required bundle variable and failed at
`env!` expansion; the corrected invocation passed. Workspace rustfmt and diff
checks pass. Native runners enforce executable/read-only assembly sections.

| Artifact | SHA-256 |
| --- | --- |
| Baseline ARM security kernel | `893f31a8266264c08b31728d8a0d53ce381c06f0fa639eb9ce09ee7c2bbb5aa3` |
| Corrected ARM security kernel | `7f3b3bc5cc01a40e02f66de7d552f942eafb86b0f26b139e2e9ead9e80bbdeb4` |
| Corrected x86 security kernel | `b711a838b60bedde869ad532b7ed0b32e4915de74296a059af406004202adbc4` |
| Corrected linked ARM kernel | `7a01ad6ad364efdd2408cccef5a4b0a96493ab3e0ed2dcdb977409892b73e428` |
| Corrected linked x86 kernel | `63565af47f742e02f5aae744704aa6278a75b066701db7694f1826e39e430601` |
| Signed ARM NS | `6c473655f837bc398f1108ef6300038f5635a0b8d66286345a02506a04712eaa` |
| Signed ARM DNS | `7aa05784ebc9074e0f6db485bfdd384eda893d4e4bc41618e46409a4374a4693` |
| Signed ARM Sitas | `a01b596eab01adc9f8bf7ac45228b1b21da9dcdb00b9e38799850358d01ed564` |
| Signed x86 NS | `86f16dc65605950eac1897a54b1c986b31c2244f1b2b79c68f74c66e1be4df28` |
| Signed x86 DNS | `969af8bd0f5faa3abd3454dc7a17dba6c4c06ba437652b449d167826dd1b1827` |
| Updated manual, 214 pages, 1,021,965 bytes | `12c7c25acd3362f73b2fb452b6dc992d4e06595f780d61ea1c150029b149a403` |

The manual was built twice and rendered pages 72/73 were visually inspected.
It now describes request-before-switch, incoming-stack staging and scheduled
physical reaping instead of claiming self-exit removes its context before save.
Evidence above is inline, independent of temporary or ignored artifacts.

## Limits and next work

The dispatch-boundary regression proves the omitted first-entry delivery and
the corrected ordering in these native executions. It does not reconstruct a
historical interleaving, establish physical release from an exit notification,
qualify arbitrary callbacks/outer guards, or close SEC-18. Existing snapshots,
worker-return markers and absence remain diagnostics, never cleanup receipts.
Historical IPC exit-watch and Raft-persistence attribution, broader G1/G7
qualification and the remaining DMA/metadata/API work stay open.
