# SEC-15: network port HMAC state erasure

Date: 2026-10-26. Reviewed baseline: `5eee8acf`.

Ephemeral-port selection retained a separate `Hmac<sha2::Sha256>` path after
SigV4 and SCRAM adopted the shared erasing HMAC helper. The pinned HMAC/SHA-2
owner has no destructor; its keyed inner/outer hash state, finalization scratch
and returned digest had no erasing owners. `NetworkRandom::Drop` already erased
its stored key and interface seed, but that did not erase these derivation-local
copies. This is a source-confirmed SEC-15 gap, without a demonstrated cross-domain
secret read or prediction of a live network key.

Port selection now uses the existing allocation-free
`charlotte_launch::sha256::hmac_sha256` and its zeroizing result owner.
Initialization also owns its entropy scratch before filling, including partial
or error returns. The label, accepted counter sequence, byte order, output
mapping and fresh-entropy startup requirement are preserved. The service's unused
direct HMAC and SHA-2 dependencies are removed; package versions and checksums
are unchanged. TLS's transitive HMAC/SHA-2 state remains separate work.

The [tracked evidence](evidence/2026-10-26-security-network-hmac-erasure.json)
records source/dependency/artifact hashes, the expected compile rejection,
independent vectors, final executions and relevant retry diagnostics. SEC-15
remains partial. The ledger stays **29 corrected / 6 partial / 3 open**, and
production trust stays disabled.

## Ownership and behavior

The shared helper already owns key normalization, both pads, hash contexts,
compression scratch, the inner digest and its returned `Zeroizing<[u8; 32]>`.
Port selection keeps that result owner through the two-byte extraction. Its
public domain label and counter use one fixed stack message, with no secret heap
allocation, growing buffer or replacement HMAC implementation. The shared hash
implementation is unchanged in this batch.

`NetworkRandom::initialize` constructs `Zeroizing<[u8; 40]>` before invoking the
entropy provider. Ordinary success, partial-fill rejection and error return
drop the same owner. The state still owns a separate 32-byte port key and its
interface seed, with their existing erasing destructor. Constructor moves and
compiler/register copies remain outside the named-storage guarantee.

TCP/IP still obtains entropy before creating its interface and emits no
clock/MAC/fixed-seed fallback. The native control uses public fixture entropy
only in the opt-in signed security probe; normal network startup never calls
that fixture. Counter exhaustion remains fail-stop without wrap or restart.
The selector still maps HMAC's first two bytes to the unbiased 49152–65535 range.
No retry, admission/resource limit, socket identity or wire/register ABI changes.

These local erasures require no allocator, registry, capability cleanup, wait,
logging or hardware operation in Drop. Existing root/capability owners and
physical-release proofs are unchanged. The C18 owner-family evidence and
[secret-state inventory](../../reference/secret-state-lifetimes.md) record this
boundary; no new custody or physical retry owner is introduced.

## Host and compile controls

`scripts/tests/test-network-random.py` compiles the actual production source in
an isolated optimized host executable and compares **1024 consecutive ports**
with independently computed Python `hmac`/`hashlib` results. The seed consists
of public bytes 0 through 39. No actual boot entropy, key or derived secret is
recorded.

A separate compile control requires destructor presence for the pinned old
`Hmac<sha2::Sha256>` type. It fails as expected with `E0080: legacy HMAC lacks
Drop`. This confirms destructor absence for that selected type; it is not a
post-Drop memory read or execution of the complete historical baseline. Pinned
source inspection establishes that the old named hash state/finalization scratch
has no wiping policy. The production typed result and shared erasing helper
supply the replacement ownership contract.

The shared eight-port vector helper runs in both the host unit suite and signed
EL0 probe. Host tests also check counters 255, 65535, `u32::MAX` and
`u64::MAX - 1`, then reject exhausted `u64::MAX`. Existing independent-boot,
port-key separation and unavailable/partial entropy controls remain enabled.
The native receipt is functional validation, not target-memory erasure
measurement.

Both debug and optimized release suites pass **237 tests each**: 227 service
library tests and ten shared launch tests. The latter include the existing
retained-storage SHA erasure check at eight empty/partial/full-block lengths.
There is no new retained-storage probe for HMAC output or constructor scratch;
those erasing owners retain their source/type contract. Strict service Clippy
passes for the AArch64 library/binaries and x86 library/TCP-IP/security probe.
The two Python controls pass, keeping the expected compile failure separate.
The new hook is in `run-host-tests.sh`; the full cross-workspace wrapper is not
rerun in this batch.

## Native validation and manual

Both service builders completed and signed the changed bundles from frozen
source. AArch64 stages its forty service binaries alongside the unchanged
external Sitas fixture; x86 stages 37. Native execution records the exact complete
bundle maps and kernel/boot/disk hashes. Formal inputs and policy transitions
are unchanged; no model execution or new refinement result is claimed.

Final fresh-storage, four-LP security/HTTP runs execute sequentially. Each
first final-source attempt finishes with `reason: complete`, both security
publication generations (1 and 2) passing, and no failed or pending deferred
tests:

| Backend | Deferred passed / failed / pending | HTTP checks passed | Native port-vector receipts |
| --- | --- | --- | --- |
| AArch64 / SMMUv3 | 20 / 0 / 0 | 14 | 5 |
| x86_64 / Intel VT-d | 18 / 0 / 0 | 14 | 5 |
| x86_64 / AMD-Vi | 18 / 0 / 0 | 14 | 5 |

The fifteen receipts run the same public eight-port vector on the actual guest
CPU, including existing alternate probe modes. They are functional checks,
not fifteen independent erasure measurements. Ordinary TCP/IP/network, mapping,
returned-resource, root-retirement and DMA controls remain enabled.

Each host run records four `ConnectionResetError` retries: one after EOF,
one after abortive peer closes and two during the idle-client probe. Every
affected probe subsequently passes. The evidence preserves all twelve
diagnostics with their eventual results. No timeout, retry bound or resource
limit is widened. No unexpected final guest failed. These passing results do
not establish the cause of historical scheduler, IPC, root-close, metrics or NIC
failures.

The manual is rebuilt in two passes; affected/adjacent PDF pages 108–109 are
rendered and visually checked. The full 249-page build still reports 189
overfull-box warnings, with no undefined references. This scoped check does not
re-audit the rest of the manual. Frozen code/manifest/lock/runner inputs,
workspace formatting, whitespace and local Markdown targets pass.

Ordinary Drop requires execution: forced domain termination, panic-abort and
power loss provide no erasure promise. Explicit/scalar/compiler/register/cache
copies, entropy-provider/socket/kernel backing, complete TLS traffic-key/IV/HKDF
state and physical memory retain separate lifetimes. No comprehensive erasure,
production custody, performance or off-path prediction-resistance proof follows.
SEC-15 and all other finding dispositions remain unchanged.
