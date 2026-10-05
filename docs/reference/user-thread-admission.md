# User thread admission and stacks

User-thread construction is fallible. `SPAWN_THREAD` rejects invalid user entry
ranges, invalid LP numbers, exhausted stack/thread admission, physical backing
failure and context/publication allocation failure with `(u64::MAX, 0)`.
`catten_rt::owned::ThreadHandle::spawn` returns `Result<ThreadHandle, ThreadError>`;
`SpawnFailed` leaves the caller running. Trusted mandatory fixtures retain a
panic wrapper; fallible supervisor launch paths retain the unstarted namespace
owner until construction/publication succeeds and roll it back on rejection.

Each user root embeds a 64-bit stack-slot bitmap. Preparing, published and
retiring stacks count toward its signed/adaptive `max_threads` limit. Slots
are selected under the address-space table guard before allocating any stack
backing. This includes concurrent preparations, rather than only threads that
have reached the master table. There is no global monotonic stack index.

The exclusive arena is `[0x01000000, 0x02040000)`: 64 slots, each containing up
to 64 pages and one guard page. Memory-object mappings, MMIO mappings and ELF
layout validation exclude the entire arena, including unused slots/guards.
The architecture walkers still permit trusted stack mappings within it.

`PreparingStackPage` owns the exact `StackSlot` root lease and a zeroed,
fallibly admitted physical page until leaf publication. The physical progress
floor applies before allocation. Mapping interruption quarantines both owners;
confirmed mapping rejection drops them. Each architecture's context then owns
the published slot and committed stack range. Teardown detaches leaves under
the table guard, releases the guard, invalidates translations, and releases
backing before returning the slot. Failed cleanup retains the original slot
and root lease. Demand growth remains within the admitted per-thread budget.

The master thread records the captured address-space handle, independently
from its reusable numeric ASID/TID. Publication revalidates that handle and the
abort fence. EL0 exit-watch authorization compares that exact identity under
the same thread-table lock as generation checking and source registration.
Zero expected thread generation permits only a current same-domain target.
Trusted supervisor watches use a distinct internal adapter.

These per-domain limits do not establish whole-node thread/control-block,
translation-table or kernel-heap admission. Existing x86 cross-LP teardown
limitations remain documented separately. See the
[renewed audit remediation](../reports/audits/2026-10-05-security-remediation.md)
for regression evidence and validation scope.
