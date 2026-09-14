# Kafka soak scratch-window exhaustion

Date: 2026-09-14

## Incident

An AArch64 Kafka soak stopped making progress after roughly 45 minutes. The
client's last healthy report showed 24,471 produced and 24,469 consumed
records; its next produce request expired after 30 seconds. QEMU and the kernel
remained alive, and no kernel panic followed the application timeout.

The serial log, packet capture, QEMU monitor, and live debugger agreed on the
failure boundary:

- all four virtual CPUs continued running the kernel idle loop;
- timer and wake activity continued, excluding a global scheduler deadlock;
- both established Kafka TCP flows stopped acknowledging frames at the same
  time, and new connection attempts received no SYN response;
- the frame router continued receiving and forwarding IP frames;
- TCP/IP's receive count advanced while its transmit and broker counters
  stopped; and
- the broker heartbeat remained at a fully completed request boundary rather
  than inside request processing.

## Root cause

`memory_map_any` allocated virtual ranges from a 512 MiB per-address-space
scratch window by advancing a high-water cursor. `memory_unmap` removed page
table entries but never returned the corresponding virtual range. The limit was
therefore accidentally a lifetime limit of 131,072 mappings, not a limit on
concurrent mapped memory.

Each Kafka exchange maps memory several times while moving a frame through the
frame router, TCP/IP service, and broker. Near the failure the counters imply
about 128,000 one-page hot-path mappings, with deployment, status, DHCP, and
other traffic accounting for the balance. Once the cursor reached the end,
`memory_map_any` returned `OutOfScratch` permanently.

TCP/IP then obscured the cause: its `OP_FRAME` handler closed a frame whose
mapping failed, incremented the receive counter, and replied success. This made
the network services appear alive while every incoming TCP frame was actually
dropped.

## Corrections

The scratch window is now a generation-aware extent allocator:

- successful unmap returns a range only after page-table removal and
  cross-processor TLB invalidation;
- adjacent free extents coalesce, while a free extent at the high-water mark
  contracts the window;
- a failed map returns its reservation after rollback and invalidation;
- failed rollback or unmap quarantines the range instead of risking a virtual
  alias;
- address-space teardown discards that generation's allocation state and
  returns successfully removed mappings in other live address spaces; and
- MMIO `map_any`, unmap, close, and rollback obey the same rules because MMIO
  and ordinary memory share the scratch window.

TCP/IP now counts only frames it actually accepted. Mapping failures increment
`RX_MAP_ERRORS`, retain the last raw status, appear in the serial heartbeat as
`rx_map_err=count:last_status`, and produce a negative frame reply. The frame
router counts such a reply as a dropped frame without discarding an otherwise
healthy protocol route.

## Validation

The AArch64 memory-object self-test now proves adjacent extent coalescing and
reservation rollback after a rejected mapping. The device self-test proves
that an unmapped MMIO scratch address is reused. The target-aware kernel and
userspace Clippy runs pass with warnings denied.

An isolated four-vCPU AArch64 boot using the corrected kernel completed all 17
registered deferred self-tests. This verifies the allocator and MMIO lifecycle
paths in QEMU. Repeating the multi-hour Kafka workload remains the endurance
test for the original symptom; the retained failed guest still contains the
old kernel and was deliberately not replaced during diagnosis.
