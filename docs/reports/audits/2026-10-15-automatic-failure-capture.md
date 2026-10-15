# Automatic guest failure capture

Date: 2026-10-15. Baseline `1109d05a`, retaining the preceding audit-process
review and Raft diagnostic changes in the working tree. This is host-tooling
work; no kernel/service behavior, runtime ABI, cleanup owner, guest deadline or
numbered finding disposition changes. Historical IPC/lease/persistence causes
remain open.

## Gap and implementation

ARM previously offered an opt-in end-of-window snapshot with four hard-coded
thread selections and fixed diagnostic-array prefixes. x86 had no equivalent
flag. Main-loop serial inspection could be delayed by synchronous host HTTP
probes. A passing debug repeat supplied no state from an earlier failed run.

Both timed runners now prepare one independent Python watcher with private
mode-0700 Unix QMP/GDB socket storage. No public debugger port is added. It
observes the original monotonic host window and captures once on:

- `Kernel panic:` or `SELFTEST FAILED:`;
- a fully written unsuccessful authoritative result, including failed/pending
  counts and bitmaps;
- `[raft-store] FATAL:`;
- the host deadline, or runner-reported incomplete deployment/host probe.

The failure marker is published before capture begins, and the runner waits
for capture before killing its QEMU child. A successful result cannot erase
an earlier failure. The watcher is independent of blocking host probes; those
probes may still delay final runner exit. The timeout now also supplies an
independent capture/failure budget rather than only a tick counter that omits
time inside synchronous probes. No guest wait budget is increased.

QMP stops the VM, records its CPU list and all-CPU registers; LLDB records all
LP backtraces/registers and timer, waker and scheduler diagnostics. The prior
QEMU running/paused state is restored after capture. QMP commands use absolute
two-second response budgets, LLDB has a 15-second subprocess budget and
finalization has a 35-second watcher budget. The result distinguishes full,
partial and unavailable capture, preserving QMP evidence when LLDB is missing
or fails. If QMP state is unavailable, restoration of the prior run state
cannot be confirmed. A QEMU exit cannot provide a live snapshot and is recorded
as such.

Diagnostic arrays use their symbol-declared sizes. These kernels expose 12,288
bytes of scheduler LP diagnostics, 18,432 timer bytes, 24 waker bytes and an
eight-byte lifecycle counter. They are read in full, rather than assuming
24 scheduler words or four timer rows. Symbol reads reject zero, misaligned or
over-64-KiB declarations. The debugger inspects existing state only; a stopped
CPU, wake, absent thread or snapshot is no exit/quiescence/cleanup receipt.

## Immutable run inputs and controls

Every timed run gets a unique `target/boot-diagnostics/<arch>-...` directory.
It retains the actual `/catten` ELF extracted from its FAT boot image, verifies
the recorded payload and symbol identities, and saves per-service signed ELF
hashes, boot-image/initial-data-disk hashes, QEMU arguments, CPU count,
revision/dirty paths, tracked-diff hash and tooling hashes. Concurrent build
output replacement cannot silently change the debugger's ELF. A reused data
disk remains separately identified; the current bundle is not asserted to be
its stored contents.

Capture preserves serial-at-trigger, final serial, CPU/register data, LLDB
output and artifact hashes. Passing runs retain immutable inputs without
attaching a debugger unless `--debug-snapshot` is explicit. Both runners now
support that flag; successful ARM execution no longer waits out the entire
window just to take it. `--no-failure-capture` opts out. Manual `--gdb`, the
existing debug-stub environment setting and ARM scheduler tracing retain
exclusive debugger ownership and disable automatic capture. Explicit snapshot
conflicts reject before build. Private socket files are removed on finalization;
evidence directories remain.

See [testing guidance](../../guides/testing.md). These files are local build
artifacts, not published audit evidence by themselves; relevant observations
and hashes from this validation are retained below.

## Host validation

Thirteen new host tests cover marker/complete-result classification, contained
user-fault exclusion, failure publication before capture, failure precedence
over a successful finish request, deadline continuity through unrelated output,
no attach on ordinary success, dynamic eight-LP command construction, declared
array sizes, run-state restoration, missing/timed-out debugger partial results,
private-storage rollback, a dead watcher's explicit unavailable outcome,
final UART-tail archival after debugger detach/guest exit,
debugger ownership rejection and immutable boot-image
payload/symbol checks. Buffered unrelated QMP events cannot restart a command
deadline. These are host contracts, not eight-LP native execution.

The final host suite passes **488 Rust tests, zero failed, four ignored**, plus
all existing boundary checks and the thirteen new diagnostics tests. Bash syntax,
Python compilation, local documentation targets and diff whitespace pass. No
Rust source changes were needed for this step. Kernel/service builds and strict
Clippy evidence remain those of the preceding
[persistence batch](2026-10-15-raft-persistence-diagnostics.md), not new lint runs.

The first payload-identity unit test mocked `subprocess.run` too broadly and
intercepted `git` reads inside `check_output`. Its assertion failed; isolating
the Git fixture results corrected the test before the final passing checks.

## Native execution and preserved negative samples

All guests use four LPs, isolated fresh disks, no network, unchanged kernels
and the signed bundles rebuilt in the preceding batch. Native runners still
check executable/read-only assembly sections. Environment:
`CATTEN_SKIP_EMBED_BUILD=1 CATTEN_SKIP_KERNEL_BUILD=1`.

