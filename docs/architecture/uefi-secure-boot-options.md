# UEFI Secure Boot options for bare-metal CharlotteOS

Status: deployment option recorded for later evaluation, reviewed 2026-11-11.
The shim/MOK route described here has not been qualified with CharlotteOS.
Current signed-chain testing is the disposable x86 QEMU fixture; production
trust remains disabled. See [current coverage](../guides/qemu-secure-boot.md)
and the [SEC-04 integration requirements](../reference/bootstrap-trust-policy.md#protected-integration-still-required).

## Why retain this option?

Leo Smith's [Not so secure boot](https://leosmith.wtf/blog/not-so-secure-boot.html),
published 2026-11-08, describes booting a custom Linux installation through a
Microsoft-signed shim and interactively enrolling the installation's signing
certificate with MokManager. This suggests a possible installation route for
CharlotteOS on PCs whose firmware already trusts a suitable shim.

The useful security observation is that enrollment changes the accepted
authority. Secure Boot being enabled does not establish that only the intended
CharlotteOS installation can boot. Whether another person can authorize a key
depends on the firmware, boot access and shim management policy.

Upstream [MokManager](https://github.com/rhboot/shim/blob/main/MokManager.c)
explicitly supports enrollment from disk. Its
[variable contract](https://github.com/rhboot/shim/blob/main/MokVars.txt)
describes both authenticated enrollment requests and an optional management
password. A MOK belongs to shim's trust database; enrolling it does not add that
certificate to the firmware's `db`. The article's demonstration should therefore
be assessed as a trust-management scenario, without treating it as proof of a
signature-verification failure or universal behaviour across machines.

## Choose the deployment model

| Route | Suitable use | Benefits | Costs and limits |
| --- | --- | --- | --- |
| Direct firmware enrollment and signed Limine | Dedicated nodes whose provisioning we control | Explicit platform authority; fewer boot components; aligns with the existing QEMU fixture | Requires firmware enrollment support, signing-key custody, update/revocation policy and recovery procedures. Required firmware drivers and option ROMs also need an appropriate trust policy. |
| Microsoft-signed shim, MOK enrollment and signed Limine | PCs where the owner wants to retain factory firmware trust | May permit custom boot software while retaining existing firmware authorities | Adds shim/MokManager and their enrollment, update and revocation policy. Depends on the machine trusting that exact shim and on the complete CharlotteOS chain being compatible. |

The preferred first qualification target is direct enrollment on one selected
physical UEFI machine. Retain shim/MOK as a compatibility option when the target
machine's provisioning requirements justify it. Neither route establishes CPU,
device or IOMMU support for the physical machine.

Retaining factory authorities may also permit other signed operating systems
to boot. Any requirement that only the authorized CharlotteOS chain can run
must account for that broader trust set and every available enrollment path.

## Proposed shim chain

```text
Firmware trust database and revocations
  -> accepted Microsoft-signed shim
  -> Limine signed by an owner key enrolled in MOK
  -> hash-bound Limine configuration
  -> exact hashed CharlotteOS kernel and boot-policy module
  -> kernel verification of the installed policy
```

This is a proposed composition. Shim's
[loader documentation](https://github.com/rhboot/shim/blob/main/README.md)
describes validating a second-stage EFI application; the selected shim build's
actual acceptance of Limine must be tested. CharlotteOS loads its kernel through
Limine, so the article's Linux kernel signing commands are not a complete
CharlotteOS packaging recipe.

Preserve the existing [Limine packaging contract](../guides/limine.md#development-measured-boot-and-secure-boot):
hash the kernel and policy module, enroll the final configuration hash into
Limine, then sign that modified EFI executable. Signing a stock loader alone
does not bind its configuration or kernel. MOK enrollment authorizes the
second-stage signer; it does not replace these downstream integrity checks.
Upstream [Limine 12.9.6 usage](https://github.com/Limine-Bootloader/Limine/blob/v12.9.6/USAGE.md#secure-boot)
documents the configuration-hash requirement and enforcement conditions.

## Qualification before adoption

1. **Capture the platform and artifacts.** Record firmware version, architecture,
   Secure Boot mode, relevant trusted certificates/revocations, shim and
   MokManager provenance and hashes, Limine/configuration hashes, and exact
   kernel/module identities. Use a maintained matching shim/MokManager pair.
2. **Exercise the complete chain.** Start with disposable QEMU firmware state,
   then repeat on the selected physical machine. Require positive kernel-entry
   evidence for the authorized chain. Require rejection of altered components
   and signatures outside the intended trust set at each stage, covering the
   loader, configuration, kernel and policy module. Record other firmware- or
   shim-authorized boot paths explicitly.
3. **Test enrollment authority.** Attempt enrollment through removable media and
   available local/remote consoles under the intended administrator policy.
   Confirm that unauthorized enrollment is rejected, while authorized updates
   and recovery remain possible. Check existing MOKs, management-password and
   validation-disable settings; firmware setup protection alone does not qualify
   a separate MokManager interface.
4. **Test updates and revocation.** Qualify signer rotation, revoked shim or
   second-stage images, and recovery after trust updates. Firmware `dbx` and
   shim's SBAT policy can reject an otherwise signed image. A result applies to
   the captured firmware and trust state, not every PC with Secure Boot enabled.
5. **Retain the production gates.** Protected installed-policy state,
   interruption-safe updates, rollback resistance and recipient-key custody
   remain SEC-04 requirements. A successful shim boot cannot qualify them.

The [management rotation fixture](../guides/qemu-secure-boot.md#chain-and-fixture-boundaries)
already records older still-signed kernel replay restoring older authority.
Production qualification must cover rollback of the whole executable chain as
well as policy modules. TPM measurement or attestation may support a future
design, but neither is an implemented custody or rollback solution here.

## Certificate compatibility when revisiting

Microsoft's signing process moved from the 2011 third-party CA to the 2023 CAs
on 2026-06-26. Check the actual firmware `db`, `dbx` and shim SBAT state before
selecting artifacts. A 2011-signed shim can continue booting where its CA remains
trusted and it has not been revoked; certificate expiry alone does not force
existing installations to stop. Conversely, a 2023-only trust store cannot
validate a shim signed only by the 2011 CA. Recheck
[Microsoft's transition guidance](https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/linux-distribution-guidance-handling-2023keys?view=windows-11)
when implementing this option.
