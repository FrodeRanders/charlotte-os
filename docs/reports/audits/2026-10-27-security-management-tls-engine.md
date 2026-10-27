# SEC-10: native management TLS protocol component

The audit work now prioritizes **management TLS and operator authentication**.
This checkpoint implements the server protocol component, not native listener
protection. **Neither `httpd` nor `deployd` is wired to it yet; existing management
traffic remains plaintext. SEC-10 remains open and finding counts are unchanged.**
Production trust remains disabled.

## Implemented

`charlotte-management-tls` is a workspace member with a `no_std`, socket-independent
Rustls `0.23.46` server engine. The provider adapts Charlotte's existing pinned
Embassy SHA-256/HMAC/P-256 operations and erasing RustCrypto AES-128-GCM family.
Rustls owns the standard TLS handshake, transcript, record sequencing and
possession proof. Only TLS 1.3, AES-128-GCM/SHA-256, P-256 ECDH and
P-256/SHA-256 ECDSA are enabled. No legacy protocol, insecure verifier, early
data, ticket or PSK resumption path is configured.

One immutable configuration consumes a typed server-key owner, matching
certificate chain, client roots and exact leaf/role enrollments. WebPKI chain,
validity and EKU checks remain mandatory in addition to enrollment and handshake
possession. An authenticated identity has private fields; reader/admin permission
checks and audit identity are derived from its exact leaf digest. Native dispatch
must still apply that permission before handlers and preserve signed mutation
checks.

The session owns erasing record buffers and returns plaintext in an erasing
owner only after authentication. Original monotonic handshake/session budgets,
positive-prefix send receipts and terminal failure fencing prevent input or
receipt failures from restarting the operation. `NeedInput` requests real socket
readiness; bounded local progress is a separate event. The component owns no
socket, capability, CQ registration or physical-cleanup authority.

Related capacities and budgets are constructor inputs in one validated `Limits`
value. Fragmentation validation co-scales maximum plaintext and aggregate wire
overhead. The Markdown configuration guide and manual describe defaults,
structural bounds and the absence of a native boot-resource interface.

## Validation and evidence

The tracked [evidence record](2026-10-27-security-management-tls-engine-evidence.json)
contains exact source/lock/compiler identities, debug/release test executable
hashes, complete control names, advisory report and backend-override diagnostics.
No temporary capture is needed to interpret the results.

| Check | Result |
| --- | --- |
| Host debug tests | **10 passed, 0 failed, 0 ignored** |
| Host release tests | **10 passed, 0 failed, 0 ignored** |
| Host Clippy, all targets, warnings denied | Passed |
| Actual AArch64 and x86 service target JSONs | Library check and strict Clippy passed; no native execution claim |
| Normal native dependency graphs | Neither ring nor getrandom present |
| x86 backend-policy override | Expected rejection, exit `101`: missing erasing software selection and `E0080` `needs_drop::<polyval::Polyval>()` assertion |
| Workspace rustfmt check | Passed |
| Fresh complete advisory gate | **205 packages, 0 vulnerabilities, 0 warnings** |
| Manual | Two LaTeX passes; 253 pages; pages 14 and 198–203 visually reviewed; no overfull warning in the changed chapter |

Controls exercise actual TLS 1.3 mutual authentication with an independent
Rustls/ring client and OpenSSL `s_client`. They cover both roles; header/record
fragmentation between engine steps; partial confirmed send receipts; minimum
fragment/maximum application output; invalid co-scaling; missing, unknown,
foreign and wrong-EKU clients; mismatched server key; wrong server root/name;
not-yet-valid/expired credentials and loss of the UTC provider; role removal on
reconnect using the same client configuration; plaintext, tampering and replay;
original deadlines, backward monotonic time, overflow, capacity and invalid
receipts. Negative authentication controls expose no application event.
Fixtures create independent temporary public test credentials per test and
remove them through their owner. No production entropy or trust slot is replaced.

An initial ring-native experiment compiled on AArch64 but rejected Charlotte's
x86 soft-float target because ring requires `target_feature="sse"` and `"sse2"`.
That provider was replaced, without enabling SIMD or changing the context-save
contract. The final native provider compiles on both **actual** service targets;
ring remains solely the independent host test client's dependency.

The fresh advisory database is
`3279c99911354c400e1d41e47c7d60e39258b147` (1378 advisories).
Lockfile SHA-256:
`0453bd69430eec94b1b02b2fa98df773cb618aa87f033e38da41dc1c2748b883`.
No ignore, severity filter, target exception or stale-database success mode was
added. Crypto-bigint remains `0.7.10` and ctutils `0.4.6`.

The manual has 190 global overfull warnings, including existing narrow
table-of-contents page-number columns; none occurs in the changed chapter.
There are no undefined references. PDF SHA-256:
`2d45fbb7f32b9863f6eda1f2685730f4151856b3232678a111355cdce211440e`.

## Remaining substantial work

The [management TLS contract](../../reference/management-tls.md#remaining-migration-gates)
retains explicit gates for trusted complete startup input, native socket/CQ
ownership, role authorization before handler dispatch, both direct/VIP plaintext
path replacements, standard verified host clients and QEMU fault/fairness tests.
No secured-listener, native-boot, rotation-installation or operational-availability
result follows from this component checkpoint.

Supplied roots/enrollments are not protected provisioning. UTC is still only a
nonzero caller provider, not SEC-09 authentication. The embedding domain must
retain its existing entropy failure/abort policy. Full Rustls/Embassy key-schedule,
HMAC, transcript, traffic-key and parser-copy erasure, compiler/platform
constant-time behavior, forced termination and physical memory remain unqualified.
SEC-04/08/09/15 and the remaining SEC-10 gates keep their existing status.
