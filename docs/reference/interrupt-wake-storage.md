# Deferred device-interrupt wakes

An IRQ masks its source, increments its pending/lifetime counters and marks
readiness for the bound driver. The interrupt handler neither allocates nor
enters a kernel registry. Cooperative yields and idle loops deliver readiness
from thread context.

## Storage and coalescing

Each routing-table slot has one statically initialized 64-bit mailbox. The
current tables have 288 slots on AArch64 (2,304 bytes of mailbox storage) and
476 on x86-64 (3,808 bytes). These figures exclude the existing route tables
and interrupt counters. There is no deferred-wake heap allocation or shared
queue capacity to exhaust.

Repeated readiness for the same route lifetime sets the same bit. A source
cannot occupy another source's mailbox, even if it repeatedly delivers before
any thread drains. This replaces a bounded FIFO whose ignored push failure
incorrectly assumed that a full queue already contained an equivalent wake.

The mailbox retains its generation watermark after readiness is claimed.
Management advances the watermark on bind and retirement; a delayed old
publisher cannot overwrite a newer pending wake or resurrect a retired wake.
An atomic claim has one winner, including with multiple LPs draining. A new
publication after a claim remains ready for another pass. A drain makes one
bounded sweep, rather than repeatedly popping while producers refill a queue.
The common clean-slot path loads without writing its cache line. The scan still
costs O(number of routing slots); this change is not a throughput benchmark.

Generation zero is reserved. Binding reserves one further retirement identity
and returns device status `ROUTE_GENERATION_EXHAUSTED` (16) before 63-bit
identity exhaustion. Retirement saturates at the final identity; no old
generation becomes valid again after integer wrap.

## Route lifetime and notification

The drain claims a mailbox, then holds `DEVICES` while checking the captured
generation and publishing to the current CQ. Bind, individual close and
address-space teardown use that same guard. Teardown retires interrupt routes
before releasing their source ownership to a new grant.

`completion::prepare_wake` increments work generation and detaches that exact
CQ's waiter batch under `COMPLETIONS`. The lock order is
`DEVICES → COMPLETIONS → waiter list`. The drain releases all subsystem guards
before notifying the batch. It does not resolve numeric ASID/CQ identifiers
again after detachment. A captured batch can notify its original live waiter
after retirement; it cannot capture waiters from a replacement queue. Ordinary
`completion::wake` uses the same prepare-then-notify path.

Coalescing is readiness, not a completion record or a FIFO ordering guarantee.
The driver must inspect device state and acknowledge/rearm its interrupt.
Pending and lifetime counters retain their existing 32/64-bit semantics.

## Validation boundary

Host tests exercise flooded independent slots, retirement/rebinding, a late old
publisher before and after claim, publication after claim, identity exhaustion,
concurrent stale publishers and competing claimers. Kernel fixtures flood the
real delivery path beyond its former queue capacity, retire/rebind a source and
check both rejection of stale readiness and delivery of a fresh interrupt.
A prepared-CQ fixture reuses the exact numeric namespace, checks that the
replacement's work generation/waiter are untouched and invokes a callback
that reenters completion/device registries. A second callback reenters those
registries through the actual deferred IRQ drain; the fixture waits for any
LP already claiming that mailbox rather than assuming its local drain wins.

These checks concern storage and deferred publication. They do not prove every
cross-LP mask/rearm interleaving, interrupt-controller pending-bit behavior,
physical-device recovery or driver scheduling progress. The existing
`CharlotteInterruptRoute` model abstracts generation fencing for one route;
it does not model mailbox capacity, atomic publication/claim interleavings or
controller MMIO.
