//! Disposable x86 QEMU fixture, not a production provisioning adapter.
//! Its anchor, exact expected state and recipient are publicly known test
//! material. The signed EFI/config/kernel chain authenticates these constants
//! only within a VM whose firmware store was enrolled by the host test owner.

use charlotte_launch::trust::signed_policy::{
    self,
    BootstrapKey,
    PolicyExpectation,
};
use zeroize::Zeroizing;

use super::PreparedBootTrust;

pub(crate) const CLUSTER: &[u8] = b"boot-trust-tests";
pub(crate) const MODULE_NAME: &str = "charlotte.boot-trust-test";
pub(super) const BOOTSTRAP_PUBLIC: [u8; 32] = [
    67, 4, 107, 254, 64, 146, 179, 233, 73, 148, 234, 218, 21, 220, 194, 13, 138, 170, 7, 182, 88,
    253, 57, 84, 235, 142, 14, 251, 139, 220, 165, 222,
];

pub(crate) fn prepare() -> PreparedBootTrust {
    let response = crate::environment::boot_protocol::limine::BOOT_POLICY_MODULES_REQUEST
        .response()
        .expect("[boot trust fixture] missing policy module response");
    let modules = response.modules();
    assert_eq!(modules.len(), 1, "[boot trust fixture] exactly one policy module required");
    let module = modules[0];
    assert_eq!(module.cmdline(), MODULE_NAME, "[boot trust fixture] unexpected module identity");
    // The Limine ABI owns this borrowed boot module. Physical admission uses
    // only MEMMAP_USABLE; inherited boot/module backing is never recycled or
    // adopted as a runtime capability. Nothing retains this borrow after copy.
    let bytes = module.data();
    assert_eq!(bytes.len(), signed_policy::ENCODED_LEN, "[boot trust fixture] invalid policy size");
    let expected_digest = signed_policy::signature_digest(include_bytes!("test-policy.bin"))
        .expect("valid compiled public fixture");
    let expected = PolicyExpectation::installed(7, expected_digest).unwrap();
    let verified = signed_policy::verify(
        bytes,
        &BootstrapKey::new(BOOTSTRAP_PUBLIC).unwrap(),
        &charlotte_launch::trust::cluster_id(CLUSTER).unwrap(),
        &expected,
    )
    .unwrap_or_else(|error| panic!("[boot trust fixture] verification rejected: {error:?}"));
    // Boot consumes already-installed state. The shared verifier also admits
    // a direct successor for an installer, but boot must not publish it before
    // the matching protected state commit (not implemented by this fixture).
    assert_eq!(
        verified.installed_expectation().unwrap(),
        expected,
        "[boot trust fixture] uninstalled policy revision"
    );
    let prepared = PreparedBootTrust::verified(verified, CLUSTER, Zeroizing::new([77; 32]))
        .unwrap_or_else(|_| panic!("[boot trust fixture] recipient mismatch"));
    crate::logln!("[boot trust fixture] verified signed module: revision=7");
    prepared
}
