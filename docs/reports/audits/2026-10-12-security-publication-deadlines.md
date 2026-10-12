# Platform publication deadlines — 2026-10-12

This follow-up to `43e212de` addresses source-confirmed SEC-06/SEC-07 startup progress gaps. It extends the [network publication correction](2026-10-12-security-network-publication.md); neither report closes those findings or attributes the historical AMD NIC exit.

## Correction and ownership

TCP/IP previously retried every rejected endpoint registration indefinitely, including permanent errors. Time, cluster control, reliable messaging and discovery instead terminated on a temporary delegated queue-full rejection. All five now use `publication::register`, sharing the existing NIC/frame-router policy: retain the exact source endpoint, retry only exact `QUEUE_FULL`, pause 10 ms, and stop admission after a selected 1000 ms budget. Permanent rejection is returned rather than retried. Existing startup failure policy remains; this does not reserve essential-service capacity or prove progress under sustained pressure.

The shared helper previously waited indefinitely after acceptance. It now polls its one pending owner with a separate 5000 ms observation budget and returns `ObservationTimeout` when that expires. Returning drops/cancels that exact owner. It never resubmits an accepted request: timeout cannot establish whether a receiver already performed publication. Returned connection/memory on an invalid reply still receives typed cleanup. A rejected pending close retains the existing domain-abort fallback, rather than ending a borrowed resource lifetime unsafely. These budgets bound the helper's retry/observation policy, not the duration of arbitrary kernel cancellation, remote scheduling or earlier dependency lookups.

Reliable messaging and discovery now create and retain typed `Endpoint` owners throughout their serving scopes. Their existing raw protocol reactors only borrow `endpoint.as_raw()`; they do not close or adopt it. Launch naming authority stays borrowed through `Context`. Endpoint fallback and other raw reactor resources keep their existing qualification limits. The existing C08/C18 rows in [cleanup/recovery](../../reference/cleanup-recovery.md) record this extension; no registry, custody category, physical retry or kernel guard qualification was added.

## Forced rejection and cancellation evidence

The signed ARM application's existing remote silent endpoint has eight queue slots and never receives requests. Before setting existing result bit 512, each primary probe launch now:

1. Retains eight original scalar calls in a fixed array, filling the remote queue. Platform registration must return exact queue-full within a two-second outer fixture budget. Its selected admission budget remains one second.
2. Drops those eight owners. A new delegated registration must then be admitted and return `ObservationTimeout` within seven outer seconds. Its selected observation budget remains five seconds; the source endpoint must still report live status.
3. Refills all eight remote slots with owned calls and drops them. A stranded queued delegation or replay would consume a slot and prevent that complete refill.

Both primary launches pass. The first full-queue trace is at guest 18.028150 s and the first success trace at 24.036866 s. The repeat is at 30.533966/36.542652 s. Each reports: `platform publication: full-queue deadline, silent-call cancellation and all eight slots recovered`. These are actual EL0 helper/architecture-emitter/dispatcher executions against an unreceived remote queue. They do not inject cancellation after remote receipt, loan revocation failure, allocation/mapping failure or uncertain physical cleanup. Existing host tests cover permanent rejection and the submission retry schedule; existing pending-call owner tests cover ordinary cleanup. No backing mapping is introduced by plain delegated publication.

The probe still requires the existing sixteen-bit `0xffff` mask, with generations 1 then 2 and retirement fencing checked by the kernel verifier. The testing guide's stale fifteen-bit references were corrected to match `security_probe_status::EXPECTED_CHECKS`; this change strengthens bit 512, rather than adding a result bit or relaxing an acceptance predicate.

## Final validation

Both complete signed service bundles were built before boot. All four runners exit 0 with unchanged 180 s windows and fresh storage:

| Configuration | Passed / failed / pending | Passed bitmap | Execution result |
| --- | --- | --- | --- |
| ARM security + HTTP + DHCP | 21 / 0 / 0 | `0x2123ffff` | Both publication probes; complete HTTP/deployment/node/cluster controls; completion at guest 37.140628 s |
| x86 AMD HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | Deployment/node/cluster controls; completion at 19.759993 s |
| Same x86 kernel, Intel HTTP + DHCP | 18 / 0 / 0 | `0x123bbff` | Deployment/node/cluster controls; completion at 21.638522 s |
| ARM signed shutdown ingress | 19 / 0 / 0 | `0x103ffff` | Completion at 10.605132 s; cooperative shutdown and PSCI power-off |

Trickled deployment rejection/recovery takes 30.09/30.16/30.15 host seconds on ARM/AMD/Intel. Existing transport-reset retries are bounded and their healthy controls pass; they do not establish the exact guest phase of a reset. The separate HTTP EOF/reset/idle script runs on ARM only.

