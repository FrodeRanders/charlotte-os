# Slot return storage before publication

Date: 2026-10-07. SEC-07/SEC-18 continuation after timer observer allocation
at `c72154d`. This moves free-slot metadata preparation ahead of publication
and makes address-space slot allocation rejection explicit. General heap
admission and allocation under subsystem serialization remain partial.

## Finding and correction

`IdTable::try_add_element` prepared payload and generation vectors, but not the
third vector used to return freed IDs. `take_element` removed the payload and
then pushed its ID into that growing vector. Thread exit/abort and failed
scheduler submission use this extraction path; abort can already have removed
the thread from its run queue. A return-vector allocation failure would enter
the kernel allocation-failure path after logical mutation. Address-space close
preflights separately grew the same vector, and an unlocked close could require
another growth before final retirement.

New-slot publication now prepares all three vectors fallibly, including enough
return-ID capacity for every published slot. Failure returns the original
payload before slot/generation mutation. Partial preparation can retain unused
vector capacity, but installs no payload or ID. Available-slot reuse requires
no allocation and preserves generation checks. The existing panic wrapper uses
this same preparation path for mandatory kernel initialization/trusted fixtures.

Ordinary extraction, close preflights, closing-retirement preparation and final
slot completion no longer grow metadata. They check the publication invariant;
missing capacity rejects before extraction or close fencing, or retains an
already-closing root/fence. A slot added during unlocked cleanup also brings its
own return capacity, so final cleanup never needs a capacity refresh. Linear
lease/close/retirement authority and quarantine behavior are unchanged.

Runtime address-space registration previously used the panic wrapper. It now
uses `try_add_element` and returns `TableAllocationFailed` on slot preparation
failure. The returned root retains its original physical backing and hardware
tag until table and lifecycle guards are released; only then is unpublished
namespace preparation dropped and the root explicitly destroyed. No capability
namespace, domain-limit record or address-space handle is published on that
ordinary rejection. Runtime thread publication already returns rejected owners
after scheduler serialization leaves and now also covers return-ID storage.

Contract: [address-space retirement](../../reference/address-space-retirement.md).

## Regression evidence

The standalone production slot owner passes **27 host tests**, including four
new cases:

- Inject return-vector allocation rejection at a real growth boundary after
  payload/generation preparation. Existing slots/generations stay unchanged,
  the rejected payload is returned without destruction and retry succeeds.
  Removing every admitted slot preserves return-vector pointer/capacity.
- Mix 32 delayed retirements with 32 ordinary extractions. Both paths preserve
  return storage through interleaved completion. All 64 slots reuse without
  invoking the allocator adapter and advance their generations exactly once.
- Corrupt the private fixture's return capacity. Ordinary extraction rejects
  before taking or destroying the payload and retains its original generation.
- Corrupted capacity rejects retirement and close preflights without allocating
  or publishing a closing fence. Corruption after an admitted close rejects
  preparation while retaining that close/root. No production recovery API is
  exposed; deliberate metadata restoration occurs only in this host fixture.

The existing staged-close growth fixture now checks that new slot publication
already prepared the capacity: final preparation preserves pointer/capacity.
Existing lease, close, abandonment, generation and interleaved-completion
fixtures remain enabled.

All three guests execute 64 address-space publication rejections after real
root, capability-namespace metadata and hardware-tag preparation. At the
rejection-release boundary they verify lifecycle, address-space-table, kernel
mapping and physical-allocator guard availability, then destroy the actual
returned root. Free-frame, table node/ordinary charge and capability counts
return to baseline. Occupied-slot and domain-limit counts remain unchanged;
an unrelated namespace remains live. Successful registration afterward reuses
the expected ID with precisely the next generation and normal close succeeds.

Injection substitutes only slot publication and instruments its explicit
rejection-release boundary. The host allocator adapter substitutes only the
return-vector reservation. These are controlled rejection/invariant fixtures,
not actual heap exhaustion, allocator corruption or a demonstrated application
exploit. The new guest fixtures permanently retain no backing. Existing
quarantine fixtures and their retained frames/accounts remain unchanged.

## Validation

| Check | Result |
| --- | --- |
| Standalone slot-owner host harness | **27 passed, 0 failed**. |
| Kernel Clippy | Both custom targets, `--locked`, `-D warnings`: passed. |
| Formatting/whitespace | `cargo fmt --all -- --check`, `git diff --check`: passed. |
| Four-LP Intel VT-d QEMU | **15 passed, 0 failed, 0 pending**. |
| Four-LP AMD-Vi QEMU | **15 passed, 0 failed, 0 pending**. |
| Arm SMMUv3 security QEMU | **19 passed, 0 failed, 0 pending**; scoped probes `0xffff`, publication generations 1/2, concurrent cancellation retired 4,780 requests. |

```sh
rustc --edition=2024 --test crates/catten/src/klib/collections/id_table.rs \
  -o target/host-self-tests/id-table-tests
target/host-self-tests/id-table-tests
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network \
  --instance slot-storage-intel-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 scripts/run-x86_64.sh --no-network --iommu amd \
  --instance slot-storage-amd-20261007 --fresh-storage --timeout 180
CATTEN_SKIP_EMBED_BUILD=1 CATTEN_HTTP_HOST_PORT=18503 \
CATTEN_DEPLOY_HOST_PORT=17803 scripts/run-aarch64.sh --security-test \
  --instance slot-storage-arm-20261007 --fresh-storage --timeout 180
```

The host harness uses the repository's Rust toolchain and is also included in
`scripts/run-host-tests.sh`. All guest runners exited successfully, rebuilt
kernels and enforced assembly section permissions. Existing validated embedded
services were reused for this kernel-only change. Arm required local
forwarding-port permission.

## Remaining scope

This removes allocation from **slot return**, not all thread/domain teardown.
Deferred dead-thread staging still uses a growing map/vector, and abort request
snapshots retain other infallible allocation paths. Address-space registration
still publishes capability/domain/usage metadata into other registries with
infallible allocation. General metadata destruction and allocation under
masking guards remain SEC-18 work.

The three slot vectors retain high-water backing for table lifetime without a
new independent heap/slot-metadata budget. Reserving return IDs earlier trades
earlier bounded-by-slot-count preparation for guaranteed storage at extraction;
it does not establish comprehensive SEC-07 heap or principal admission.

Physical-platform quiescence, broader device reset and abandoned-owner recovery,
authentication, production provisioning and security-time findings are unchanged.
Scoped guest tests do not certify hostile-workload containment or close either
broad finding.
