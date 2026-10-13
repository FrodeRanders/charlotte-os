# Current tooling inputs and removed exports (S07)

Date: 2026-10-13. Continues [S06](2026-10-13-protocol-simplification-cq-wait.md)
and the ordered [simplification ledger](../../reference/api-simplification.md).

## Changes

- Removes the unused `catten_services::socket::MAX_SOCKETS` constant. Current
  total capacity and per-principal socket/buffer quotas remain launch-owned.
- Removes the paired `--cluster-service-name` runner flag. Repeated
  `--cluster-service [NAME=]IPv4:PORT` assignments use one table. An explicit
  empty name now rejects rather than becoming an unnamed platform assignment.
- Removes the singleton compile-time VIP/port/name fallback and its two
  parsing helpers. The runner publishes only `CATTEN_CLUSTER_SERVICES`;
  kernel builds reject each old variable, including an empty one, without
  printing its value. First-assignment runner-local values still select the
  SLIRP forwarding target and are not an independent kernel policy.
- Removes the signer's exact-public-fixture argv exception. Artifact signing
  uses the existing bounded no-follow, ownership/permission-checked,
  zeroizing file reader for public fixtures and real keys alike. Known public
  fixture contents retain the existing permission exemption. Other signing
  and decryption commands already use key files.

No syscall, runtime ABI, stored format, wire opcode, budget, owner or physical
cleanup contract changes. Unnamed platform ingress and named deployment ingress
are distinct current policies; neither is removed by identifying it as old.
The SDK export carries the same signer. Living guides, contributor rules and
the LaTeX/PDF manual document [current inputs](../../reference/tooling-inputs.md).
Historical audit reports preserve their original argv-compatibility evidence.

## Failed checks and their correction

The first tooling test run fails one of six tests: the port-zero assertion
expects “from 1 to 65535”, but the existing runner reports “from 1 through
65535”. The runner rejects correctly; the diagnostic assertion is corrected.
The first full host run stops at this same Python assertion.

The next host run reports signer **17 passed / 1 failed**. The new raw-fixture
test expects an argv-specific diagnostic for every fixture; the recipient
fixture has 32 bytes, while Ed25519 artifact secrets have 64. Both are rejected,
but the shorter input receives the bounded file-open rejection. The assertion
now requires rejection with no input-value echo for both lengths; the random
64-byte secret still requires the argv-specific diagnostic. The initial SDK
archive reproduces that same **17/1** failure after a sandbox DNS failure and
an approved registry-access repeat. Its SHA-256 is retained below.

Initial direct kernel Clippy commands omit required signed bundle variables
and the ARM `acpi` feature, yielding missing compile-time bundle/E0433 errors.
A second ARM command still lacks `acpi`. Corrected checks use each signed bundle
and the native runner's `acpi,virtio_net_test,http_net_test` feature profile.
An initial x86 bundle invocation uses unsupported `--embed`; it rejects with
usage before building. The supported no-argument invocation succeeds.
These are validation-command errors, not evidence of guest-code failures.

### First ARM table/HTTP run

Instance `s07-table-arm`, four LPs, fresh storage, --http-test,
--cluster-service 10.0.2.42:80 --cluster-service orders=10.0.2.43:8080,
HTTP/deployment ports 19101/19102, 180-second window. The guest completes
**19/0/0** at 18.157662 s, bitmap 0x23ffff. Node/cluster HTTP keyholes and
malformed/EOF/abortive/idle/trickled-body recovery controls pass, with bounded
ConnectionResetError retries up to 2/2. The cluster table reports one platform
backend for 10.0.2.42:80 and no eligible backend/advertiser for unready `orders`
at 10.0.2.43:8080.

The runner exits **2**, after reporting all deferred tests passed, with:

