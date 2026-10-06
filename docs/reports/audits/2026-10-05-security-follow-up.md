# Security audit follow-up and remediation — 2026-10-05

This records omissions identified after the [renewed audit](2026-10-05-security-audit.md)
and its [remediation](2026-10-05-security-remediation.md). The reviewed baseline
is `32c23d54` (2026-10-05); implementation and validation span October 5–6. Earlier reports remain historical records. Findings
were established by source review; the validation below records the subsequent
implementation evidence, rather than claiming the original impacts were
reproduced before correction.

## Additional findings

| ID | Severity | Baseline defect | Required access |
| --- | --- | --- | --- |
| SEC-23 | High | x86 user exceptions invoke kernel panic instead of domain containment | A running ring-3 application |
| SEC-24 | Medium, potentially higher | First user entry exposes volatile kernel GPR/SIMD state | An admitted ELF or spawned thread that snapshots initial registers |
| SEC-25 | High, conditional | Unbounded TCP/IP request draining and frame backlog can deny shared progress | Sustained admitted IPC requests or forwarded network traffic |

### SEC-23: x86 user fault containment

At the baseline, `ih_divide_by_zero`, `ih_invalid_opcode`, general-protection,
and unrecovered page-fault handlers unconditionally panic. Ordinary exception
stubs do not pass the saved CS to their Rust handlers. The page-fault handler
recognizes some demand-growth cases but still panics after growth rejection or
protection failure. The kernel panic handler disables interrupts and stops its
LP. This is an application-to-shared-availability failure, including accidental
application bugs; it does not establish code execution or kernel-memory access.

Correction: ordinary fault stubs pass vector, error code, saved RIP/CS and CR2
where relevant to one handler. The hardware-saved ring-3 CS authorizes domain
containment. The exact executing ASID comes from the per-LP translation context.
User faults enable maskable interrupts only after the trusted frame/GS setup,
so cleanup and remote IPIs can progress, and abort the whole offending domain.
Valid data demand growth still retries; rejected growth, instruction/protection
faults and other user faults terminate the domain. Kernel exceptions, hardware
aborts and unsolicited NMIs remain fatal. Return stubs mask interrupts before
restoring/swapping GS. Existing requested watchdog NMIs retain their behavior.

### SEC-24: initial machine-state isolation

At the baseline, both user trampolines enter userspace without scrubbing
volatile GPRs. ARM `switch_ctx` retains kernel context pointers in argument
registers, and only its callee-preserved SIMD subset comes from the initialized
frame. Kernel-pointer exposure follows directly from the instructions. Disclosure
of particular secret material in residual registers was not demonstrated.

Correction: ARM programs the banked entry/SP, then clears all 31 GPRs, all 32
SIMD registers and FP control/status before first `eret`. x86 clears all GPRs
other than the prepared stack pointer before first `iretq`.

The x86 review also found no per-thread ownership of FP/SIMD or user FS/GS
bases. Clearing that state only for a new thread would corrupt the previous
owner. The correction therefore adds an aligned, initialized 512-byte FX state
and both user TLS bases to each pinned context; every switch saves/restores
them. Initial x87 register payload and XMM registers are zero with default
control words and empty x87 tags. CPU setup establishes the required FXSR/SSE2
contract, clears EM/TS and enables OSFXSR/OSXMMEXCPT. OSXSAVE stays disabled:
AVX/extended XSAVE state is not exposed without a corresponding owner. Kernel
code retains its software-float build contract. This is eager state ownership,
not a lazy-FPU scheme or a general microarchitectural side-channel mitigation.

### SEC-25: bounded TCP/IP progress and frame storage

At the baseline, `tcpip` receives until its shared endpoint becomes empty before
returning to packet polling, socket cleanup, shutdown checks and CQ handling.
Each authenticated router frame is copied into an unbounded `VecDeque` and
acknowledged before smoltcp consumes it. Bounding outstanding router calls does
not bound that acknowledged backlog. Protocol time advances by a nominal timer
interval rather than actual time lost while processing requests. Whether one
particular workload maintains permanent pressure requires runtime measurement.

