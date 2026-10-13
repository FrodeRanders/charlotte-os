# Fixed object-store grammar and fail-closed mount (S08a)

Date: 2026-10-13. Continues [S07](2026-10-13-protocol-simplification-tooling.md).
This completes the object-store portion of S08; other format/adapter candidates
remain in the [ledger](../../reference/api-simplification.md).

## Current contract

Store v4 uses `COBJSTR4` superblocks, current directory CRC discrimination and
fixed 384-byte `CCOBJHD4` object headers. Header/directory codecs now reside in
the no-resource protocol crate and are exercised directly by host tests. They
require exact lengths, nonzero identity/generation/owner, known flags/attributes
and zero reserved/unused fields. Header CRC checking uses fixed stack storage
instead of allocating a checksum copy. No larger-header compatibility path or
owner-zero public-read default remains. Host staging supplies kernel principal
1; runtime creation keeps its authenticated principal. Zero callers and owners
cannot use reserved/shared/scope shortcuts.

Mount distinguishes successfully read Blank, Valid, Corrupt and Unsupported
slots. Only two zero-filled superblock slots permit first-boot formatting.
Current redundancy can recover a blank/corrupt peer; retired tags reject even
beside a valid current slot. No valid current copy, incompatible device/layout
metadata, two invalid directory copies or invalid reachable headers reject
before mount writes. The old synthesized-empty directory fallback and live-ID
missing-header skip are removed. Valid newer tombstones still win over older
live entries. No migration or old decoder is retained.

The old superblock's actual bytes spelled `COJBSTR3` despite the comment
`COBJSTR3`. Both retired spellings reject; the new tag uses the documented
`COBJSTR4` spelling. This is an intentional new grammar, not a version-1 rename.
The image producer, inspector, service, guides and LaTeX/PDF agree on v4.
RPC version 1 and its opcodes, runtime ABI 3, extent geometry, copy-on-write
ordering, quotas and admitted block/IPC owners remain unchanged. C18
[cleanup evidence](../../reference/cleanup-recovery.md) records this same-family
scope without a new custody owner or physical retry proof.

See [format contract](../../reference/object-store-format.md). There are no old
production stores to migrate now. Once deployed, future changes require
version-directed, interruption-tested migration preserving committed state;
unsupported/unreadable state must never silently reset an existing store.

## Failed checks and review corrections

- First new protocol run: **3 passed / 1 failed**. Retired superblock test
  expects Unsupported but sees Corrupt, exposing the historical swapped-byte
  magic/comment mismatch. The new canonical magic and explicit rejection of
  actual historical bytes correct the classifier; no old state is accepted.
- The next full host run reaches passing protocol tests but fails its newly
  enabled rustdoc harness: the existing directory-layout Markdown fence is
  interpreted as Rust (`expected one of ! or ::, found :`, at `id: u64`). The
  fence is marked `text`; it is documentation, not a dormant executable test.
- Early service builds report two now-unused integer helpers after codec
  extraction. They are removed. A later extraction temporarily puts the
  service's private `StoredObject` snapshot in the protocol module, producing
  a dead-code warning; it is restored to its existing service boundary before
  final builds/native execution. Strict final checks contain no such warnings.
- An ARM kernel Clippy command points at the security-fixture parent rather
  than the generated fixture directory and fails its include paths. The
  corrected command uses the actual run's directory and feature profile.
  A documentation authoring command likewise uses a repository-relative path
  from the manual directory; the source edit is applied with the correct path.
  These are validation/authoring-command errors, not guest failures.

Review also identifies that two invalid directory records could synthesize an
empty record, allowing reconstruction to discard reachability. Shared decoding
and selection now reject this case; checksum-valid retired-directory tests
exercise the correction before final execution.

## Final validation

- CharlotteOS host suite: **436 passed / 0 failed / 4 ignored**, plus existing
  signing/boot/secure-boot/syscall-emitter controls, six tooling tests and **five
  object-store Python tests**. New Rust tests exercise header/extent roundtrip,
  checksum-valid invalid fields, retired/missing superblocks, mount selection,
  directory rejection and newer-tombstone selection. Python tests cross-check
  current producer/inspector, explicit staged ownership, actual and documented
  retired tags alongside current peers, redundant corruption, invalid headers
  and two retired directory copies. Inspection leaves each input unchanged.
- Both signed service bundles rebuild. Strict service Clippy on both targets
  and native kernel Clippy with each runner's exact feature profile pass.
  Rust formatting, source diff checks and Python compilation pass.
- The rebuilt manual has **212 pages**; changed pages are rendered and visually
  checked. Final artifact hashes are retained below, not in temporary evidence.

### ARM current store and security

