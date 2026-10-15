# Thread-exit watch preparation outside scheduler serialization

Date: 2026-10-15. Baseline `3fd7d15`, the committed audit/diagnostics batch.
Scoped SEC-07/18 progress for C16, R7-1/2 and G1/G4. No numbered finding changes
disposition; historical exit-watch/lease/persistence causation remains open.

## Source review and finding

The next review traced the production thread/stack entry points rather than
adding another recovery registry. The following boundaries remain distinct:

| Call chain | Current ownership/context observation | Disposition |
| --- | --- | --- |
| Pinned reaper → `reap_dead_threads` → `retire_stacks` | Public entry rejects masked callers, selects the current LP's head and checks executing SP/Arm CPU ownership. Complete nodes detach before metadata notification and physical work. | Existing scoped reaper proof; arbitrary callbacks and every wider destructor remain separate. |
| `publish_thread` rejection → `release_unstarted` | Rejected complete thread leaves local lifecycle/publication/table guards before one-shot cleanup. | Caller-owned outer masks/guards remain required; local unlock alone is insufficient. |
| `try_spawn_thread_on_lp` / `sys_spawn_thread` submission rejection | Exact generation is checked when extracting the never-admitted thread; cleanup follows table/scheduler unlock. | Constructor/loader/syscall outer contexts are not all qualified here. |
| Architecture context constructors → `Stacks::user` / `Stacks::kernel` | Stable context allocation precedes backing. Initial user-page and kernel-allocation rollback retain exact root/slot/reservation on uncertainty. | Kernel-arena metadata allocation and complete upstream contexts remain R7/G1 work. |
| EL0 stack fault → `grow_current_user_stack` → growth preparation | Exact thread generation/ASID is checked; complete parent stack remains borrowed. | Master thread-table write guard still encloses growth allocation/mapping and ordinary rollback. This remains open; no context is detached from a running thread in this batch. |
| Thread join → completion event submission → scheduler exit observation | Completion submission/observer creation precedes scheduler registration, but scheduler holds the master thread table while source/entry allocation occurs. | Concrete allocation-under-serialization defect corrected below. Completion-registry allocation/destruction is a separate outer boundary. |

In the last row, `observe_thread_exit_matching` held `MASTER_THREAD_TABLE.write`
through `Thread::try_observe_exit`. `ExitSource::register` could then allocate
a charged observer-list Arc and charge-holder Arc under its own source mutex,
followed by a boxed registration entry under the still-held thread table.
These are fallible allocations, but fallibility does not make their guard
context safe. This is source-supported context remediation, not a reproduced
deadlock or an explanation of an earlier intermittent timeout.

## Preparation and allocation-free publication

Registration now has three phases:

1. Capture and authorize the thread's exact generation and optional existing
   source under a short table read guard. Missing/stale/foreign targets reject
   before preparation. Source lookup only clones an existing Arc.
2. After table/source guards leave, prepare one `PreparingExitWatch` containing
   possible source storage and a `PreparedRegistration` with its charged entry
   and weak observer. Existing sources reuse their backing. New source
   classification comes from the existing captured watch charge.
3. Reacquire the table write guard, revalidate that exact generation and caller
   domain, then relink prepared source/entry storage without allocation, free,
   callback or charge refund. A competing first registration supplies the
   winning source. Losing preparation remains owned until after table unlock.

Retirement removes the thread under the same write guard as final publication,
closing the lost-exit window. Even a legacy TID-only observation binds the
generation captured before preparation; it cannot silently attach to a later
occupant. Disappearance/reuse returns `InvalidThread`, retaining the completion
adapter's existing terminal-result interpretation. Preparation/source admission
rejection remains `WaitRegistrationFailed` / `WouldBlock`, never absence or a
fabricated completed watch. There is no wait retry, replay or deadline change.

The generic list's existing `register` path uses the same prepared-entry linker;
other source adapters retain their existing outer-context limitations. Direct
registration on an unpublished worker also prepares outside the exit-source
mutex. No new registry, heap policy, capability ABI or protocol grammar is added.

## Scoped allocation inventory

This inventory covers the scheduler's exit-watch source/entry, not all kernel
allocation sites or the containing completion transaction.

