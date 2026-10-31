# Minimal boot idle activity — 2026-10-31

A minimal ARM image still did substantial periodic work after boot. Guest
debugger snapshots showed that all four CPUs reached `lp_idle_loop`'s `WFI`;
the measured activity came from operational service scheduling rather than a
continuously runnable idle loop. Removing regression fixtures does not remove
polling in operational services.

## Measured baseline and changed behavior

Both images used the optimized debug profile, four Cortex-A710 CPUs, 512 MiB,
TCG, SMMUv3, NVMe and VirtIO networking under QEMU 11.1.2. The baseline was
revision `9c84aab6`; the changed image used the wait policies described below.
No external workload was submitted during the intervals. Normal DHCP, NTP,
discovery, Raft, telemetry and deployment reconciliation remained active.

| Service | Baseline dispatches/s | Changed dispatches/s |
| --- | ---: | ---: |
| Deployment agent | 114.78 | 8.38 |
| Object storage | 109.72 | 2.49 |
| NVMe | 97.03 | 3.90 |
| Network driver | 94.67 | 83.58 |
| DNS/Raft | 54.35 | 48.37 |
| All userspace services | 518.63 | 190.48 |

Completed userspace on-CPU intervals, including their syscall execution,
accounted for approximately 21.86% and 5.78% of one CPU respectively. LP0
anonymous plus keyed timer firings fell from 520.55/s to 283.13/s. Host `ps`
displayed 34.9% for the baseline process and 7.7% for the changed process.
These host readings are observations, not a controlled benchmark or a fixed
performance guarantee.

The complete accounting intervals were 24.506764 and 30.533888 seconds.
The [machine-readable capture](2026-10-31-minimal-idle-profile.json) preserves
kernel SHA-256 identities, source identities, captured thread generations,
before/after counters, verified 62.5 MHz counter frequency and measurement
limits. Only completed thread intervals are counted; the in-progress interval
at each stop is excluded. Debugger stops perturb execution, and host load
varied. The original VM exited before a second simultaneous-control snapshot,
so the earlier complete baseline interval supplies the comparison.

## Rules that remove idle work

- Bind the object-store endpoint before registration, drain requests and wait
  on its default CQ. Endpoint arrivals and lifecycle publication already wake
  that queue, including arrivals between draining and parking.
- Keep one monotonic deadline for the agent's reconciliation delay. Lifecycle
  publication wakes the default CQ, so a 500 ms delay need not allocate and
  wait on fifty 10 ms timers. Unrelated notifications recheck lifecycle without
  finishing or restarting that delay. This helper requires a sole CQ waiter.
- A controller with zero submitted commands cannot miss a completion for an
  outstanding command. NVMe can wait event-only in that state and retain its
  10 ms recovery watchdog while commands own DMA/reply state. Background
  telemetry and catalog persistence can still produce genuine storage I/O.

The network receive fallback remains: external packets can arrive while the
transmit queue is empty. DNS/Raft's protocol timer and the scheduler's 50 ms
idle-wake recovery timer also remain. Neither this profile nor a successful
QEMU run qualifies removal of their missing-notification recovery contracts
on other platforms. Current contracts live in [checked CQ waits](../../reference/cq-wait.md).

## Validation scope

The existing cooperative shutdown fixture now waits through the shared helper
with a 60-second delay, much longer than its retirement grace. Its existing
shutdown checks therefore exercise lifecycle interruption of that wait rather
than a separate short polling loop.

Host runtime and service suites pass 54 and 265 tests. The first sandboxed run
passed the runtime suite and 262 service tests; three OpenSSL interop tests
could not bind their local sockets. The unrestricted repeat passed all three.
Clippy passes for NVMe, object storage, the agent and shutdown probe on both
architectures with warnings denied; workspace formatting also passes.

| Native release run, four CPUs | Passed | Failed/pending | Additional host PASS groups |
| --- | ---: | ---: | ---: |
| ARM storage/network/security | 20 | 0/0 | 21 |
| Intel storage/network/security | 18 | 0/0 | 21 |
| ARM isolated shutdown | 1 | 0/0 | 0 |
| Intel isolated shutdown | 1 | 0/0 | 0 |

Both shutdown runs reported `production object store and 3 device adapter(s)
flushed/reset and quiesced` and `SUCCESS: bounded domain retirement and
reverse-order service drain verified`. This exercises shutdown of the actual
event-waiting store/controller as well as the long-delay probe. The capture
preserves each run's kernel/bundle identities, terminal bitmap verdict and
relevant diagnostics. The measured minimal boot completed memory validation,
signed bootstrap seeding and normal local-ready publication.