Instance `s08a-current-arm`, four LPs, --security-test --http-test --fresh-storage
--timeout 240, HTTP/deployment ports 19111/19112. **20/0/0** at 57.115821 s,
bitmap 0x2023ffff, runner exits **0**. Native object-store clients, both security
generations, scope/ownership controls and checked CQ pressure controls pass.
The security probe reports checks=0xffff for both generations; retired
publication policy remains fenced, and concurrent cancellation retires after
14,908 requests. HTTP initial metrics, EOF, abortive/idle recovery, deployment
status, malformed/oversized and complete invalid input, EOF/abortive/idle-peer
and trickled-body controls pass. Trickle rejects in 30.19 s; bounded transport
reset retries remain observations, not clean first-attempt success.

The post-run read-only inspector reports v4, 512-byte blocks, 32,768 blocks,
superblock generation 2, next ID 43, 55 reachable objects and no validation
warnings. Current headers retain explicit owners after runtime mutation.

### x86 blank-store format and signed seed

Instance `s08a-blank-x86`, four LPs, --http-test --blank-storage
--data-size-mib 16 --fresh-storage --timeout 240, ports 19113/19114.
**17/0/0** at 19.138782 s, bitmap 0x23bbff, runner exits **0**. First boot
formats the zero-filled superblock slots and installs the signed service
bundle through kernel-owned creation. HTTP/cluster responses and deployment
framing/EOF/abortive/idle/trickle controls pass; trickle rejects in 30.12 s.
The inspector reports v4, 512-byte blocks, 32,768 blocks, generation 2, next
ID 3, 46 reachable objects and no validation warnings.

### Native retired-store rejection control

Synthetic 16 MiB ARM fixture derives from the current signed bundle, then
replaces both superblock tags/version/CRC with actual v3 `COJBSTR3` records,
directory CRC salts with v3, and every reachable header with the old
`CCOBJHD2`/version-3/owner-zero fields and recomputed CRC. Payload ELFs remain
current signed bytes. This is an explicit negative fixture, not production
state, a migration result or an accepted old decoder.

Instance `s08a-retired-arm`, four LPs, --no-network --reuse-storage --timeout 45,
explicitly reuses the same ARM kernel from the successful security/HTTP run.
Its enabled test profile still expects ordinary startup, so the runner exits
**1** with no authoritative final suite result. A partial capture initially
shows no startup reason; the existing deadline diagnostics arrive at
35.864749–35.867923 s:

```text
driver main=3 init=18 io_status=0 outstanding=0 opcode=0x2 nblocks=1
objstore stage=40 error=0xe5 block_result=0x0 block_op=0x2 reply_status=0x0 detail=0x41
objstore_threads=[]
panic at self_test/el0_nvme.rs:212: objstore registration deadline expired
```

Successful metadata I/O followed by the new selection-rejection diagnostic
qualifies fail-closed startup, with no registered store endpoint. The entire
image's SHA-256 is **identical before and after**; formatting/reconstruction
does not mutate it. The separate prefix excluding the final three pages also
matches (those pages are reserved for the existing NVMe probe). No extra
failure logging or kernel relaxation is needed. Empty thread diagnostics do
not prove physical quiescence, and this deliberately rejected run is not
reported as a passing guest suite.

## Remaining limits

This qualifies current grammar rejection and owner policy, not full device
power-loss atomicity, encrypted/authenticated storage, production enrollment or
all cleanup/IRQ contexts. Current-copy recovery remains distinct from migration.
The earlier intermittent ARM stall and DMA-map contention are unresolved;
this negative fixture's registration-deadline panic is not a diagnosis of
those failures. Remaining S08b candidates include DNS suffixes, connector
profile/operations variants, signature helpers, physical queries and raw
mailbox adapters; coherent caller/ownership replacements are still required.

## SHA-256 artifacts

| Artifact | SHA-256 |
| --- | --- |
| ARM security/HTTP kernel and retired control | `d734be6f6a6a679c4b1ceb2d42aebed5f3bdc7fff372762b0ed8e6c76757dd95` |
| x86 blank-store/HTTP kernel | `e590255fb9551ff6786f8e53904dc35808bbe4d209ac8fee4bc238c54909c903` |
| Signed ARM object store | `bc44055f31a15e7808429c39f12978cabc8c492dd0bff0df098eb8ba3d8c22af` |
| Signed x86 object store | `c5c349fcb90ff8cc86dcfca0ac5623ac5366785e28c4d08572521522f71a17dc` |
| Retired fixture, entire image before/after | `e4ae6a3d4ece6c8f7ee24f5fd44fb85ad35ca5faa5007bc6e33ea8a604c0ebee` |
| Retired fixture prefix before/after | `85d09e32772a862919d122b50e5eed860a7e98f0ad3a193718cde19f31511e4b` |
| Final 212-page manual | `c4420894f38f74b543ae4a37f62930f334247b4bd95129b72d57b60e3215ee68` |