```text
scripts/run-aarch64.sh: line 1314: syntax error near unexpected token `)'
```

Its source was edited while that shell was active to reject explicit empty
names. Current `bash -n` passes. This run is not counted as a successful runner;
the stable-source repeat is recorded separately. The original shell's complete
read-state/source snapshot is unavailable, so the terminal error is not a
reconstructed parser diagnosis or proof of a guest failure.

## Final validation

- CharlotteOS host suite: **430 passed / 0 failed / 4 ignored**, including all
  18 signer tests and signing/metadata selftests; six tooling boundary tests
  pass. The latter cover canonical named/platform assignments, malformed and
  duplicate entries, the retired paired flag and old variables (empty/nonempty),
  value-free diagnostics before link output, and both architecture build paths.
  They run in the normal host test script.
- Exported standalone SDK signer: **18/0/0** with the pinned toolchain. Both
  signed service bundles rebuild successfully through file-only key arguments.
  The archive is exported from the reviewed working tree at base `21e5991e`,
  before this batch's commit; its VERSION identifies that base. The archive
  has no lockfile; the exercised standalone resolution is hashed below.
- Strict signer and both native kernel Clippy checks pass; Rust formatting,
  source diff and shell syntax checks pass.
- Updated manual: **212 pages**, changed page rendered and visually checked.

### Stable-source guest validation

Final ARM instance `s07-table-final-arm`, ports 19103/19104, same assignments,
four LPs, fresh storage and 180-second window: **19/0/0** at 19.459328 s,
bitmap 0x23ffff, runner exits **0**. Node HTTP initial metrics, EOF, abortive
peer and idle-client recovery controls pass. Deployment HTTP status,
malformed/oversized framing, invalid complete body, EOF, abortive response,
idle-peer and trickled-body controls pass; trickled input rejects in 30.13 s.

x86 instance `s07-table-x86`, ports 19105/19106, four LPs, fresh storage,
--http-test --timeout 180, the same canonical table supplied directly through
`CATTEN_CLUSTER_SERVICES`: **17/0/0** at 18.220269 s, bitmap 0x23bbff,
runner exits **0**. Metrics/cluster responses and deployment HTTP controls
pass; trickled input rejects in 30.18 s.

Both cluster responses report `charlotte.cluster.v2`, `fresh_committed=true`,
with these exact independent projections:

| Assignment | Backend name | Eligible nodes | Advertiser |
| --- | --- | --- | --- |
| 10.0.2.42:80 | None: platform policy | 0000000042e21edb | 0000000042e21edb |
| 10.0.2.43:8080 | orders: no ready deployment | empty | None |

Neither projection grants `orders` a backend merely because the platform
service is ready. Bounded transport-reset retries up to 2/2 occur in the
negative probes and remain recorded limitations. These are single-node
table/HTTP validations, not mixed-cluster rollout or exactly-once invocation
proof. The same ARM kernel hash appears in both first and stable-source runs;
the original runner error is preserved separately above.

## Remaining scope

S07 does not close a numbered security finding or add readiness/quiescence
proof. The S06 intermittent ARM stall and earlier DMA-map contention remain
unresolved. Bounded HTTP reset retries remain qualified observations. No
multi-node movement/failover or production-trust claim follows from a
single-node table test. Sibling broker and Durga argv callers previously
identified still require their own file-path migration; no sibling repository
is changed here. S08 remaining formats/raw adapters is next.

## SHA-256 artifacts

| Artifact | SHA-256 |
| --- | --- |
| Initial SDK archive, failing assertion | `66d636a4d1fa5e41c5ff2dc3f6961ec7b735c6a335dbdd9d69084724db44b9a9` |
| Final SDK archive | `2dcf25007de4553a6acb9bfcc4764f7fc07565a85804b962b2d7e365f0085642` |
| Exercised SDK dependency lock | `91f0b61a65adfe65cfeaf3d4d713f9075b028ba89cb5365c1db7bfb593dc9c76` |
| First ARM table kernel | `e37b9c65ba72bdf882f63cbde7e51ef62aad040d66ec1212378c5140329897bd` |
| Stable-source ARM table kernel | `e37b9c65ba72bdf882f63cbde7e51ef62aad040d66ec1212378c5140329897bd` |
| x86 table/HTTP kernel | `64710e3588f711266d094f5b6159a052eb9ac24c6f38657098ed1219e4cf2668` |
| Final 212-page manual | `9fc86f8d1aac964d64524f514f57fcae79152068738fe5337724519127db37cc` |
