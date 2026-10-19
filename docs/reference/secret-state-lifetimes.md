# Secret-derived state lifetimes

This is the current SEC-15 inventory for Charlotte's SHA-256, HMAC, SigV4,
SCRAM and HPKE recipient helpers. It is a bounded part of the wider secret-copy
review.
The [execution report](../reports/audits/2026-10-19-security-hash-state-erasure.md)
records a retained-storage reproduction and the correction's validation.

## Owned memory and its release

| Material | Owner and lifetime | Release or transfer |
|---|---|---|
| SHA chaining words, partial input block and length counters | `charlotte_launch::sha256::Sha256` | `Zeroize` erases the named fields; Drop calls it on unfinished hashes and after consuming finalization. Erasure is not a reset to a fresh hash; start again with `Sha256::new`. |
| Complete input block | Borrow from caller input or the existing hash buffer | Compression does not create a separate 64-byte block copy. A completed buffered block is wiped before further input. The caller still owns its original input. |
| Expanded 64-word message schedule | Local `Zeroizing<[u32; 64]>` | Drops at compression return. |
| Eight compression working words | Local `Zeroizing<[u32; 8]>` | Drops after updating the chaining state. Scalar round intermediates remain outside the erasure guarantee. |
| SHA digest | Returned `[u8; 32]` | Caller owns the output. Public artifact digests need no secret owner; secret-derived callers wrap the output in zeroizing storage. |
| Normalized HMAC key, long-key digest, inner/outer pads and inner digest | Local zeroizing owners in the shared `hmac_sha256` helper | Drop after the derivation. Both hash contexts also erase their state. |
| HMAC result | Returned `Zeroizing<[u8; 32]>` | Ownership continues through derivation or encoding; Drop erases it. No plain-array HMAC helper remains in S3 or SCRAM. |
| SigV4 `AWS4`-prefixed secret and date/region/service/signing keys | Existing zeroizing prefix buffer and shared HMAC result owners | Retained through request signing, then dropped. The returned signature/authorization strings are intentional protocol output. |
| SCRAM PBKDF2 current block and accumulated result | Zeroizing owners; PBKDF2 returns its result owner | Replacing the current block drops its old owner. The accumulated result transfers directly to the exchange. |
| SCRAM exchange-local password and initial transcript | Initial `Client` phase, with zeroizing password/transcript buffers | Consuming server-challenge processing drops the initial phase on success or rejection, before the next transport exchange. The caller's configured credentials keep their separate lifetime. |
| SCRAM client/server keys and stored key | Local zeroizing derivation owners | Drop when challenge processing returns. |
| SCRAM proof and expected server signature | One `ClientFinal` owner | Proof bytes are borrowed for encoding. Consuming final verification or ordinary cancellation drops both fields; even early length/UTF-8 rejection consumes the phase. |
| SCRAM Base64 encoded proof/nonce and decoded salt/server verifier | Helpers construct `Zeroizing<String>` / `Zeroizing<Vec<u8>>` before writing and return those owners | Reserve the complete output bound once. Ordinary Drop erases successful output; rejection inside decoding erases its existing prefix. The decoder has no growth copies. |
| Base64 quantum digits and packed words | Local zeroizing array/word owners | Drop on each iteration or early rejection. Scalar expression/compiler copies remain outside the guarantee. |
| Encoded SASL authentication request | `Zeroizing<Encoder>` during construction, then `Zeroizing<Vec<u8>>` | Validate the complete frame before allocation/copying; reserve once and transfer the same buffer. Drop erases a completed request or any partial guarded construction. |
| Parsed SASL authentication reply | `SaslAuthenticate` retains the complete `Zeroizing<Vec<u8>>` and a validated byte range | Parsing consumes the input on every outcome. `auth_bytes()` borrows from that same frame; no independent auth-byte allocation remains. Drop erases the complete accepted/rejected frame. |
| HPKE X25519 private key and shared secret | The selected `x25519-dalek` `StaticSecret` and `SharedSecret` owners inside HPKE | The direct dependency requires `zeroize`, which HPKE's disabled defaults omitted. Production trait bounds require both owners to implement `ZeroizeOnDrop`. Drop erases their named 32-byte fields. |
| Generated recipient private bytes | Returned `Zeroizing<[u8; 32]>`, created as an owner before serialization | Transfers directly to the signing tool; ordinary return/error drops the owner. Public recipient bytes remain an array. Explicit caller copies still need their own policy. |
| Failed HPKE authentication output | Caller-owned output slice | `operations::open` uses `Zeroize` on the attempted plaintext prefix after AEAD rejection. Earlier validation errors do not modify output; the caller owns successful plaintext and unused capacity. |

