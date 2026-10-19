# SEC-15: receive-buffer ownership

Date: 2026-10-19. Reviewed baseline: `78c60c2d`.

Kafka now retains plaintext chunks, partial replies and completed frames under
erasing owners. The shared TLS receive helper constructs its owner before
calling the reader; S3 carries that return type through its local body buffers.
The [tracked evidence](evidence/2026-10-19-security-receive-owners.json) records
controls, diagnostics, artifact hashes and validation. SEC-15 remains
**partial**, with **29 corrected / 6 partial / 3 open** overall.

## Finding and correction

The former TLS receive method filled a plain vector and returned it. The Kafka
transport copied those bytes into another plain, growable vector, then used
`split_off` and replacement to return a frame. Failure/reset only cleared its
logical length. The SASL caller already wrapped the completed frame, protecting
that final allocation after return; earlier chunk releases and accumulation
had separate lifetimes. Other broker response callers returned plain vectors.

`read_owned_chunk` now initializes `Zeroizing<Vec<u8>>` before lending the
destination to its reader. Success transfers that owner directly; error, EOF
or an impossible returned count drops it. TLS and plain socket adapters return
the same owned type. Existing socket memory/mapping owners retain their
contracts; these changes establish no kernel backing-erasure guarantee.

One `FrameReceiver` replaces the existing broker accumulator. It assembles the
four-byte length header inline, validates the payload bound, and reserves the
complete frame once before copying its body. It erases consumed chunk ranges.
Coalesced replies retain the remainder in that same chunk owner, avoiding a
split/remainder allocation. Completed frames keep their owner through routing
and parsing. The frame and producer chunk have separate maximum-size checks;
this is no aggregate heap-admission guarantee.

A receiver fences subsequent reads before invoking its producer and clears
that fence only on success. Transport/protocol rejection or a host unwind
therefore requires explicit reset. The connector discards its old stream and
resets on send/read failure and before opening/authenticating a new stream.
Reset/Drop erases its partial header, frame and cached chunk. Neither reset nor
readiness grants permission to replay an accepted business operation. Native
panic-abort and forced domain termination do not promise Rust destructors.

S3's shared TLS caller now retains zeroizing HTTP/body buffers, reserves output
chunks before copying, and erases consumed body ranges. Header accumulation
and chunked decoding still grow; their older allocation copies remain an open
boundary. Signed request formatting, TLS record consumption/key schedules,
other application copies and physical memory release remain separate work.

The existing [C18 cleanup row](../../reference/cleanup-recovery.md) records these
local owners. Ordinary Drop erases/frees their buffers without capability
operations or recovery custody. No new owner family, registry, protocol grammar,
persistent format, wait policy or cryptographic algorithm is introduced.

## Controls and host validation

Four baseline controls fail, using public fixture bytes and explicit adapters
for the former private receive methods. The adapters preserve their byte
operations while replacing live socket/session access with an injected reader.
They are host byte-lifetime controls, with no baseline guest execution claim.
Their complete source and baseline source hashes are in the evidence.

| Baseline control | Allocations | Growths | Observed release |
|---|---:|---:|---|
| Successful plaintext chunk | 1 | 0 | Initialized prefix un-erased |
| Shortened chunk with initialized tail | 1 | 0 | Initialized tail un-erased |
| Fragmented reply | 1 | 1 | Completed frame un-erased |
| Partial reply, error, reset and containing-owner Drop | 1 | 0 | Partial frame un-erased; input chunk erased |

The shortened-tail fixture deliberately writes all 32 bytes and returns 16.
It establishes the helper's stronger initialized-tail policy. The pinned
`embedded-tls` reader calls its fallible buffered read before copying into the
supplied destination; synthetic write-before-error/EOF probes do **not**
reproduce that behavior in the TLS library. Growth counts also do not prove
that an allocator moved an allocation or exposed specific retired bytes.

The allocator observer reads eight known initialized fixture bytes immediately
before actual host deallocation. It never reads freed storage, padding or
uninitialized spare capacity. An early partial-reset control missed the retained
baseline allocation because clear did not free it; the final control explicitly
drops the containing receiver inside the observation window. Both observations
are retained separately. The corrected probe observes erasure and one release
of each relevant allocation, with no frame growth.

Nine transport tests pass in debug and optimized builds. They cover successful
and failed destination cleanup, initialized tails, every fragment boundary,
coalesced frames, partial-owner cleanup, invalid/maximum lengths, independent
chunk bounds, and rejection/unwind fencing before another source call.
The combined optimized launch/S3/Kafka/transport suites pass **105 runtime
tests and four compile-fail doctests**. Strict Clippy and workspace formatting
pass. A host-only workspace package selects library dependencies without
building freestanding service entry points; Cargo.lock adds that package with
no third-party version change.

Setup failures are retained separately: direct service integration selected
freestanding entry points and failed with duplicate `panic_impl`; an initial
AArch64 build found one remaining plain coordinator response type (`E0308`);
a host command run under the repository's build-std configuration failed with
duplicate core lang items (`E0152`). The harness/type correction and host runs
from outside that configuration pass. No runtime result is inferred from any
failed build.

## Native validation and remaining scope

Both bundled service builds pass: **41 signed AArch64 ELFs** and **37 signed
x86_64 ELFs**. AArch64 freshness/signature checks pass. The native checks below
reuse those staged services, with fresh per-run storage and development trust.

The Kafka TLS/mTLS/SCRAM fixture passes **20/0/0**, bitmap `0x503ffff`,
including route-leader interruption, reconnect, transaction output, group and
generic-step checks. Two `TLS handshake transport failed (6)` diagnostics
occur in this passing fault run; their cause is not independently isolated.
The kernel SHA-256 is
`421c874d4a0598d4f726ea581f4981fa3b224b4d4a41953b5dc622baa1f4f286`.

The S3 TLS fixture passes **20/0/0**, bitmap `0x303ffff`, including the
33-byte PUT/HEAD/GET/DELETE round trip. Its kernel SHA-256 is
`11bef2af034e17ea1d99b4351cfcc96ae48558ea151a6d81feb150e7b16d3982`.
Both project-scoped container/volume inspections are empty after cleanup.

The Kafka image reference was explicitly `apache/kafka:4.1.1`; only the local
post-run image identity was captured, which does not independently identify
each removed running container. The running RustFS server image was captured;
its removed initialization container has only local image evidence. The JSON
retains these limits alongside references, digests and public certificate hashes.

These guest runs establish protocol regressions and the actual owned-return
callers; the host allocation probes establish the bounded byte observations.
Neither proves every compiler spill, register/cache, library copy, forced-exit
path, DMA buffer or physical page has been erased. Production enrollment,
rollback-resistant state and recipient-key custody remain unimplemented;
production trust remains disabled.

All **180 formal inputs** match the preceding evidence (34 modules, 36 safe and
109 negative configurations). No fresh TLC cases were run: this batch changes
local byte ownership without changing a modeled protocol transition. The
pre-existing manual PDF remains untouched; its preserved hash is in the
tracked evidence. No LaTeX or manual rendering change is part of this batch.
