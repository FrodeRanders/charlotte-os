//! Local policy slots exercise the production handoff without replacing the
//! live boot policy. Public fixture seeds are documented in the host test;
//! only this test uses the publicly known recipient private value [77; 32].

use charlotte_launch::trust::signed_policy::{
    self,
    BootstrapKey,
    PolicyExpectation,
};

use super::*;

const CLUSTER: &[u8] = b"boot-trust-tests";
const POLICY: &[u8; signed_policy::ENCODED_LEN] = include_bytes!("test-policy.bin");
const BOOTSTRAP_PUBLIC: [u8; 32] = [
    67, 4, 107, 254, 64, 146, 179, 233, 73, 148, 234, 218, 21, 220, 194, 13, 138, 170, 7, 182, 88,
    253, 57, 84, 235, 142, 14, 251, 139, 220, 165, 222,
];

fn verified() -> VerifiedPolicy {
    signed_policy::verify(
        POLICY,
        &BootstrapKey::new(BOOTSTRAP_PUBLIC).unwrap(),
        &trust::cluster_id(CLUSTER).unwrap(),
        &PolicyExpectation::enrollment(7).unwrap(),
    )
    .unwrap()
}

fn prepared() -> PreparedBootTrust {
    PreparedBootTrust::verified(verified(), CLUSTER, Zeroizing::new([77; 32]))
        .unwrap_or_else(|_| panic!("valid test trust preparation"))
}

pub(crate) fn run() {
    let live = configured_admission_trust();
    #[cfg(feature = "boot_trust_test")]
    {
        let expected = verified();
        let installed_digest = BOOT_TRUST.lock().as_ref().unwrap().view.verified_digest;
        assert_eq!(live, Some(*expected.policy().public()));
        assert_eq!(installed_digest, Some(expected.digest()));
    }
    let ns = crate::service::supervisor::node_name_service();
    let slot = Mutex::new(None);
    assert!(with_installed_trust(&slot, |_, _, _| panic!("unpublished key exposed")).is_none());

    assert!(matches!(
        PreparedBootTrust::verified(verified(), b"foreign-cluster", Zeroizing::new([77; 32])),
        Err(PreparationError::WrongCluster)
    ));
    assert!(matches!(
        PreparedBootTrust::verified(verified(), CLUSTER, Zeroizing::new([78; 32])),
        Err(PreparationError::RecipientMismatch)
    ));
    // Dropping valid, unpublished preparation installs nothing.
    drop(prepared());
    assert!(slot.lock().is_none());
    // The actual composition entry point rejects a foreign launch cluster
    // before touching live boot policy or spawning any services.
    let rejected =
        super::super::launch::launch_steady_state_with_trust(b"foreign-cluster", prepared())
            .expect_err("foreign launch cluster accepted");
    assert!(rejected.matches_cluster(CLUSTER));
    drop(rejected);
    assert_eq!(configured_admission_trust(), live);
    assert!(live.is_some(), "deferred test requires boot publication");
    let rejected = super::super::launch::launch_steady_state_with_trust(CLUSTER, prepared())
        .expect_err("composition replaced live boot policy");
    assert_eq!(rejected.verified_digest, Some(verified().digest()));
    drop(rejected);
    assert_eq!(configured_admission_trust(), live);

    let published = publish_into(&slot, prepared(), ns)
        .unwrap_or_else(|_| panic!("initial publication rejected"));
    assert_eq!(published.public(), verified().policy().public());
    assert_eq!(published.verified_digest, Some(verified().digest()));
    assert_eq!(published.name_service().domain.address_space, ns.domain.address_space);
    assert_eq!(published.name_service().endpoint_cap, ns.endpoint_cap);
    let manifest = published.public().encode().unwrap();
    assert_eq!(trust::AdmissionTrust::decode(&manifest).unwrap(), *published.public());
    assert_eq!(
        with_installed_trust(&slot, |public, recipient, registry| {
            assert!(slot.try_lock().is_some(), "callback executed under policy guard");
            assert_eq!(public.encode().unwrap(), manifest);
            assert_eq!(public.cluster_id, trust::cluster_id(CLUSTER).unwrap());
            assert!(recipient == &[77; 32]);
            assert_eq!(registry.domain.address_space, ns.domain.address_space);
            public.sequence
        }),
        Some(7)
    );

    for replacement in [prepared(), PreparedBootTrust::development(CLUSTER)] {
        let returned = match publish_into(&slot, replacement, ns) {
            Ok(_) => panic!("installed trust replaced"),
            Err(owner) => owner,
        };
        assert!(slot.try_lock().is_some(), "rejection retained policy guard");
        assert!(
            charlotte_launch::operations::recipient_public_key(&returned.recipient).unwrap()
                == returned.public.recipient_key
        );
        drop(returned);
        assert_eq!(slot.lock().as_ref().unwrap().view.public, *published.public());
        assert_eq!(slot.lock().as_ref().unwrap().view.verified_digest, published.verified_digest);
    }

    let development_slot = Mutex::new(None);
    let development = publish_into(&development_slot, PreparedBootTrust::development(CLUSTER), ns)
        .unwrap_or_else(|_| panic!("development publication rejected"));
    assert_eq!(development.verified_digest, None);
    assert!(publish_into(&development_slot, prepared(), ns).is_err());
    assert_eq!(development_slot.lock().as_ref().unwrap().view.public, *development.public());
    assert_eq!(configured_admission_trust(), live);
    crate::logln!(
        "[boot trust] signed handoff, cluster/recipient rejection, shared manifest/gate policy \
         and one-shot publication passed"
    );
}
