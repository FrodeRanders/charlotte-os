# Captured park-delivery fixture panic and current-build matrix

Date: 2026-10-15. Baseline `1109d05a`, with the preceding audit review,
persistence diagnostics and automatic failure capture changes retained.

## Bounded investigation

The initial plan was one concurrent wave of fresh four-LP ARM/SMMUv3,
x86/VT-d and x86/AMD-Vi no-network guests, each with a 180-second host window,
then one fresh four-LP ARM/x86 linked deployment pair with 300-second windows.
Automatic failure capture remained enabled; passing guests did not attach a
debugger. This exercises existing admission, cancellation, retirement and
persistence fixtures, not the complete R7/R18-5/G7 fault/pressure matrix.

The first ARM guest panicked. Its failed run is retained separately from the
correction and subsequent execution. Intel passed **15/0/0** at 31.003525 s;
AMD passed **15/0/0** at 28.201623 s. Both completed the existing 64 scheduled
IPC close/exit episodes and four first-entry controls. These two passes use
the original fixture and do not erase the ARM failure.

## Spontaneous failure and captured evidence

The ARM panic follows guest timestamp **11.254088 s**. Concurrent UART output
interleaves the assertion, but the captured matching-ELF backtrace identifies:

```text
Kernel panic:
crates/catten/src/cpu/scheduler/deferred.rs:257:17
assertion failed: matches!(MASTER_THREAD_TABLE.write().get(tid).unwrap().state,
                          ThreadState::Running(_))

LP1:
panic_stop
rust_begin_unwind
core::panicking::panic_fmt
core::panicking::panic
deferred::test_park_delivery::{closure#0}     deferred.rs:257
CallOnNotify::notify                        observer/mod.rs:113
NotificationBatch::notify                   registration.rs:287
deferred::drain                             deferred.rs:98
switch_ctx + 120
cond_yield_lp_from_irq                      ops.rs:187
irq_dispatcher                             aarch64/lp/mod.rs:399
irq_common + 100
test_scheduler_waiter_admission             cq_wait.rs:58
cq_driver                                  cq_wait.rs:497
```

The independent watcher classified `kernel-panic` at host **17.868869 s**,
captured QMP state/registers and all four LPs, and completed LLDB with exit 0
and no reported errors. The runner returned 1 without authoritative completion.
Other LPs were in timer-event processing, the device fixture's scheduler yield,
and idle. Later UART output reaching bootstrap seed completion is not recovery
of the panicking LP or a passing suite. This is the first native spontaneous
panic capture in this follow-up, separate from the earlier deadline controls
and unsuccessful NMI-induction experiment.

## Diagnosis and correction

The fixture publishes a callback on the producer LP and then enters a wait.
Publication temporarily masks IRQs but restores its caller's state. A timer
can preempt the producer before wait entry. The scheduler correctly changes
the outgoing producer to `Ready`, then the incoming-stack switch helper drains
that LP's callback queue. Requiring the producer to remain `Running` rejects
this valid pre-registration delivery. The captured IRQ-tail/switch/callback
path and scheduler's Ready-before-incoming-dispatch implementation establish
the invalid premise. The spontaneous capture does not contain a decoded
producer-state value; the forced control below checks that value directly.

The corrected fixture permits `Running` or `Ready`, verifies the captured
producer generation and still rejects `Blocked` or any terminal state. The
zero-hit assertion now shares a local publication mask with enqueue, avoiding
another legitimate delivery race. All wait-admission, original IRQ policy,
before-registration delivery and returned-charge assertions remain enabled.

Four additional cases prepare a same-LP fresh peer and its exact-generation
exit watch before masking. After publishing the callback, they request a direct
architecture context switch under that mask, deliberately bypassing cooperative
yield's pre-switch drain. The callback must observe the producer as `Ready`.
Timed/event-only waits and both enclosing IRQ policies are covered. Peers remain
owned by their existing thread/exit-watch machinery; each exact exit result is
observed and its completion explicitly closed. No scheduler implementation,
wait deadline, cleanup owner, physical proof or replay policy changes.

