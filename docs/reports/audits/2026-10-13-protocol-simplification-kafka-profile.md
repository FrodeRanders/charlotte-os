# S08b2a: closed Kafka launch authentication

Date: 2026-10-13. Baseline: local `69decb2a`, clean tree. This batch continues
internal API/protocol simplification; it does not close a numbered security
finding. Current contract: [Kafka profile](../../reference/kafka-client.md#current-profile-wire-contract).

## Change and source coverage

`charlotte-protocol-kafka::Profile` now emits and accepts only `CHKAFP7\0`,
version 7, with an exact 112-byte header. One authentication choice selects none,
SCRAM-SHA-256, P-256 mTLS, or both. Four explicit lengths delimit username,
password, certificate and private-key bytes. Disabled mechanisms require empty
fields; selected mechanisms retain their nonempty size/syntax bounds and TLS
requirement. Unknown choices, reserved bytes, trailing data and retired tags
reject. The profile still includes explicit rights, broker/route ceilings,
transaction policy and separately named attenuated authority endpoints.

The section encoder/iterator, section/payload headers, optional/critical flags,
unknown-optional skip branch, section count/byte ceilings and associated error
are removed. The unused packed Kafka `NAME` export is removed; `DEFAULT_NAME`
and full profile-selected names remain. External Kafka ApiVersions negotiation,
request schemas and RecordBatch codecs are unchanged.

The producer in `catten::service::launch::launch_kafka_profile` and the shared
profile-backed supervisor use this encoder. `kafka.elf` maps the launch-owned
read-only profile and uses the same decoder before opening broker transport.
Operational pickup/admission uses the shared decoder too. In-tree searches find
no independent profile encoder or external use of removed exports. Both signed
service bundles rebuild; the actual native kernel consumes the rebuilt ARM
bundle. Existing generic operations envelopes and catalogs keep their tags:
the opaque inner connector profile is checked at pickup, so retired encrypted
profiles must be regenerated from reviewed current inputs, not relabelled,
reset or supplied compatibility defaults. Runtime ABI remains 3.

## Ownership and documentation

The codec copies four borrowed credential slices directly into its final
allocation. The retired encoder allocated a second secret-bearing section
vector before copying it into that allocation; this extra copy no longer
exists. The kernel retains its existing `Zeroizing<Vec<u8>>` encoded-buffer
owner and read-only transfer transaction. The connector retains its existing
profile mapping and zeroizing password/private-key owners. No raw capability
adoption, cleanup retry, readiness or physical-retirement policy changes.

C18 in [cleanup/recovery](../../reference/cleanup-recovery.md) records this
same publication family. The S08b1 profile paragraph is moved from C02 to C18,
correcting its living coverage-row placement without changing historical audit
evidence. AGENTS, Kafka role architecture, testing guide, reference contract,
LaTeX and the tracked manual reflect v7. The rebuilt 212-page manual's changed
pages 97--98 were rendered and visually checked with no clipping or overlap.

## Host and build evidence

- Full host suite: **441 passed / 0 failed / 4 ignored**, including existing
  Python signing, boot, syscall, tooling and object-store controls. After a
  test-only refinement to isolate missing authentication material with valid
  TLS configuration, the current Kafka package repeats **16/0/0**.
- All four authentication choices roundtrip. The tests use placeholder CA/key
  bytes and establish codec behavior, not certificate validity.
- Rejection controls cover unknown choices, valid-digest material for disabled
  mechanisms, selected modes with missing material, every truncated prefix of
  plain and combined-auth records, declared and undeclared trailing input,
  reserved header bytes and inconsistent credential lengths.
- Constructed exact v6 records with no auth, skipped optional unknown auth and
  critical unknown auth all reject with recomputed valid integrity digests.
  Relabelling the v6 header cannot make it v7. Older versions/tags 1--6 reject.
- Both service bundles build and sign. Strict service Clippy (`--bins`) and
  kernel Clippy pass on ARM and x86 with `-D warnings`. Kernel checks use
  `acpi,virtio_net_test,dhcp_test,kafka_test` and the prepared fixture inputs.
  Native ARM build includes executable/read-only assembly-section validation.
  `cargo fmt --all -- --check` and `git diff --check` pass.

No failed validation run occurred in this batch. Rejection tests are intentional
negative controls; native fault-window transport diagnostics are retained below.

## Native ARM Kafka execution

Command: `scripts/run-aarch64.sh --kafka-test --instance s08b2-kafka-arm
--smp 4 --fresh-storage --timeout 300`. Fresh object-store-v4 image, QEMU TCG,
fixture-root signed service bundle, ephemeral public test CA/client certificate,
and disposable three-broker Kafka TLS/mTLS/SCRAM fixture. Runner exits **0** and
removes its brokers/volumes. No production credentials or enrollment claim.

- Connector ASID 121 publishes three attenuated endpoints with rights 1/2/7,
  then serves at **14.578452 s**, TLS enabled, one consume partition and one
  produce route. Group coordinator is 2; transaction coordinator is 1.
- At **14.580066 s** the guest opens the fault window; the host kills output
  leader `kafka2`, retains the 45-second runner tick window and confirms broker
  restart. These synchronous Docker operations are outside a hard wall-clock
  deadline; `--timeout 300` is the existing loop budget, not such a guarantee.
- TLS handshake transport diagnostics **6** occur at **26.007352 s** and
  **28.586420 s** during this injected-failure run. They do not establish the
  exact transport cause or authorize replay of an accepted application request.
- At **30.065690 s** the smoke app reports producer attenuation, idempotent
  produce, multi-topic transaction, read-committed consume, transactional offset
  commit and abort filtering success. Low-level output offset is **1**.
- At **34.027810 s**, generic-step results are commits **4**, produced **4**,
  business retries **2**, DLQ **1**, timeouts **1**. Connector state reports group
  generation **3**, heartbeats **2**, metadata refreshes **2**, coordinator
  refreshes **1**, routing retries **5**, terminal errors **2**, output-route
  produced **5**, lag **1**.
- Exact final result at **34.028968 s**: **20 passed / 0 failed / 0 pending**,
  passed bitmap **0x503ffff**. The runner observes this after 35 loop seconds
  and completes only after broker restart.

This qualifies real combined SCRAM/mTLS launch and data-plane recovery on ARM.
Native x86 Kafka, separate coordinator-migration and producer-fencing scenarios
were not run for this batch. x86 evidence is compilation/signing/Clippy, not
native broker authentication.

## Remaining work

S08b2b retains DNS encodings/bounded writers, operational release-tail variants,
physical-address queries and mailbox adapters. Current binding policies and
serialized fixture ownership/IRQ contexts require coherent replacements before
removal. Earlier ARM stalls, DMA-map contention, x86 HTTP EOF intermittence,
shared-CQ per-waiter readiness and production trust/custody remain unresolved;
this passing Kafka run supplies no new completion or hardware-quiescence proof.

## SHA-256 artifacts

| Artifact | SHA-256 |
| --- | --- |
| Native ARM Kafka kernel | `b501c90ed4a39bd421cb7fe4f3cde4117c0694d2729d8ad1f412eb57d6e5606b` |
| Signed ARM Kafka connector | `029f711f48a66033444ff812ebda63a91d72f45b69602f43ab4429d09ca93414` |
| Signed ARM Kafka smoke | `0c01fe2c3be863d1b8561e75c873584cf37bc2372bb71d1b0768850ab44bfe1e` |
| Signed ARM generic step | `6e0c203ae457f4562d9e646893deeee21af7bd0354525fec70b754d8c72de277` |
| Signed x86 Kafka connector, build only | `e1fba502f63d3990876546e7d3e550940bc84389bce2a0cde48301a3be8553cd` |
| Public fixture CA DER | `525d65265711c0216a75b9093359cea53e21c16f353e9cb50bd0f5b24cbb1aa8` |
| Public fixture client certificate DER | `ce5c0a0a16c75144ee9897dd03e537e51554291e6f9ebf2b379c82cdb1032068` |
| Final 212-page manual | `8ad31201c2d90803b0aa4ddc0254abfb83914a1351236f1b457b649a7001a46a` |