Shutdown records Time acknowledgement at guest 14.649928 s, ReliableMessaging at 14.791242 s, Discovery at 14.871166 s, TcpIp at 14.921972 s and FrameRouter at 14.924312 s. Each has one acknowledged, zero unacknowledged, zero forced, lifecycle status 2. ObjectStore acknowledges at 14.928518 s; PSCI power-off occurs at 14.943352 s. This is ordinary cooperative shutdown with zero sockets/peers at the recorded service drains, not failure injection or an outstanding-I/O/physical-NIC test.

Kernel SHA-256: ARM combined `d6ec262b6924dc57d43a40c3e22ca098ebd420f4bccfd170fdc266b5d29da0ab`; x86 AMD/Intel `88f65c8355b521327ca907667bdf44f9e47e6553a8c00ce133b28ea48cca8e9f`; separate ARM shutdown profile `966edb8e782fa39f69114bf625912f7f3017e205e31c2dc8f6fc1bf3ac388d82`. Runtime service images are also identified by the complete signed bundle digest below; an unchanged shutdown kernel does not imply unchanged runtime service ELFs.

Bundle SHA-256, using `catten_boot_bundle_sha256`'s sorted `filename:ELF-digest` lines: ARM `65c49de5dd3f2db973701af3afa89ef8bcf4758dfe6de7375d45d5fedf18a0e3`; x86 `46b7170ba11d146b34f5cf35f12c8f2fabba60815afb3ad4b7e864562aa5fd9e`.

Selected signed ELF SHA-256:

| Artifact | ARM | x86 |
| --- | --- | --- |
| tcpip | `9e0264e209815d403f5f4224e64294b76243e988d7b7e8dcf775f3d7c87bf37e` | `92c621f467344f72d5883ff1323252c5502a13e241ab2fde24005719476b13ef` |
| time | `7a5553c9293178748209253d4122e6c928dde4a4eb9d6bfe64e58b1535e5a880` | `de6665fdbda64cb31bb3b5420d4df5ace4851acb1e4ccb3057b1c69f2e4d035d` |
| clusterctl | `3b63d3242b51f7a45b6c753ca49bbb7384dfe38a0f873910071c30922ee242b4` | `f7d2c3549fa792b0297a952607a7adcc94c3fb5804d4aeb38be1311caa6f78f4` |
| relmsg | `8d91fc8a9536f878b7252a2c87c848dc175a7b3970622b558dfa75608f2efcb7` | `ee7dc13c5af48df77c37f796b6038e7516facc33c1655e9719e9ba58047d2a13` |
| disco | `cc8fe99524292d4140fd81099157cccae5105eb03e3733f6409c50ad7768a94c` | `67167d2bb073c8a95869a0a7eb83a49abb1bdbeed33181cc5400dda7e78107f2` |

The bundled ARM probe ELF is `ff36446f72d4c3d32813fd365296ffcbc34f9757c222615adffaaa92875109c6`. Fixture generation copies and re-signs it under the fresh public test root; the actually executed fixture ELF is `91f4ed133eeebceb06c97764a97bdeb1f789687aeab09629c5b0316bec5c018d`. Shared-helper NIC/router artifacts are included in the bundle digests; their ordinary publication passes on all three targets. Their real name-service queues are not forced full by this fixture.

Host suite: **372 passed, zero failed, three ignored**. Final strict service Clippy, workspace formatting, diff whitespace and syscall-emitter coverage pass. A development rebuild separately rejected a new local array shadowing the probe's pre-existing `recovered` owner (`E0382`, later use after move). Renaming the array to `recovered_queue` corrected this before final bundles/guest runs. That rejected build is not counted as execution evidence. No failed guest run occurred in this follow-up; the historical AMD failure remains separately retained in the preceding report.

## Reproduction and remaining gates

Build with `scripts/build-catten-services.sh --embed`, then `scripts/build-catten-services-x86_64.sh`. ARM uses `--security-test --http-test --dhcp-test --fresh-storage --timeout 180`; x86 uses `--http-test --dhcp-test --iommu amd` or `intel`, fresh storage and the same window. Give simultaneous guests separate instances and host forwarding ports. Reuse a built kernel only with identical profile and bundles. Run ARM `--shutdown-ingress-test --fresh-storage --timeout 180` separately.

General controller RPC completion, startup dependency lookup, other publication adapters, concurrent healthy-client fairness, sustained connection/packet and allocation pressure, aggregate principal budgets, cancellation after remote receipt/uncertainty, broader cleanup contexts and physical-device shutdown remain open. The earlier AMD NIC startup cause is still unattributed. This follow-up supplies bounded registration policy and targeted cancellation evidence, not completion of SEC-06/07/18.