## Validation and artifact identity

With the forced-switch cases present but the original `Running`-only check
restored, ARM again panicked at `deferred.rs:273:21`, after guest timestamp
**6.428304 s**. Its captured stack goes through `cond_yield_lp` directly from
`test_park_delivery` at line 310, rather than an incidental timer tail. Thus the
new case reproduces the bad premise without depending on random preemption.
The watcher captured all four LPs with QMP/LLDB at host **12.799849 s**, no
reported capture errors; the runner returned 1 with no authoritative result.
This is an intentional negative control, separate from the spontaneous failure.
The `Running | Ready` correction was restored before subsequent builds.

The corrected fresh no-network guests all return 0. The two x86 guests run
concurrently with the same kernel and distinct VT-d/AMD-Vi machine arguments;
ARM starts after its serialized Cargo build. Each guest completes four forced
Ready-producer cases, four first-entry controls and 64 scheduled IPC close/exit
episodes, plus durable Raft voter process restart to term 2.

| Corrected guest | Park-delivery controls | First-entry controls | IPC episodes complete | Authoritative result |
| --- | --- | --- | --- | --- |
| ARM/SMMUv3 | 7.433650 s | 8.380351 s | 8.963881 s | **17/0/0**, 10.938499 s, bitmap `0x1ffff` |
| x86/VT-d | 21.188720 s | 22.081531 s | 22.761315 s | **15/0/0**, 24.898928 s, bitmap `0x1bbff` |
| x86/AMD-Vi | 22.104158 s | 23.028260 s | 23.727795 s | **15/0/0**, 25.804700 s, bitmap `0x1bbff` |

The fresh linked pair also returns 0 on both runners. ARM's actual QEMU
listener on TCP 19158 was confirmed as PID 22706 before starting x86. Both
use the signed fixture MAC identities `52:54:00:12:34:01/02`; their prepared
descriptor bundles are byte-identical. The x86 deployment kernel was rebuilt
before ARM startup, then reused for execution. No no-network kernel is reused
for a deployment profile.

| Linked observation | ARM/SMMUv3 | x86/VT-d |
| --- | --- | --- |
| Four forced Ready-producer controls | 7.322388 s | 14.522772 s |
| Four first-entry controls | 8.322298 s | 15.396399 s |
| 64 scheduled IPC close/exit episodes | 8.918416 s | 15.955547 s |
| NIC online, actual TX completion | 7.416084 s | 14.713288 s |
| Controller upload/rejection/key controls | 13.121350 s | 20.494788 s |
| Signed deployment success | 66.887134 s | 44.620483 s |
| DNS replication/invocation/retraction success | 66.888826 s | 44.621771 s |
| Authoritative result | **22/0/0**, 66.889894 s, bitmap `0xcfffff` | **20/0/0**, 44.622906 s, bitmap `0xcfbbff` |

Both observe generation 1 pinned to ARM variant 0, generation 2 to x86 variant
1 and generation 3 back to ARM variant 0. Deployment success includes
cooperative and zero-grace forced retirement and legacy-forwarding rejection.
Across the five corrected guests, **320 IPC episodes, 20 first-entry controls
and 20 forced Ready-producer cases** complete. There is no corrected-run kernel
panic, Raft fail-stop or authoritative failed/pending result. The initial two
x86 passes and both failed ARM samples remain separate evidence.