Correction: each reactor cycle handles at most 16 IPC requests and at most one
ring-capacity of CQ entries. Receive admission caps both frames (32) and bytes
(64 KiB), rejects before copying and prepares vector/queue capacity fallibly.
Congestion replies `ERR_WOULD_BLOCK`; the frame and request attachments are
released by their existing owners. Monotonic protocol time is sampled from the
kernel counter/frequency, independent of cookies, wakes or cycle count. Invalid
clock state fails closed. Socket expiry/close, DHCP/transport timers and periodic
assignments use that sampled time. Diagnostic status exposes queue drops,
monotonic milliseconds and reactor cycles. This bounds work/backlog, not
per-client CPU fairness or guaranteed progress through an unresponsive NIC.

## Validation

All three corrections are implemented. Final checks:

| Check | Result |
| --- | --- |
| `scripts/run-host-tests.sh` | Passed; the production adapter tests cover frame count/byte saturation, 4,096 sustained refill cycles, drain/reuse and delayed/frequent actual-clock samples. Existing runtime, services, protocol, authorization and signing suites passed. |
| `scripts/build-catten-services.sh --embed` | Passed; staged and signed AArch64 services include the new pressure-client mode and bounded TCP/IP reactor. |
| x86 service bundle | Rebuilt and signed through the x86 runner. |
| Kernel Clippy | `-D warnings` passed for both custom x86 and AArch64 targets. |
| AArch64 service Clippy | `--bins --lib -- -D warnings` passed, including the adapter dependency. |
| Formatting and whitespace | `cargo fmt --all -- --check` and `git diff --check` passed. |
| Isolated x86 guest, four LPs, no network | **15 passed, 0 failed, 0 pending**. First-entry snapshots passed; nonzero x87/XMM and user FS/GS survived a timer wait and another domain's first entry. All six actual ring-3 fault cases retired their domain while the verifier and other suites continued. |
| Isolated AArch64 security guest | **19 passed, 0 failed, 0 pending**. Initial GPR/SIMD/FP snapshot passed. Two scoped TCP/IP CALL clients submitted 1,637 and 1,690 requests while protocol time advanced at least 1,000 ms and reactor cycles advanced, then retired successfully. Both authorization probes reported `0xffff`; publication generations advanced through 1 and 2. Concurrent grant-cancellation traffic retired after 4,800 requests. |

Final guest commands reused the already rebuilt signed bundles:

```sh
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance followup-security-20261005-final --fresh-storage --timeout 160

CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18385 \
CATTEN_DEPLOY_HOST_PORT=17685 scripts/run-aarch64.sh --security-test \
  --instance followup-security-20261005-approved --fresh-storage --timeout 150
```

The initial x86 fixture incorrectly used `map_page`, which zeroes backing,
after filling its code page. Its contained fault left the other 14 tests
passing, but the new verifier correctly timed out. The fixture now uses
`map_existing_page` behind `PreparingUserBacking`. An initial ARM fixture
had a store-pair immediate outside its encoding range; separate stores
corrected it. Neither unsuccessful run is counted as passing evidence. A
sandbox ARM launch also failed to bind forwarding ports; the approved isolated
rerun above passed. Existing VMs and stores were not stopped or reset.

Logs:

- `/private/tmp/charlotte-followup-host-final.log`
- `/private/tmp/charlotte-followup-clippy-x86.log`
- `/private/tmp/charlotte-followup-clippy-arm.log`
- `/private/tmp/charlotte-followup-clippy-services.log`
- `/private/tmp/charlotte-followup-guest-x86-final.log`
- `/tmp/charlotte-x86-followup-security-20261005-final-serial.log`
- `/private/tmp/charlotte-followup-guest-arm-approved.log`
- `/tmp/charlotte-followup-security-20261005-approved-serial.log`

These are implementation regressions, not a physical packet-flood campaign,
a hardware-abort test, or exhaustive multi-LP migration/exception injection.
Kernel-origin classification is tested without deliberately panicking the guest.
Allocation-failure branches in RX admission are source-reviewed; no allocator
fault was injected there. No dependency version or lockfile change was needed.


## Remaining scope

The earlier SEC-07 resource-admission and SEC-18 bulk physical-retirement gaps,
management/peer authentication, protected production provisioning and security
time remain open as documented. A finite IPC cycle does not provide per-sender
fairness. RX pressure fixtures do not establish behavior under every physical
NIC/DMA failure. x86 FX state excludes AVX/XSAVE extensions until separately
owned and tested. No speculative-execution or comprehensive crypto-state audit
is claimed.
