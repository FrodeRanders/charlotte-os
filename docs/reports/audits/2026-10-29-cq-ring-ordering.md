# CQ cursor ordering and consumer ownership audit — 2026-10-29

The remaining shared-CQ ordering gap is fixed in the CharlotteOS SDK and kernel.
Sitas's safe reader serialization and atomic cursors were already implemented in
`6d63266eb52492c1958c45f3ed71c8c7a451489a`. CharlotteOS's raw SDK reader was
already `unsafe` and required one consumer; this audit makes that requirement
explicit for mixed SDK/Sitas readers and fixes the matching producer protocol.

## Findings and changes

The SDK previously loaded cursors with volatile accesses and placed its release
fence **after** publishing `tail`. That did not order the payload copy before
slot reuse. The production kernel producer also read `tail` without an acquire
operation. Kernel fixture helpers used the same volatile cursor pattern, with
some acquire fences preceding their loads.

Both wire-compatible header definitions now use `AtomicU32`. The protocol is:

1. The serialized producer acquires `tail` before reusing a free payload slot.
2. It initializes the record, then publishes `head` with a release store.
3. The sole consumer acquires `head` before copying the record.
4. It finishes that copy, then releases `tail` for producer reuse.

Payload copies remain volatile. Their visibility and exclusive slot ownership
come from the cursor atomics. The kernel retains its private producer cursor
and capacity; userspace-writable mirrors never establish kernel bounds. The
header is still 16 bytes, with cursor words at offsets 0/4, capacity/overflow at
8/12 and 32-byte records beginning at offset 16. Runtime ABI 3 and syscall 88
are unchanged. The SDK rejects misaligned bases and invalid capacities/indices
before reading payload or advancing the consumer cursor.

`cq_read` remains an unsafe ABI boundary: the caller retains the mapping and
excludes concurrent consumers. Sitas's safe readers share an internal consumer
lock, but that lock does not cover independent SDK reads. Mixing those APIs
requires external serialization. The new guest fixture alternates readers on
one thread after all shard workers join. CQ records remain hints; the probe
observes each retained timer owner through `Completion::poll`, and never adopts
capabilities from stale notifications. Readiness waits retain one monotonic
two-second budget across unrelated wakes and keep the timer owner alive through
wait failure and cleanup.

The kernel's production/SDK fixture verifies complete record fields through
128 slot reuses and checks that consuming a CQ record does not consume its
operation capability. The signed Sitas fixture requires 64 timer notifications,
alternating SDK and Sitas readers, in addition to its KV/index/join results.
The old adder-only success path cannot bypass this required probe.

## Validation and immutable observations

Host: macOS/AArch64, pinned `nightly-2026-07-27`. Native SDK manifest tests,
invoked from a temporary working directory to avoid bare-metal Cargo config,
passed **7 tests**, including **80,000** concurrent producer/SDK consumer
records over capacities **2, 3, 32 and 127**. ABI layout and malformed-header
rejection tests passed. SDK Clippy with `--all-targets -- -D warnings`, the
`x86_64-unknown-none` SDK check, repository-wide `cargo fmt --all -- --check`
and `git diff --check` passed. No x86 guest qualification is claimed.

The first attempt used instance `sitas-cq-20261029` and stopped at compilation
(exit 101), before a guest boot. Preserve this failure independently of the
successful repeat:

```text
error[E0616]: field `overflow` of struct `CompletionQueueRing` is private
  --> crates/catten/src/self_test/completion.rs:119:38
```

The remaining fixture now calls `overflow_count()`.

The successful repeat rebuilt the signed service bundle and Sitas image, then
built the debug/headless AArch64 kernel with its normal `acpi` feature. No
embedded-build skip or stale-storage override was used:

```sh
CATTEN_HTTP_HOST_PORT=18681 CATTEN_DEPLOY_HOST_PORT=18445 \
  ./scripts/run-aarch64.sh --instance sitas-cq-20261029-2 \
  --fresh-storage --timeout 180 --smp 4
```

The runner exited **0** after observing authoritative completion. Serial
observations, retained here rather than depending on temporary logs:

```text
[+     8.102982] [sitas] SUCCESS: basic_kv total_len=3, mailbox index verified 1024 records, join probe sum 81, SDK/Sitas CQ probe verified 64 records
[+     8.159938] [CQ contract] production producer / SDK consumer: 128 wrapped records verified
[+    11.722326] SELFTEST COMPLETE: passed=17 failed=0 pending=0 passed_bitmap=0x1ffff failed_bitmap=0x0 pending_bitmap=0x0
>>> All registered deferred self-tests passed.
```

Qualified inputs and artifacts (SHA-256):

| Input/artifact | Identity |
| --- | --- |
| CharlotteOS base revision | `2ae4f97d98c3f71e887d5d03320c96a350f56464`, with this CQ patch and the existing working-tree edits |
| Sitas revision | `6d63266eb52492c1958c45f3ed71c8c7a451489a`, clean working tree |
| Debug AArch64 kernel ELF, 77,561,328 bytes | `e17c0207b545a0bbc1a49adeea659cf6a94405e6e024eabd84250df74b61da7c` |
| Signed staged Sitas ELF, 165,064 bytes | `499b3f652c80bc1218ec3df7d505e12f3d30f247f4a054f7b078820174664f14` |

The guest used the repository's development signing policy. The 21 pre-existing
modified tracked files were verified byte-for-byte unchanged. This is local
integration evidence against Sitas `6d63266`; CI still pins the previously
published Sitas revision. Publishing Sitas and then advancing that pin is a
separate repository-integration step. Finite stress/guest runs exercise the
protocol; the acquire/release ownership argument establishes its ordering.
