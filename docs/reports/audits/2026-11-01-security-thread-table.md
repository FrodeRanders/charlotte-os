# Master thread-table admission and boot-stack qualification

Date: 2026-11-01. Base revision: `06bdbda449853ee298f4ec5ad10d059a95064c21`,
with the table-admission changes under review. This qualifies a bounded part of
SEC-07 R7-2/R7-3 and SEC-18 C16/G4; the broader findings remain partial.

## Result

Runtime thread publication now admits complete replacement backing before
allocation and leaves lifecycle/publication/master-table guards for buffer
preparation and disposal. Its typed table has no allocating `try_add_element`
shortcut. Every retained, competing and replaced vector set keeps its original
principal and trusted class through actual deallocation. Exact generations,
leases and closing state survive the guarded ownership move. Scalar rejection
leaves the complete thread in its containing preparation; only proven unstarted
capacity conflicts permit bounded re-preparation.

The [current contract](../../reference/thread-table-admission.md) specifies the
byte/slot limits, sponsorship of shared capacity, continuation and abandonment.
Root-table growth, general heap admission, wider caller contexts, sustained
pressure/fairness and guarded-copy latency remain separate work.

The final four-CPU security matrix used fresh stores and the same predefined
240-second runner limit throughout:

| Target | Instance | Passed / failed / pending |
| --- | --- | --- |
| Arm SMMUv3 | `thread-table-20261101-arm-128k` | 19 / 0 / 0 |
| Intel VT-d | `thread-table-20261101-intel-128k` | 17 / 0 / 0 |
| AMD-Vi | `thread-table-20261101-amd-128k` | 17 / 0 / 0 |

Each run records the new real-buffer/signed-sponsor and counter-reserve controls,
the original domain-abort/stack-retirement checks and complete scoped-launch
security verdict. The host production-table suite passes 39 tests, including an
allocator tracer and an allocating unsafe control; the resource-policy harness
passes three groups covering actual counter behavior, invalid profiles and
trusted selector paths. Target Clippy, minimal target compilation, workspace
formatting and whitespace checks pass.

[Machine-readable evidence](evidence/2026-11-01-thread-table-admission.json)
contains kernel and bundle-map identities, exact native receipts and separate
failed/intermediate runs. No temporary log is required to interpret these results.

## Boot-stack failure and capacity decision

Four ARM attempts stopped at exception-vector entry before scheduled tests.
The first retained a by-value rejected-thread cleanup frame; subsequent captures
reached the nested stale-service-page fixture. The initial interpretation of
the four-page request as 16 KiB of actual backing was wrong. The pinned
[Limine 12.9.6 implementation](https://github.com/Limine-Bootloader/Limine/blob/v12.9.6/common/protos/limine.c#L1328-L1347)
uses the larger of 65536 bytes and the request, so changing the request from
16 KiB to 32 KiB did not change actual stack capacity.

The 32 KiB-request kernel
`a6b90e230a04db7d5c06b8539a3103f68154e2e7343fa2249eb1fc5f4553db72`
has these fixed AArch64 frames on the captured path, including saved-register
areas:

| Frame | Bytes |
| --- | --- |
| `bsp_main` | 8960 |
| `run_synchronous_self_tests` | 23216 |
| `thread_admission::run` | 17520 |
| `domain_abort::tests::run` | 12240 |
| `test_rejected_service_pages` | 12096 |
| Total before deeper helpers | 74032 |

That exceeds actual 64 KiB backing by 8496 bytes. The supplemental register
capture has SP `0xffff00005cb75ed0` and TTBR1 `0x5cb75000`: through the HHDM,
the stack is in the active root-table page. ESR_EL1 is `0x86000004`, with FAR_EL1
and ELR_EL1 both `0xffffffff80000200` at exception-vector entry. The fixed-frame
overrun and overlapping root page explain the captured recursive instruction
translation failure. The initial exception syndrome was overwritten by recursive
entry, so that first instruction cannot be reconstructed from this capture.
[Arm's ESR register definition](https://support.arm.com/documentation/111107/2026-09/AArch64-Registers/ESR-EL1--Exception-Syndrome-Register--EL1-)
defines the syndrome fields. The [preserved diagnostics](evidence/2026-11-01-thread-table-arm-failures.txt)
include backtraces, raw registers, original deadlines and all five prologues.

Two alternatives were considered and executed:

- Separating fixture phases and heap-placing large fixture coordinators lowers
  stack demand. That intermediate ARM campaign passed 19/0/0, but it complicates
  fixture ownership and adds fixture allocation dependencies. Those changes
  were removed; the original fixtures and all their assertions remain intact.
- Requesting 128 KiB boot stacks supplies headroom above the measured chain.
  Compared with Limine's actual 64 KiB minimum, the extra fixed cost is 64 KiB
  per LP: 256 KiB at four LPs, or 16 MiB at 256 LPs. This was selected as the
  simpler tradeoff. It does not raise runtime stack quotas or resource admission
  limits. The final matrix above uses this request and the original fixtures.

The scalar-error publication API is retained independently: it avoids unnecessary
large error copies and keeps rejected ownership in place. Its compiled ARM
publication frame decreased from 2304 to 1472 bytes; that reduction alone did
not make the full boot call chain fit the loader minimum.

These captures explain this specific boot-stack capacity failure. They do not
attribute older exit-watch, retirement-pin, signing-process or scheduler
incidents. Debugger stop/restore observations supply no physical completion or
cleanup authority, and passing QEMU campaigns do not qualify physical platforms.
