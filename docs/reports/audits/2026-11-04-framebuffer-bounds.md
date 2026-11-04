# Framebuffer bounds and ARM deadline capture — Q3

Date: 2026-11-04–05. Base: `5228a5a4`. This extends the
[boot allocation contract](../../reference/boot-allocation-boundaries.md),
without accepting Q3 or changing finding dispositions.

The optional console now validates RGB32 format, nonoverlapping 8-bit channels,
pointer/stride alignment, row coverage, complete glyph geometry and checked
size/address arithmetic before C entry. It fixes the existing 1/2/4 font scale
explicitly and checks all six C allocation layouts against a **12 MiB**
requested-byte ceiling. The framebuffer span is independently limited to
**256 MiB**. Ordinary 8K modes fit without cropping; excessive cell catalogs or
padding can reject. Actual mapped backing remains the trusted bootloader's
responsibility, and allocator overhead/aggregate capacity remains Q4. No
runtime pool ceiling, physical heap reserve or production deadline changed.

The full host suite, workspace formatting and strict display minimal/test
checks on ARM and x86 pass. Host controls reject 29 malformed/over-limit cases
before allocation or C entry, exercise both limit boundaries, compare actual C
allocation sizes against the plan, and preserve both failure-prefix cleanup and
inert abandonment. Minimum, padded-row, alternate-RGB and 2x/4x rendering keep
guard words and padding intact. Unsafe owner and removed-byte-guard controls
both reject. Native x86 display execution is not claimed.

## Failed native campaign

The first fresh four-CPU ARM display/security campaign accepted the real
800-by-600 framebuffer at scale 1 with **230056 requested heap bytes**. Live
handoff and persistent upgrade completed. Eighteen verifiers passed; the
remaining scoped-launch verifier failed its second adversarial grant probe's
unchanged 45-second deadline. The 240-second runner deadline was not reached.

The exact kernel SHA-256 is
`01b1e83cd866a095610bcffccd64f144fd3d40109c4bebdf397a735e94123ebb`.
Relevant original diagnostics:

```text
[60.416185] adversarial grant checks passed in 40187 ms
[84.724078] standalone entropy rollover passed in 18981 ms; domain retired
[127.035319] controller RPC: scalar/move deadlines, eight queue slots,
             rejected/cancelled/returned page charges and healthy status passed
[130.408235] fixture deadline root=170:10
             phase=adversarial application grant checks budget_ms=45000
             stage=1 checks=0x0 completed_requests=0
Kernel panic: self-test deadline expired while waiting for adversarial application grant checks
```

The debugger captured the verifier panic stack and three idle LPs. The status
bitmask is only written when the probe completes, so zero does not identify its
last completed resource check. This capture establishes a deadline failure,
not its cause, a cleanup replay right or attribution to framebuffer admission.
The [self-contained evidence](evidence/2026-11-04-framebuffer-bounds.json)
retains artifact identities, phase diagnostics and debugger frames independently
of ignored local logs. Earlier ARM exit-watch and Intel retirement-pin incidents
remain separate.

## Controlled repeat and remaining work

The repeat reused the identical kernel, signed service bytes and initial disk
contents with fresh storage and unchanged deadlines. It also failed the second
45-second probe, at root `177:9`, and reported **18 passed/1 failed**. Its first
probe took 40.017 seconds; its second logged controller-RPC completion at
125.322009 seconds before the 129.057070-second deadline.

A third control reused those same three byte identities but omitted QEMU's
`ramfb` device. The kernel still contains `display`; the bootloader supplied no
framebuffer, so the existing ARM serial fallback ran. This passed **19/19**,
with probe times **32.664 and 32.673 seconds**. The comparison establishes a
configuration-dependent overrun, not its exact resource phase or causal
mechanism. Removing drawing also removes its boot allocations/device, so this
does not isolate drawing latency alone. No deadline was extended and no
passing run resolves either failed episode. The full native display campaign
is **not accepted**; framebuffer initialization/local host controls are
qualified separately.

The current contract also assigns concrete boot/per-LP factories to bounded
boot storage, unused helpers or required fixes. Next identified Q3 batches are
explicit initialization of runtime-reachable timer/IRQ storage and bounded
MCFG/PCI traversal. Other producer/dependency assignments, aggregate sizing and
caller/IRQ context qualification remain open.