| Allocation | Bound, sponsor and owner | Rejection/disposal and remaining limit |
| --- | --- | --- |
| `ObserverList<watch_budget::Charge>` Arc backing | One retained initialized source per thread, plus admitted in-flight first-source candidates; shared observer-list pool caps all lists at 8,192 total / 6,144 ordinary. Captured watch classification sponsors the first source. `PreparingExitWatch`, then `ExitSource`/registration tokens retain it. | Fallible before thread-table publication. A competing unused list frees after table unlock. Existing charged allocator retains list admission through actual Arc/Weak backing lifetime. General heap byte/aggregate-principal limits remain R7-3. |
| `ChargedAllocator<list_budget::Charge>` private Arc holder | One fixed-layout holder per new charged list; included in that same list's lifetime owner. | Allocation failure returns its original charge. Shared holder destruction/refund follows the allocator contract; this is not a separate unbounded cache. |
| `Box<EntryNode<watch_budget::Charge>>` | Fixed layout containing ID, weak observer and next charged entry. Watch limits: 128 per thread, at most 1,024 per observing domain, 8,192 node / 6,144 ordinary shared with endpoint/callback watches. Original domain charge travels with the entry through detach/notification. | Prepared fallibly before table lock. Capacity/closed/serial rejection retains unused preparation until post-table disposal; successful publication consumes it once. Cancellation and notification retain existing exact list token/entry ownership. Nested callback/observer allocation is outside this inventory. |
| `CompletionExitObserver` and completion/authority records | Prepared by the existing completion transaction before scheduler entry; exact completion identity and original admission remain retained. | Their own registry/Drop/heap and byte/principal contexts are not qualified by moving scheduler watch storage. |

All layouts above include their ordinary Rust Arc/Box metadata; no claim of a
general heap-byte ceiling or allocator fragmentation bound follows from count
budgets. Cancellation removes the entry under its independent list guard and
destroys it after that guard leaves, but each outer cancellation/notification
caller still needs its own context review.

## Regression evidence

Serialized boot fixtures cover:

- Injected preparation rejection with table reentry, no source entry or retained
  watch charge, and the distinct admission-error result.
- Removing the captured never-admitted target during preparation and creating
  a replacement in the same numeric TID slot. Both TID-only and generation-bound
  observations reject the old episode and leave the replacement unwatched.
- A second watch winning first-source publication during preparation. Both
  entries use that winner; the losing list returns its original list charge.
- Foreign-domain denial before the preparation callback can run.
- Actual prepared publication and closed-source rejection while both the heap
  allocator and thread table are held. Neither path allocates/frees metadata or
  refunds its charge there; rejection retains the complete preparation until
  guard exit. These synthetic targets were never scheduler-admitted.
- Zero fabricated callbacks, no remaining weak observer references and exact
  local watch/list count restoration at the serialized checkpoint.

A live scheduled fixture reenters the thread table during preparation under
both enabled and caller-masked IRQ policies. It uses blocking lock reentry,
allowing real remote contention rather than assuming a global try-lock must
succeed. It preserves the original IRQ policy and cancels each exact token.
Global counter equality and forced TID reuse remain serialized fixtures, not
claims about unrelated live LP work.

Existing full-capacity rejection, 512 cancellation cycles, generation fencing,
reentrant exit notification, namespace teardown/owner-install races, watched
self-exit and worker-completion tests remain enabled. The previous park and
first-entry regression controls remain enabled too.

## Execution and checks

The bounded plan uses fresh four-LP ARM/SMMUv3, Intel VT-d and AMD-Vi no-network
guests with 180-second host windows, followed by ARM/x86 security profiles at
the same limits after those pass. Automatic failure capture stays enabled.
All five runners return 0 with authoritative zero failed/pending results.

| Fresh guest | Serialized preparation probes | Scheduled preparation/IRQ probes | Authoritative result |
| --- | --- | --- | --- |
| ARM/SMMUv3, no network | 1.175354 s | 7.498490 s | **17/0/0**, 10.088408 s, bitmap `0x1ffff` |
| Intel VT-d, no network | 1.123518 s | 17.790379 s | **15/0/0**, 20.581172 s, bitmap `0x1bbff` |
| AMD-Vi, no network | 1.331557 s | 17.663578 s | **15/0/0**, 20.599663 s, bitmap `0x1bbff` |
| ARM/SMMUv3, security | 1.793075 s | 9.792071 s | **19/0/0**, 57.541611 s, bitmap `0x2003ffff` |
| Intel VT-d, security | 1.105528 s | 14.196176 s | **17/0/0**, 62.461499 s, bitmap `0x2003bbff` |