Finalization pads the same SHA owner, avoiding the former second full hash-state
copy. The public SHA digest API still returns an array. The shared HMAC API
returns an owner so its secret-derived outputs do not cross that helper boundary
as unprotected arrays. The hash and key-result owners are allocation-free.
Existing connector buffers, SCRAM salt/authentication-message storage and
third-party HPKE contexts keep
their own allocation contracts.

## Evidence and limits

The erasure test retains a `MaybeUninit<Sha256>` storage slot, destroys its one
initialized owner, and reads only the initialized scalar/array fields with raw
volatile reads. It never reads padding, adopts a second owner or reads freed
memory. It covers empty, partial and complete-block states. The committed
baseline fails; the corrected optimized build passes. Independent Python
`hashlib`/`hmac` fixtures check digest and HMAC output across block/padding and
short/long-key boundaries, alongside the existing S3/SCRAM vectors.

These checks establish the observed named-storage behavior on the tested host.
They do not establish erasure of every move, scalar intermediate, compiler
spill, CPU register/cache, crash dump or caller-owned input/output. Destructors
also need execution: panic-abort, forced domain termination and power loss do
not promise ordinary Rust Drop. Heap/page teardown has its separate contract.

The pinned P-256 `SecretKey` and ECDSA `SigningKey` implementations have
erasing scalar destructors. This source observation does not qualify the full
TLS parser or session. Kafka transport accumulation and TLS receive copies,
TLS key schedules, HPKE deserialization scratch and cipher contexts, host-tool
copies and the rest of the connector memory inventory remain
separate SEC-15 work. No cryptographic algorithm, wire format, transport policy,
production custody, physical-release proof or retry authority is added here.

The [SCRAM phase follow-up](../reports/audits/2026-10-19-security-scram-phases.md)
reproduces password/verifier retention in the former mutable exchange and
replaces it with these two consuming owners. Rust compile-fail controls reject
phase reuse; runtime error cases and a TLS/mTLS/SCRAM broker fault run cover
the actual caller. This is ownership/lifetime evidence, without a new memory-
forensics guarantee or erasure claim for forced termination.

The [HPKE erasure follow-up](../reports/audits/2026-10-19-security-hpke-erasure.md)
records two failing retained-storage controls on the former feature graph and
passing debug/optimized repeats after enabling erasure. Removing the feature
from an isolated build fails both production trait bounds. A separate compile
control rejects implicit reuse of a moved generated private owner. These checks
cover named storage and the API boundary; they do not promise erasure of every
library/compiler copy or establish recipient-key custody.

The [Base64 scratch follow-up](../reports/audits/2026-10-19-security-base64-scratch.md)
observes initialized fixture bytes immediately before host deallocation. The
baseline releases a decoded prefix after late rejection without erasure;
wrapping a successful result at the caller still allows two decoder growths.
The corrected probes observe erasure and no growth. Encoding already reserved
its output and its production callers wrapped the result; returning the owner
strengthens that helper boundary. Full allocator/physical erasure and the
remaining Kafka frame copies are separate work.

The [SASL frame follow-up](../reports/audits/2026-10-19-security-sasl-frames.md)
removes the parser's independent authentication copy, including copies made
before late trailer rejection. Requests preflight the complete frame bound and
start under an erasing owner. The connector scopes the first request/reply to
challenge handling. Transport buffers before the frame enters this owner still
need separate qualification; this is not end-to-end plaintext erasure.
