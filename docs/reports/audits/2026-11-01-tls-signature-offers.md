# TLS signature-offer investigation — 2026-11-01

## Result

The reproduced host `Handshake(32)` failure is a protocol parser defect in the
pinned embedded-tls client, triggered by OpenSSL 4.0.3's client-authentication
request. Its `signature_algorithms` vector contains
[SM2/SM3 (`0x0708`)](https://www.iana.org/assignments/tls-parameters/tls-parameters.xhtml#tls-signaturescheme) alongside
P-256. The client rejected that unknown offer and returned `DecodeError` before
reading the server certificate. It was not a socket timeout or an exhausted
kernel resource.

The corrected client ignores complete unknown offer values, validates the whole
vector, retains the server's actual client-signature offer and signs only with
an offered, TLS-1.3-valid scheme. Unknown or legacy certificate-only algorithms
selected in `CertificateVerify` reject. RSA remains disabled in Charlotte's
client; no SM2 provider, insecure verifier, protocol downgrade or retry is added.

The complete host suite passes, including 268 service tests. Final four-CPU
QEMU campaigns pass ARM Kafka TLS/mTLS/SCRAM **20/0/0** and Intel-IOMMU S3 TLS
**18/0/0**. Parser controls pass **9/0**, with an isolated restoration of the old
parser failing the captured offer. Original host and native failures remain
separate from passing repeats.

## Captured failure

Base revision: `43c266bd`. The prior three host-suite failures are retained in
[the memory-account evidence](evidence/2026-11-01-memory-account-admission.json).
Those captures lack endpoint versions and wire data. This investigation
reproduces the same error class with both endpoint and I/O diagnostics; it does
not reconstruct every historical exchange.

The instrumented full suite fails with:

```text
TLS open failed: DecodeError; client identity=true
left: Err(Handshake(32))
right: Ok(())
```

The [client I/O capture](evidence/2026-11-01-tls-signature-offers/failed-client.io.txt)
shows a complete 177-byte ClientHello write and exactly these server reads:

| Record | Header bytes | Content bytes |
| --- | ---: | ---: |
| ServerHello | 5 | 123 |
| Compatibility ChangeCipherSpec | 5 | 1 |
| EncryptedExtensions | 5 | 23 |
| CertificateRequest | 5 | 119 |

The client consumes 286 bytes, then returns the decode error. It has not read
the Certificate, CertificateVerify or Finished records. OpenSSL's
[decoded handshake](evidence/2026-11-01-tls-signature-offers/failed-server.messages.txt)
and [state trace](evidence/2026-11-01-tls-signature-offers/failed-server.stderr.txt)
show the server emitting its normal flight and waiting for client authentication.
The fixture's signal-9 exit status was requested by cleanup; it does not establish
a server crash.

The complete 102-byte CertificateRequest begins:

```text
0d 00 00 62 00 00 5f 00 0d 00 2c 00 2a
09 05 09 06 09 04 04 03 05 03 06 03 08 07 08 08
08 1a 08 1b 08 1c 08 09 08 0a 08 0b 08 04 08 05
08 06 04 01 05 01 06 01 07 08
```

The 42-byte signature vector has 21 entries. P-256 is `04 03`; the final `07 08`
is unknown to this client. The 25-element known-scheme storage is not full.
The old parser calls `read_list(..., SignatureScheme::parse)`, which turns an
unknown scheme into `ParseError::InvalidData`. Extension decoding converts that
to `TlsError::DecodeError`, then Charlotte reports numeric code 32.

Exact ciphertext consumed/written is retained as
[server-to-client hex](evidence/2026-11-01-tls-signature-offers/failed-server-to-client.tls.hex)
and [client-to-server hex](evidence/2026-11-01-tls-signature-offers/failed-client-to-server.tls.hex).
The peer trace supplies the corresponding decoded message. No fixture private
keys or TLS traffic secrets are included in published evidence.

## Why standalone repeats differed

The local command environments select different tools: repository invocations
resolve `/opt/homebrew/bin/openssl` (4.0.3), while the temporary-directory
invocations resolve `/opt/local/bin/openssl` (3.6.5). The latter does not advertise
SM2 in this offer. Twelve initial isolated runs passed under that environment.

A controlled PATH comparison uses the **same unpatched client binary**:

| Peer | Old client | Corrected client, all five interop controls |
| --- | --- | --- |
| OpenSSL 3.6.5 | Pass | Pass |
| OpenSSL 4.0.3 | `Handshake(32)` at CertificateRequest | Pass |

[Results and executable identities](evidence/2026-11-01-tls-signature-offers/results.json)
retain hashes, full peer version strings, commands and receipts. This controlled
comparison explains the reproduced class without a scheduling hypothesis.
Parallel and serial full-suite failures alone did not identify its cause.

## Protocol correction and security boundary

[RFC 8446 section 3.5](https://www.rfc-editor.org/rfc/rfc8446.html#section-3.5)
defines extensible enumerations. The two signature-offer parsers now ignore only
complete unknown u16 values, require a nonempty, even, exactly bounded vector,
preserve recognized preference order and deduplicate repeated offers. Known
storage remains fixed at 25 entries; unknown values do not consume it. No
capacity increase or kernel restructuring is required.

The old CertificateRequest parser also discarded the parsed signature offers.
The complete request now retains those values through client authentication.
Missing/duplicate signature extensions and trailing request data reject. Both
blocking and asynchronous paths use the same client-certificate function, which
checks the actual key scheme before emitting the identity or signing. A server
requesting only Ed25519 or RSA-PSS cannot cause the configured P-256 identity to
sign outside its offer.

Recognizing a value in an offer does not activate its crypto provider.
[TLS 1.3 CertificateVerify](https://www.rfc-editor.org/rfc/rfc8446.html#section-4.4.3)
requires an allowed handshake signature; the original PKCS#1 v1.5 code points
may appear in certificate-signature offers but not CertificateVerify.
[RFC 9963](https://www.rfc-editor.org/rfc/rfc9963.html) adds separate opt-in
legacy client-signature codes (`0420`, `0520`, `0620`); these are not implemented
or advertised by Charlotte. RSA remains disabled here. The
shared predicate and selected-signature parser explicitly reject PKCS#1 v1.5,
SHA-1 and SHA-224 handshake signatures. Certificate, hostname, dates, transcript
and Finished verification remain required.

The [source copy and provenance](../../../crates/embedded-tls/UPSTREAM.md) retain
upstream revision `6487c62bb084c33361822022e21836ddfd48388f`, all 52 source files
and its Apache-2.0 license. Five source files contain the protocol changes; two
contain documented workspace lint accommodations. The other 45 source files
match the recorded upstream hashes. The local manifest exposes qualified
configurations without importing unused RSA/KEM dependencies or upstream's
conflicting `all` test-driver registrations. Enabled dependency versions and
Charlotte's erasing crypto drivers remain unchanged.

## Validation and retained limitations

The [self-contained results](evidence/2026-11-01-tls-signature-offers/results.json)
record source/artifact identities, the old-parser control, host receipts, both
native outcomes, failed preparations and the advisory database identity.

- Nine parser controls cover the captured offer, unknown/repeated offers,
  malformed vectors, bounded distinct storage, unknown/legacy selections and
  complete CertificateRequest ownership with missing/duplicate/trailing rejection.
  They import the production parser modules; the old-parser mutation must fail
  the captured-offer check.
- OpenSSL 3.6 and 4.0 both pass authenticated server/client-identity exchanges,
  wrong host/root/date/client rejection, malformed/tampered/truncated streams,
  and offered-key enforcement. The same production handshake, verifier and
  selected crypto drivers are used.
- `CARGO_NET_OFFLINE=true scripts/run-host-tests.sh` completes. Target Clippy
  passes with denied warnings on ARM and the x86 service target. Formatting
  passes. The complete advisory gate reports zero vulnerabilities/warnings:
  database `2295466c4e9df26647fd67d8fa1c705ac2fc7f81`, 1395 advisories.
- QEMU uses the existing bundled-service build scripts and four LPs. ARM Kafka
  performs mTLS/SCRAM, produce/consume, transaction/offset/abort checks, generic
  steps and route-leader failure/recovery. Intel S3 performs verified
  PUT/HEAD/GET/DELETE and checks the returned 33-byte object.

The initial S3 and Kafka runs both pass their target TLS application checks but
fail the separate DHCP verifier. Without a management resource, httpd correctly
reports stage 0/error `0xe010` and disables ingress; the verifier nevertheless
waits 30 seconds for stage 6. Final repeats supply the normal signed/encrypted
disposable management resource, satisfying that precondition. They do not change
kernel policy, verifier deadlines or credential-free production behavior. Those
initial failed campaigns remain captured in the results; credential-free test
composition needs a separate verifier correction.

Filtered packet captures are retained for
[S3](evidence/2026-11-01-tls-signature-offers/native-s3.pcap) and
[Kafka](evidence/2026-11-01-tls-signature-offers/native-kafka.pcap). Observed complete
guest ClientHellos offer TLS 1.3 (`0304`), AES-128-GCM/SHA-256 (`1301`) and only
P-256/P-384/Ed25519 signatures (`0403`, `0503`, `0807`). Later records are
encrypted; no native key exporter or verifier bypass was added. Captures prove
neither physical resource retirement nor hardware behavior.

Live interop and guest consumers exercise the blocking client. The asynchronous
path shares the corrected selection function but has no separate live async
campaign here. General intermediate-chain validation, authenticated security
time, production trust/custody and secret-lifetime qualification remain their
existing SEC-14/15 and production gates. This fix does not close those findings.