The host suite passes **488 Rust tests, zero failed, four ignored**, plus the
existing boundary checks and all thirteen diagnostic watcher tests. Strict
kernel Clippy passes both custom targets with `--no-default-features --features
acpi -- -D warnings` and their staged bundle variables. Workspace rustfmt and
diff whitespace checks pass. Native runners verify executable/read-only kernel
assembly sections. Bundled service source/signatures are unchanged from the
[persistence diagnostics batch](2026-10-15-raft-persistence-diagnostics.md#artifact-identity);
this batch does not claim a new service build or service Clippy run.

Commands use `CATTEN_SKIP_EMBED_BUILD=1`; kernel rebuilds retain
`CARGO_BUILD_RUSTFLAGS='-D warnings'`. Only the identified, already-built kernel
profile is reused with `CATTEN_SKIP_KERNEL_BUILD=1`. Fresh instances keep
failed and passing stores separate. Linked execution also sets
`CATTEN_ALLOW_SANDBOX_NETWORK=1`.

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance matrix-arm-fixed
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance matrix-intel-fixed
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance matrix-amd-fixed
./scripts/run-aarch64.sh debug --smp 4 --deploy-test --fresh-storage --timeout 300 --net-listen 19158 --mac 52:54:00:12:34:01 --instance matrix-deploy-arm
./scripts/run-x86_64.sh debug --smp 4 --deploy-test --fresh-storage --timeout 300 --net-connect 127.0.0.1:19158 --mac 52:54:00:12:34:02 --instance matrix-deploy-x86
```

The spontaneous ARM sample used `matrix-arm-4` with the original cached
no-network kernel. The negative control used `matrix-park-negative` with the
new forced-switch cases and the old Running-only assertion. The initial Intel
and AMD samples used `matrix-intel-4` / `matrix-amd-4`, respectively. All these
single-guest windows were 180 seconds; failures triggered capture early.

| Preserved original artifact | SHA-256 |
| --- | --- |
| Spontaneously failing ARM kernel | `d34502ddcf9905af677a75433a512d102730d2cbe4b85e02c308863ad9ec4752` |
| Original passing Intel/AMD kernel | `685e01571e00be39f36e154ca27b60084673517ab550fecda2f3cb21eb85b893` |
| ARM serial at trigger | `7b4af8c282e23f57fd013c26f1c7d6727f61028168241401630425e8cb2df9ef` |
| ARM final serial | `97cf2ed51e4f2f896be59bf510be70fdfc0031686917c7ada410ed13135312fa` |
| ARM all-LP LLDB capture | `b41135b41c192adc8c77d037e437faeaa58bc7cd2bcf825cb97055186b6d269b` |
| ARM QMP register capture | `b663e9898261305cf8acdba72e00cbc69dd1804c1d54cd52a2b5a1fb532f4fd1` |
| Forced-switch negative kernel | `35fccce4d61a128035dfce1d75675f95903d8bbd2c435397493dd00d839a41c5` |
| Negative all-LP LLDB capture | `fc46a546c5a755d83cd3416c39c16c1015f4fd31a26215e10bcc8230139bd38d` |
| Negative final serial | `00b5f08802feefc36f0e039343db4611390ea0b54ce7832ad986e7da927deb78` |
| Corrected no-network ARM kernel | `1d875b66edb5d4d2a371aed82503629c2570883fc14ecc1005d361d9505b9d30` |
| Corrected no-network Intel/AMD kernel | `fd2ec13917fa695d30fb0efeb7a38d066e27ab760f17e6fdbdbf40772ee6e4ba` |
| Corrected linked ARM kernel | `1d73a6037dab10c6f26b12e2e1ddb0ff6329b7fd7dd155a7f4b0a12bfb17d61f` |
| Corrected linked x86 kernel | `a9e6b0f85c3f25fdfb28146d58fdc45557c3221f7b77efbdbe61412f6da08676` |
| Linked ARM final serial | `34b00f4c1b400cf4cef7919e8a4dd0d9911975a2298d498b5ae95dc06d99bcd5` |
| Linked x86 final serial | `95e5c437e7da7eca29ae2ceec381c02ecdad981ef158f200d9fd12b4550bed3d` |

## Disposition

This is a newly attributed fixture panic. It does not explain the historical
IPC exact-exit, memory-close watch, stack lease, persistence or NIC failures.
Those observations remain open. Numbered finding accounting remains 29
corrected within scope, six partial and three open; C16/C18/G1/G7 are not closed
by this fixture correction or by a bounded passing matrix.
