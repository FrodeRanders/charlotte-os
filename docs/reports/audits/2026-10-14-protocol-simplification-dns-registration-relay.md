# S08b2c3: correlated full-name DNS registration and retraction

Date: 2026-10-14. Baseline: `21d8d6e3` (DMA-map readiness follow-up).
This batch replaces the remaining lossy DNS registration/retraction relays and
updates their callers. It does not close the remaining scalar API inventory.
See the [current contract](../../reference/dns-registration-relay.md),
[batch ledger](../../reference/api-simplification.md) and existing
[C18 owner family](../../reference/cleanup-recovery.md).

## Change and ownership boundary

Current tags `0x28/0x29/0x2a` replace `0x15/0x16/0x14`. Complete u32 identity
lengths replace truncating u8 writers: owners are nonempty and bounded to 4 KiB;
service names are nonempty and bounded to the current 256-byte publication
policy. Decoders validate the entire frame and borrow slices without allocating.
Malformed, empty/oversized, retired/relabelled, missing-generation and trailing
records reject. Deployment generation stays explicit, including zero for system
services. Retraction requires its nonzero exact distributed generation.

Registration replies now carry DNS session and nonreused request ID, matching
the captured leader before any pending record is consumed. The name and local
generation come from that original record, rather than reply-supplied overrides.
Follower request bytes, leader identity and pending slots prepare before transport
publication; pending state publishes first. Leader identities, pending slot and
terminal reply bytes prepare before the first Raft command. PreparedReply moves
through register/activate and abort, patching only generation without another
codec allocation. Catalog-v18, Raft pair-v3, DNS interface 3 and runtime ABI 3
remain unchanged. This is an internal grammar replacement with no negotiation.

Prepare/Activate/RemoteRegister retain Connection and ReplyToken owners from the
unique raw receive boundary. Connections move into LocalPublication without
integer re-adoption. Full names travel in owned copied memory for local lookup,
scoped generation observation and delegated registration, including short names.
Scoped agents still retain their complete DeployedArtifact and acquire no legacy
invocation authority. Immediate/deferred rejection releases the same owners;
returned resources remain in CallResult until disposal. Retraction encoding
rejection retains the existing publication record and one-second retry interval;
its exact owner/generation fence still prevents deleting a replacement.

Service registration now rejects missing local publication before a leader
proposal. Catalog-only markers/events use the explicit memory-carried
OP_EVENT_FIRE operation. Both kernel DNS/HTTP alpha-marker callers were updated;
real echo publication and the follower deployment barrier retain their actual
local service ownership. This removes the leader's empty-resource fallback.

Existing transport/source-MAC binding is not new cryptographic authentication.
Raw close-watch completions, watch admission retry, local unregister adapters,
other catalog/transport vector allocations and broad IRQ/Drop contexts remain
separate qualification. No new registry, custody, wait source, physical proof or
non-idempotent replay authority is introduced.

## Host and build verification

- Final `scripts/run-host-tests.sh`: **457 passed, 0 failed, 4 ignored** Rust
  tests, including five new registration/retraction codec controls. Existing
  signing, boot, secure-boot, syscall, tooling and object-store controls passed.
- Golden layout, distinct full 256-byte names sharing the retired 255-byte
  prefix, exact 4 KiB owners, explicit deployment generation zero, complete
  invalid frames/writers, every truncated prefix, overflowing declared lengths,
  missing suffixes, trailers, zero correlations and retired/relabelled bodies
  passed. Prepared reply completion preserves its original allocation; wrong
  peer/session/request combinations reject through the production matcher.
- Signed ARM/x86 service builds passed. Strict service Clippy passed on both
  architectures; kernel Clippy includes discovery, deployment, cluster control
  and the updated HTTP caller. Native runners enforce assembly permissions.
- Workspace rustfmt/diff checks passed. Contributor instructions, living
  Markdown and LaTeX were updated together. Two final PDF passes produced
  **213 pages**; changed pages **89–90 and 106–107** were rendered and visually inspected.
  This does not claim whole-manual layout repair.

Allocator exhaustion and every live reactor cleanup phase were not injected.
The large identity boundary is codec evidence; native node/service identities
remain short. Ordinary pending-call ownership and resource cleanup keep their
existing runtime test coverage rather than inventing a physical completion proof.

## First linked failure: catalog-only caller mismatch

Fresh four-LP ARM/x86 guests used `--deploy-test` and 300-second guest budgets.
ARM listened on TCP 19142; x86 connected. QEMU PID 33247 was confirmed listening
at x86 boot publication. Both runners ultimately exited 1.

At **25.784237 s**, x86 logged `register result = Some(-1)` for the alpha marker.
The DNS selftest then panicked at `self_test/el0_dns.rs:775` because it expected
a committed generation. Its final result at **25.843755 s** was **18 passed /
1 failed / 1 pending**, bitmaps `0x47bbff / 0x80000 / 0x800000`. Source review
confirmed this caller supplied neither an endpoint nor a local publication and
had depended on the removed leader fallback. The corrected caller uses the
explicit catalog-only event operation; real service registration remains strict.
The HTTP fixture had the same assumption and was corrected too.

