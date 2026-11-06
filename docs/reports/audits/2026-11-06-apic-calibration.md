# APIC calibration measurement interval

Baseline `2c4da15c`. The x86 boot calibrator now timestamps before arming its
countdown, with a local execution fence, and preserves raw failed-sample
information. A deterministic control demonstrates the old ordering can measure
zero after a countdown has already expired. This corrects that mechanism; the
original intermittent Intel panic remains historically unattributed.

[Version-controlled evidence](evidence/2026-11-06-apic-calibration.json)
contains source/input identities, control diagnostics, compiled ordering and
native artifacts. The [original capture](2026-11-06-q4-device-principals.md#preserved-intel-boot-failure)
and its evidence remain unchanged. Q4–Q8, other incidents, sustained reliability
and physical-platform acceptance remain open.

## Mechanism and boundary

Previously each sample armed 10000 APIC ticks, then read the TSC start, waited
for zero and read the TSC end. If the countdown expired before the start read,
all its elapsed time escaped the interval. Integer division could then produce
zero picoseconds per tick, correctly rejected by the ten-sample accumulator.
A delayed instruction or VM exit is a possible trigger, not a demonstrated
cause of the historical capture.

The production sample helper now reads the start timestamp, arms the same
countdown, observes bounded completion and reads the end timestamp. The local
start callback uses RDTSCP followed by LFENCE. Intel documents that LFENCE after
RDTSCP orders subsequent execution after the timestamp read.
[Intel SDM, Volume 2, RDTSCP](https://cdrdv2-public.intel.com/835781/325462-sdm-vol-1-2abcd-3abcd-4.pdf).
The compiled Intel campaign kernel independently confirms this order before
WRMSR to `0x838` with `0x2710` ticks. General runtime timestamp reads are unchanged.

Each sample carries TSC start/end, picoseconds per TSC cycle, completion flag and
last observed APIC count. Checked subtraction and multiplication reject reversed
clocks and arithmetic overflow. Timeout or a nonzero final count rejects as
incomplete; a zero derived period still rejects. The existing ten complete,
positive samples and checked mean remain mandatory. Sampling stops at the first
error without retry or fallback period. Failure stops the masked countdown before
panicking with LP, one-based sample index, typed error and raw sample fields.
The existing Prepared/Initializing/Ready boot owner still forbids replay or use
after uncertain initialization.

No timer limit, divisor, poll bound, deadline or backing allocation is raised.
The hardware poll still permits 100000000 attempts and its final recheck. One raw
sample is retained on the boot stack; there is no sample vector, new heap object
or successful-boot diagnostic stream. CPU interrupt masking, publication after
guard release and the mandatory all-LP readiness barrier are unchanged.

Starting before programming includes programming and completion-observation
latency in the mean. This closes the omitted-countdown interval at the cost of
that measurement overhead; increasing the sample count or countdown would cost
boot time and would not itself fix the ordering. Arbitrary VM scheduling delays
can still bias calibration. These changes establish neither precise oscillator
accuracy nor a bound on host scheduling delay. TSC frequency discovery and the
existing picosecond conversion policy are unchanged.

## Controlled evidence

The host fixture executes the actual production sample helper with a clock whose
countdown expires before the programming callback returns. Starting at TSC 100,
10000 ticks advance the end to 10100, with a 1000-picosecond TSC cycle. The corrected
sample returns 1000 picoseconds per APIC tick, with zero allocations.

A private source mutation restores arm-before-timestamp. The same fixture then
fails with `Err(Zero)` instead of `Ok(1000)` and the assertion `complete countdown
escaped measured interval`. The runner requires this negative control to fail;
it is not a failed qualification or a native reproduction of the historical
panic. Other controls cover incomplete completion/nonzero final count, zero
cycle/period and truncated resolution, reversed timestamps, multiplication and
sum overflow. Every typed sample error is injected at each of ten positions;
no later sample or retry runs. Existing boot-owner phase, allocation-free access
and firmware parsing controls remain active: 11 focused tests pass.

The full host suite, formatting and all four strict test/minimal profiles pass.
Fresh-storage, four-CPU security campaigns pass ARM SMMUv3 19/19 and Intel VT-d/
AMD-Vi 17/17 at the existing 240-second deadline. Both x86 backends use the same
kernel and embedded signed security fixtures. ARM minimal boot passes. Ten
additional fixed four-CPU x86 minimal cold boots alternate Intel/AMD, five each,
using one minimal kernel and signed bundle at 90 seconds per boot. No failed
native run is omitted. These finite regressions do not establish a failure rate
or sustained boot reliability.

The first full host attempt ran inside the sandbox and failed four TLS fixtures
at `TcpListener::bind(("127.0.0.1", 0))` with `PermissionDenied: Operation not
permitted`. The complete repeat with local socket access passes. The denied
attempt's identity and relevant diagnostics are preserved separately; no TLS or
production timeout change was made.

## Historical incident status

The original Intel kernel
`14207a4806874ffb2ecf567d19f48f57fe13fe7174d7401b7a696ace1bf48080`
stopped around 0.053 seconds with interleaved `Zero` calibration panic output.
LLDB places the panic beneath `bsp_main`; the other three processors wait at the
startup barrier. Its unchanged 240-second runner records a deadline; an identical
kernel/service/initial-disk repeat passed 17/17. The capture lacks the failing
sample index, TSC endpoints/period and APIC count.

The new control proves a real ordering defect and supplies a correction. It does
not recover those missing historical values. Keep the incident's causal
attribution open and preserve raw diagnostics if a failure recurs. Passing native
runs qualify the captured configurations only; no connection to the earlier
retirement-pin, ARM display or ARM exit-watch incidents has been established.