Each guest also completes 64 scheduled IPC claim/endpoint close episodes,
four first-entry controls and four forced Ready-producer controls: **320 / 20 /
20** across this batch. All three no-network guests pass durable voter process
restart to term 2. ARM security passes policy publication generations 1/2 at
35.327916 / 57.536039 s and retires 14,852 concurrent cancellation requests.
x86 security passes those generations at 40.136337 / 62.447312 s and retires
14,820 requests. Scoped denial, independent roots and raw-frame controls remain
part of those authoritative security suites. No kernel panic, historical
exit/lease timeout, persistence fail-stop or failed native run occurs here.
No extra passing repeat is used to replace a failed sample.

The full host suite passes **488 Rust tests, zero failed, four ignored**, plus
existing boundary checks and thirteen automatic-diagnostics tests. These new
kernel fixtures are native execution evidence, not additional host Rust tests.
Strict kernel Clippy passes both custom targets with `--no-default-features
--features acpi -- -D warnings` and their staged bundle variables. Workspace
rustfmt, diff whitespace and local documentation targets pass. Boot runners
verify executable/read-only native assembly sections. No userspace source,
signature, ABI or stored-format changes are needed; existing signed bundles
are reused from the [persistence diagnostics batch](2026-10-15-raft-persistence-diagnostics.md#artifact-identity).

Commands use `CATTEN_SKIP_EMBED_BUILD=1` and
`CARGO_BUILD_RUSTFLAGS='-D warnings'`. Both no-network kernels are rebuilt;
the x86 image is prepared with `--build-only` before running Intel and AMD
with `CATTEN_SKIP_KERNEL_BUILD=1`. Security kernels are separately rebuilt
with their current fixture inputs, with `CATTEN_ALLOW_SANDBOX_NETWORK=1`.
ARM security uses HTTP/deployment host ports 18880/18881; x86 uses 18980/18981.
Each instance has separate fresh test storage.

```sh
./scripts/run-aarch64.sh debug --smp 4 --no-network --fresh-storage --timeout 180 --instance exit-preparation-arm
./scripts/run-x86_64.sh debug --smp 4 --iommu intel --no-network --fresh-storage --timeout 180 --instance exit-preparation-intel
./scripts/run-x86_64.sh debug --smp 4 --iommu amd --no-network --fresh-storage --timeout 180 --instance exit-preparation-amd
./scripts/run-aarch64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance exit-preparation-arm-security
./scripts/run-x86_64.sh debug --smp 4 --security-test --fresh-storage --timeout 180 --instance exit-preparation-x86-security
```

| Executed artifact | SHA-256 |
| --- | --- |
| ARM no-network kernel | `3af22433e1a1ee0f59bb406bd1bffdd67c48a28ab2150832908c982ac181d4ee` |
| Intel/AMD no-network kernel | `07e1b605b6dee1dd2a4466d844031e4c4f404a5318f4d11e1ab06c7bc4866815` |
| ARM security kernel | `cb94f0e90a59a5179c8c7fb3c829b0dfb34a02ae3df2f0c855f9316cbb617bfd` |
| x86 security kernel | `400f5939949c255b5e998c7e646bb7d3eae87398d264bac44590780146a23499` |
| ARM no-network final UART | `f93b40938e296ec1a60c7d0ad4ca13ac66e483a77e2d5f2ba784f29c9c3dde77` |
| Intel no-network final UART | `6d4bc4df395bd73a5f523498e6401b083679affe4287d3016e651a5f1eb6d533` |
| AMD no-network final UART | `d99fb5f15855cf27d13f5a3c4d744de7b06af6607ddf5df0c082cc69b49f8d2c` |
| ARM security final UART | `d01ecefbd6c8c678a1541e5bea93f58b53b42cc24952c282dd4e76fa5a5fd7fa` |
| x86 security final UART | `eb4bbbf50a3953b5be345c33e1fc20be725115a62672d6ebc3cbfef8dae2eaf8` |

Native run manifests retain the actual boot-image ELF and signed inputs.
Results and qualifications above are inline, independent of ignored artifacts
or temporary log links. Passing runs do not attach LLDB, and supply no live
failed-run snapshot or causal attribution for historical incidents.

## Limits

No original intermittent IPC/memory-close/stack/persistence failure is attributed
to this allocation context. Reaping/physical pair release, root/stack/backing
charges, quiescence gates and terminal retention policy are unchanged. There is
no new cleanup receipt or retry authority. Wider completion/observer/registry
contexts, growth beneath the thread-table guard, kernel-arena metadata,
aggregate heap/principal admission and integrated outstanding-I/O/pressure
qualification remain open. This does not complete R7-1/2, R18-1 or G1/G4/G7.
