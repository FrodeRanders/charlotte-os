# Charlotte's embedded-tls source copy

This workspace member is copied from
[embassy-rs/embedded-tls](https://github.com/embassy-rs/embedded-tls/tree/6487c62bb084c33361822022e21836ddfd48388f),
revision `6487c62bb084c33361822022e21836ddfd48388f` (upstream version 0.19.0),
under the included Apache-2.0 LICENSE. The 52 source files, upstream README and
license are retained. The crate's own rustfmt configuration preserves upstream
formatting; `cargo fmt --all -- --check` includes this member.

Five source files contain the protocol changes:

- `extensions/extension_data/signature_algorithms.rs`: validate a complete,
  nonempty, even-width vector; ignore complete unrecognized offers; retain
  first-preference order without duplicate storage. Known storage stays bounded.
  A separate predicate identifies schemes valid for TLS 1.3 handshake signatures.
- `extensions/extension_data/signature_algorithms_cert.rs`: use that same parser.
- `handshake/certificate_request.rs`: retain the server's recognized signature
  offers with its request context; reject missing/duplicate signature offers and
  trailing request bytes.
- `connection.rs`: both blocking and asynchronous paths check that the client's
  actual key scheme is valid for TLS 1.3 and was offered before emitting its
  certificate or signature.
- `handshake/certificate_verify.rs`: reject unknown and legacy certificate-only
  schemes when selected for a TLS 1.3 handshake signature.

Two additional source files accommodate workspace linting: `lib.rs` lists the
pre-existing upstream style/prototype lint exceptions that Cargo previously
capped for a Git dependency; `handshake/client_hello.rs` removes a redundant
single-condition `cfg(all(...))`. Neither changes the crypto implementation.

Unknown selected signatures still reject. This patch adds no crypto algorithm, verifier
bypass, transcript rewrite, retry, TLS-version fallback or kernel adapter.

The workspace manifest exposes only the configurations used or qualified here:
`rustpki`, P-256, P-384, Ed25519, optional logging and host `std`; default features
are empty. Other upstream source branches remain inactive and are listed in
`check-cfg`. Enabling another configuration requires its own backend, advisory
and target review. This avoids adding unused RSA/KEM/provider dependencies to the
complete lockfile. Existing enabled dependency versions remain unchanged.

Upstream's test/example dependency graph is not imported: its `all` crypto-driver
feature would conflict with Charlotte's erasing SHA/GCM registrations. The
upstream unit-test driver and example branches remain unmodified source but
are not test targets in this manifest. Charlotte's production-parser harness
(`scripts/tests/test-tls-signature-lists.py`) imports the actual offer, selected
signature, CertificateRequest/extension, buffer and parse modules, with an isolated old-parser rejection
control. The host service suite exercises the complete production TLS client
against OpenSSL, including client authentication and rejection. Bundled target
builds retain the existing driver/ownership guards.

The [investigation](../../docs/reports/audits/2026-11-01-tls-signature-offers.md)
records the original wire capture, tool-version distinction and qualification.
If upstream accepts an equivalent fix, compare the complete source change before
replacing this copy with another reviewed pin; rerun parser, interop, advisory
and target checks. Do not edit Cargo's shared checkout to apply this patch.