ARM subsequently timed out waiting for cluster membership at **132.614084 s**,
with **20 / 1 / 1**, bitmaps `0x47ffff / 0x80000 / 0x800000`. This followed the
peer's panic; it is preserved as a separate observed timeout, not another
identified registration defect.

First-run artifact SHA-256 identities:

| Role | SHA-256 |
| --- | --- |
| ARM kernel | `83f4ca9ea3ddeeae6125e157c0442b18c48edf8de0b71deb64a8f3f74fef290c` |
| x86 kernel | `7eb930cee1d8be888921d074a76308f6cd418046453059d07c41b5bb77ff4458` |
| Signed ARM DNS, 904,904 bytes | `8afa776ee30da977181504b95ba8211cb2221c6a8a9f35121c69e5760813c8c9` |
| Signed x86 DNS, 932,016 bytes | `3db1bc0cf73ef8e6eeec1232c73c47e16663f4b3153972e54666ae42df92bab8` |

The first DNS bytes were read/hash-checked from the v4 stores before replacement.
Subsequent public preparation validation/full-name local calls changed DNS bytes;
this failed attempt is not final-source execution evidence.

## Second linked failure: IPC worker exit readiness

The corrected caller and final signed DNS bytes ran with the same fresh-store,
four-LP configuration and TCP 19142. QEMU PID 40645 was confirmed listening.
Both runners exited **1**; x86 did not publish a completed selftest summary.
At **20.265305 s**, x86 panicked at `ipc/claim_wait/tests.rs:352`:

```text
assertion failed: crate::completion::wait_timeout(CLIENT, watched, 5_000).unwrap()
```

The affected probe retains an owned endpoint close and a real queued memory
loan. It confirms registration on the active cancellation claim and a blocked
worker, finishes that claim, then requires exact-generation thread-exit readiness.
The timeout alone does not establish whether the close remained blocked, returned
without retirement, or lost an exit notification. No original worker-state
snapshot was captured; that limits historical attribution.

At **30.528249 s** DNS logged `FATAL: failed to persist Raft log state`; telemetry
archive flush failures appeared too. These are subsequent observations, not a
proven cause of the earlier wait failure. ARM later reported its membership
deadline failure at **133.951261 s**, **20 / 1 / 1** with the same failure/pending
bitmaps as the first ARM run.

The probe now records RESULT, captured TID/generation, exact live thread state
and staged-retirement snapshot after an unsuccessful wait. All local table guards
leave before logging. Absence or a worker-return marker supplies no quiescence
proof; the existing five-second budget and exact exit requirement remain. This
adds diagnosis, not a readiness/retirement fix, retry or cleanup authorization.

Second-run kernel hashes: ARM
`651a1c76dd4fa845c26a78754f11b2f588c709cc3fea6805bb936925a84f3a8c`
(62,331,816 bytes); x86
`6a4e6850df70c43a46c88eeb74500f84074d46b1b4e61a2c629a3a8f1743f617`
(59,260,968 bytes). Final DNS identities below apply to this run too.

## HTTP caller verification

Fresh four-LP `--http-test` guests used separate localhost HTTP ports 19143/19144,
deployment-ingress ports 19145/19146 and 180-second guest budgets. Both runners
exited **0**. ARM completed **19 passed / 0 failed / 0 pending** at **14.601945 s**,
bitmap `0x23ffff`; x86 completed **17 / 0 / 0** at **18.322723 s**, bitmap `0x23bbff`.
The alpha event returned generation 1 and populated the exact catalog on ARM at
**14.600940 s** and x86 at **18.321810 s**. This exercises the changed HTTP caller,
not just its compilation.

Each architecture passed all 11 host controls: initial metrics (including complete DNS
catalog validation), HTTP EOF/abortive/idle recovery, deployment status,
malformed/oversized and complete-invalid bodies, deployment EOF/abortive/idle
recovery, and rejection/recovery after a trickled deployment body (30.07 s ARM,
30.16 s x86). The
host probes observed ConnectionResetError retries before successful controls;
those observations do not qualify transport failure causation or erase earlier
historical terminal failures.

The exact scheduled IPC claim/endpoint-close probes passed at **9.896201 s**
on ARM and **13.388459 s** on x86. Those passing fixtures do not explain or
close the second linked run's timeout. Kernel artifacts for these executions:

| Artifact | Bytes | SHA-256 |
| --- | --- | --- |
| ARM HTTP kernel | 61,638,912 | `d9b5f4a9bbef00b14ceeb3f99c892dc174b0fae44541962a797e9042d1881f48` |
| x86 HTTP kernel | 58,728,456 | `6a7552086de5bbaccba637b6646b2bdfad5600b7b4c00a42dd3fc461ba3502fb` |

