# Secret-derived hash state

This is the current SEC-15 inventory for Charlotte's SHA-256, HMAC, SigV4 and
SCRAM derivation helpers. It is a bounded part of the wider secret-copy review.
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
| SCRAM client/server keys, stored key, proof and expected server signature | Existing exchange/local zeroizing owners | Local derivations drop at function return; the expected signature is consumed by final verification or drops with the client. The password remains owned until the client drops. |

Finalization pads the same SHA owner, avoiding the former second full hash-state
copy. The public SHA digest API still returns an array. The shared HMAC API
returns an owner so its secret-derived outputs do not cross that helper boundary
as unprotected arrays. All of these paths are allocation-free except the
existing connector buffers and SCRAM salt/authentication-message storage.

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

TLS session/key-schedule storage, TLS identity parsing, HPKE/library scratch,
host-tool secret copies and the rest of the connector memory inventory remain
separate SEC-15 work. No cryptographic algorithm, wire format, transport policy,
production custody, physical-release proof or retry authority is added here.
