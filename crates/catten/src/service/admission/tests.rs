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

fn verified() -> VerifiedBootPolicy {
    signed_policy::verify_for_boot(
        POLICY,
        &BootstrapKey::new(BOOTSTRAP_PUBLIC).unwrap(),
        &trust::cluster_id(CLUSTER).unwrap(),
        &PolicyExpectation::installed(7, signed_policy::signature_digest(POLICY).unwrap()).unwrap(),
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
        let installed_digest = BOOT_TRUST.lock().installed.as_ref().unwrap().view.verified_digest;
        assert_eq!(live, Some(*expected.policy().public()));
        assert_eq!(installed_digest, Some(expected.digest()));
    }
    let ns = crate::service::supervisor::node_name_service();
    let slot = Mutex::new(BootState::empty());
    assert!(with_installed_trust(&slot, |_, _, _| panic!("unpublished key exposed")).is_none());
    assert_eq!(artifact_key_from(&slot), None);
    assert!(publish_into(&slot, prepared(), ns).is_err());

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
    assert!(slot.lock().selected.is_none());
    assert!(select_into(&slot, b"foreign-cluster", prepared()).is_err());
    assert!(slot.lock().selected.is_none());
    select_into(&slot, CLUSTER, prepared())
        .unwrap_or_else(|_| panic!("valid boot selection rejected"));
    let key = verified().policy().public().artifact_key;
    assert_eq!(artifact_key_from(&slot), Some(key));
    assert!(with_installed_trust(&slot, |_, _, _| panic!("selected key exposed")).is_none());
    // Selection is immutable, including before the operational slot exists.
    assert!(select_into(&slot, CLUSTER, PreparedBootTrust::development(CLUSTER)).is_err());
    assert!(select_into(&slot, CLUSTER, prepared()).is_err());
    assert_eq!(artifact_key_from(&slot), Some(key));
    assert!(publish_into(&slot, prepared(), ns).is_err(), "preparation was not taken");
    let (cluster, selected) = take_from(&slot).expect("selected preparation");
    assert_eq!(cluster, CLUSTER);
    assert!(take_from(&slot).is_none());
    assert_eq!(artifact_key_from(&slot), Some(key), "taking key owner lost public authority");
    assert!(publish_into(&slot, PreparedBootTrust::development(CLUSTER), ns).is_err());
    assert!(slot.lock().installed.is_none());
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

    let published = publish_into(&slot, selected, ns)
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
        assert_eq!(slot.lock().installed.as_ref().unwrap().view.public, *published.public());
        assert_eq!(
            slot.lock().installed.as_ref().unwrap().view.verified_digest,
            published.verified_digest
        );
    }

    let development_slot = Mutex::new(BootState::empty());
    select_into(&development_slot, CLUSTER, PreparedBootTrust::development(CLUSTER))
        .unwrap_or_else(|_| panic!("explicit development selection rejected"));
    let (_, preparation) = take_from(&development_slot).unwrap();
    let development = publish_into(&development_slot, preparation, ns)
        .unwrap_or_else(|_| panic!("development publication rejected"));
    assert_eq!(development.verified_digest, None);
    assert!(publish_into(&development_slot, prepared(), ns).is_err());
    assert_eq!(
        development_slot.lock().installed.as_ref().unwrap().view.public,
        *development.public()
    );
    assert_eq!(platform_artifact_key(), live.map(|policy| policy.artifact_key));
    #[cfg(feature = "boot_trust_test")]
    {
        let image = crate::service::store::service_elf(b"ns").unwrap();
        let selected = platform_artifact_key().unwrap();
        assert_ne!(selected, charlotte_launch::CLUSTER_PUBLIC_KEY);
        assert!(crate::service::loader::validate_user_elf(image));
        assert_eq!(
            charlotte_launch::signature_note::verify_elf(image, &selected),
            charlotte_launch::signature_note::VerifyOutcome::Valid
        );
        assert_ne!(
            charlotte_launch::signature_note::verify_elf(
                image,
                &charlotte_launch::CLUSTER_PUBLIC_KEY
            ),
            charlotte_launch::signature_note::VerifyOutcome::Valid
        );
        assert!(crate::service::supervisor::persistent_elf_is_trusted(image));
    }
    assert_eq!(configured_admission_trust(), live);
    crate::logln!(
        "[boot trust] signed handoff, cluster/recipient rejection, shared manifest/gate policy \
         and one-shot publication passed; early artifact selection retained"
    );
}