Read-only v4 inspection verified each staged DNS object and matched its complete
bytes/hash to the corresponding final signed ELF below.

## Final linked verification

The diagnostic-enabled fresh-store repeat used four LPs, `--deploy-test`, a
300-second guest budget and TCP 19147. The coordinator registered stdout
readiness, started x86 after ARM boot publication and confirmed QEMU PID 52003
listening before x86 boot. Terminal runner exits were **ARM 0 / x86 0**.
Production DNS bytes are identical to the second linked and HTTP runs;
the additional IPC change only diagnoses an unsuccessful probe wait.

| Observation | ARM | x86 |
| --- | --- | --- |
| Final selftests | **22 passed / 0 failed / 0 pending**, 87.773610 s, bitmap `0xcfffff` | **20 / 0 / 0**, 39.788823 s, bitmap `0xcfbbff` |
| DNS success | 87.770900 s | 39.787770 s |
| Signed deployment success | 87.769932 s | 39.787140 s |
| Scheduled IPC claim/endpoint-close probe success | 9.004268 s | 15.213019 s |

ARM reported follower state at **74.991952 s**; x86 observed the follower's
remote invocation at **28.532012 s**. DNS success included replicated catalog,
remote invocation and automatic endpoint-death retraction fenced by generation.
The follower registered its still-live echo through the new correlated relay
as the deployment phase-1 barrier. Scoped application registration used the
memory-carried publication-generation observation without invocation authority.

Both nodes observed these exact signed replica pins:

| Generation | Node | Variant | ARM status time | x86 status time |
| --- | --- | --- | --- | --- |
| 1 | `0x42e278ea` (ARM) | 0 | 79.334598 s | 31.599486 s |
| 2 | `0x42e27737` (x86) | 1 | 83.932176 s | 35.901698 s |
| 3 | `0x42e278ea` (ARM) | 0 | 87.575924 s | 39.596304 s |

Runtime ABI 3 and signed target checks remained required. Cooperative and
zero-grace forced retirement passed, as did retired/malformed controller inputs
and the fixture key ceremony. Legacy forwarding stayed denied. Passing exact
exit probes in this run do not establish the second failure's cause.

## Final artifact identity

The read-only v4 inspector validated each guest's staged DNS object
`0xfffe2f18f4769a5c` and matched complete bytes to the signed ELF. Kernel hashes
matched each runner's captured payload identity before any subsequent profile
build. Artifact paths identify roles, not required ignored evidence files.

| Artifact | Bytes | SHA-256 |
| --- | --- | --- |
| ARM linked debug kernel | 62,344,760 | `a42a5f5f470c7e6365d98e170289baa8afd8d99cb2a263e35b6e850c6b50f2d7` |
| x86 linked debug kernel | 59,273,128 | `5e7e190d55dae946d1ce9a2f158c05d72674635117e8599231865c8594e4bc70` |
| Signed ARM DNS ELF | 904,792 | `263797bd2e66bbd51db10d1b86131af1373ccf6911276c99ba5ac1d8a537e0fe` |
| Signed x86 DNS ELF | 931,928 | `80c283996b2bb67f1b3b7052c34c7684c350a2b948ff06b1e1cb8e6ad5cd215f` |
| Local greet fixture bundle, identical on both runners | 1,896 | `a98e8a52f23fdb3f610c41484913cde50f938b03892c5f78427d5752aa7f2ce7` |
| Tracked manual PDF | 1,017,246 | `5df7d3b8dac4e0ce807e22f6b40a33c30d16432a5f3d815fb9c0d96b69747d2c` |

## Remaining issues

The scheduled IPC endpoint-close worker's exact-exit timeout remains unresolved.
Its later Raft persistence failure and the ARM membership timeout remain retained
observations, not attributed effects of registration framing or an identified
hardware fault. Original worker state was unavailable; new diagnostics preserve
that verification limit instead of inventing completion or physical retry proof.
The first catalog-only caller mismatch was corrected through the explicit event
operation; it is a separate failure with a source-backed explanation.

Remaining internal simplification includes full-name owning asynchronous local
invocation, other packed/scalar/named APIs, operational release-tail forms,
physical-address queries and mailbox word/capability contexts. Current boot/IRQ
and connector-binding contexts need coherent replacements, not keyword removal.
Registration's existing queue-cap uncertain-abort policy remains; no new remote
registration deadline or replay authority was added.

Masked ARM SVC progress, DMA unmap/close readiness, AMD-Vi runtime and wider
pressure qualification remain open. No S3, Kafka or new admission-pressure guest
suite ran here. Historical ARM stalls, NIC contender attribution, HTTP reset/EOF
failures, shared-CQ per-waiter readiness and production trust/custody remain
separate. No passing run grants new root, stack, hardware or backing quiescence.