| Run | Guest / runner outcome | Capture outcome |
| --- | --- | --- |
| Final x86 positive, `--debug-snapshot --timeout 180` | **15/0/0** at guest 21.359380 s, bitmap `0x1bbff`; runner 0 | Complete, all four LPs, QMP and LLDB; observed at host 23.573349 s |
| Final ARM positive, `--debug-snapshot --timeout 180` | **17/0/0** at 10.919935 s, bitmap `0x1ffff`; runner 0 | Complete, all four LPs, QMP and LLDB; host 17.591466 s |
| ARM short kernel window, `--timeout 12` | Deliberately incomplete; runner 1, no authoritative completion | `deadline` at host 12.087149 s, all four LPs, complete QMP/LLDB capture |
| Final x86 short kernel window, `--timeout 12` | Deliberately incomplete; runner 1, no authoritative completion | `deadline` at host 12.027134 s, all four LPs, complete QMP/LLDB capture |

The final positives each pass durable voter process restart 1→2, 64 original
IPC close/exit episodes and four first-entry delivery controls. Neither produces
a kernel panic or Raft fail-stop. Snapshots after success are not failure traces.
The short windows intentionally stop valid startup work; their stack locations
do not establish a spontaneous deadlock. For example, the ARM capture includes
retirement-list insertion at `0xffffffff80122b54` and peer barrier/atomic waits;
the x86 capture includes comparison/atomic frames at `0xffffffff800a05c5` and
`0xffffffff800521e5`. The raw register sets and full frames survive independently
of those optimized/inlined source labels.

An earlier positive pair also passed ARM 17/0/0 at 10.513663 s and x86 15/0/0
at 20.330992 s with four-LP capture. Those passes predate the final boot-image
extraction check. An initial five-second ARM deadline captured firmware state
at host 5.022464 s and returned 1; it is not kernel execution evidence. The
12-second ARM control predates final image extraction but exercises the same
capture implementation; final positives and x86 negative validate extraction.

Two initial sandbox invocations failed before guest boot because QEMU could
not bind the private Unix QMP sockets (`Operation not permitted`). Their empty
serial captures and `guest-exited`/unavailable results are setup failures,
separate from approved native execution. A first manual NMI injection attempt
missed its live window and found its socket already removed; that guest passed
15/0/0 at 15.893292 s. A coordinated repeat sent QMP `inject-nmi` while the guest
was running during round-zero IPC work. QEMU accepted it, but no guest panic
appeared: the guest passed 15/0/0 at 19.712185 s and the runner returned 0.
The host control then failed its expected-exit-1 assertion. This is an
unsuccessful panic-induction control, not a reproduced kernel panic or proof
of NMI delivery/handling. Its cause was not investigated in this tooling batch.
Panic-marker trigger behavior is host-tested; native evidence here establishes
live all-LP capture and incomplete-window failure, not native panic capture.

Commands for the final positives and negative windows, with the environment above:

```sh
./scripts/run-x86_64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --debug-snapshot --instance capture-x86-final
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --debug-snapshot --instance capture-arm-final
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 12 --instance capture-arm-kernel-deadline
./scripts/run-x86_64.sh debug --smp 4 --no-network --fresh-storage --timeout 12 --instance capture-x86-final-deadline
```

## Artifact identity

| Artifact | SHA-256 |
| --- | --- |
| Executed ARM kernel | `d34502ddcf9905af677a75433a512d102730d2cbe4b85e02c308863ad9ec4752` |
| Executed x86 kernel | `685e01571e00be39f36e154ca27b60084673517ab550fecda2f3cb21eb85b893` |
| Final ARM positive LLDB capture | `663b3bf55b14114901e90aac5c7399ca8604407eb407e49eb3a9916d84b2a460` |
| Final x86 positive LLDB capture | `cf555fc4a628bd687196a5dd625ce409a6604c2cec3793d39bef014e8bfb5805` |
| ARM kernel-deadline LLDB capture | `8cbc0ccc66acee41cf47240fb512baaac7a981188fd737001b2525077af24e1e` |
| ARM kernel-deadline register capture | `c04dbf93d9be0dd84a025a4d358e91d66b9b6513d844423b7fd56704a8605df4` |
| Final x86 deadline LLDB capture | `aa2f7151cc47972bc97af8688ec3d2edb46be19655a499210691a498fb3f4eef` |
| Final x86 deadline register capture | `041105f3f77ed9136a74cf389ba9420352253ad1da2a233151a7a04751763dde` |

Results, failure diagnostics and capture qualifications above are inline rather
than depending on temporary-file links. Exact signed Raft/DNS identities for
these unchanged bundles are in the version-controlled
[preceding report](2026-10-15-raft-persistence-diagnostics.md#artifact-identity).

## Remaining qualification

No historical IPC/stack/persistence failure was reproduced, and there is no
causal conclusion for one. No physical device, actual outstanding-I/O failure,
HTTP blocking/flood campaign, AMD-Vi capture run, more-than-four-LP native capture,
manual-debugger coexistence or scheduler-trace run was qualified. Kernel/firmware
debugger register availability and unwind quality can vary; partial output is
reported explicitly. LLDB warns that this installation lacks a Rust language
plugin; symbolized frames and registers still succeeded in the captures above.

This tooling supplies evidence for the next current-build failure. It grants
no physical cleanup, replay, restart or custody authority, and closes no
C16/C18/G1/G7 or numbered security acceptance gate.

## Subsequent current-build execution

The [bounded matrix follow-up](2026-10-15-captured-park-fixture-panic.md)
subsequently captured a spontaneous ARM kernel panic and an intentional
forced-switch negative control, each with complete four-LP QMP/LLDB output and
matching immutable kernel identity. It corrects an invalid Running-only
assumption in the park-delivery fixture. This adds native panic-trigger evidence
beyond the deadline controls above; the unsuccessful NMI experiment remains
separate. Corrected ARM/Intel/AMD and linked ARM/x86 suites pass. Historical
IPC/stack/persistence attribution and the full integrated qualification gates
remain open.
