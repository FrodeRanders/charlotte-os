//! One immutable admission policy per boot, shared by service manifests and
//! the kernel's final operational launch gate. This is a trusted kernel
//! handoff, not protected enrollment, persistent installation or key custody.

use charlotte_launch::trust::{
    self,
    AdmissionTrust,
    signed_policy::VerifiedPolicy,
};
use zeroize::Zeroizing;

use super::supervisor::NameServiceHandle;
use crate::cpu::multiprocessor::spin::mutex::Mutex;

/// Prepared policy and exclusive recipient-key owner. No authority is
/// installed until this owner is consumed by the one-shot boot publication.
pub(crate) struct PreparedBootTrust {
    public: AdmissionTrust,
    verified_digest: Option<[u8; 32]>,
    recipient: Zeroizing<[u8; 32]>,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum PreparationError {
    WrongCluster,
    RecipientMismatch,
}

impl PreparedBootTrust {
    pub(crate) fn matches_cluster(&self, cluster: &[u8]) -> bool {
        trust::cluster_id(cluster) == Some(self.public.cluster_id)
    }

    /// The caller must authenticate the boot chain, commit protected policy
    /// state and obtain the key from custody before calling this boundary.
    /// A VerifiedPolicy alone proves none of those platform conditions.
    pub(crate) fn verified(
        policy: VerifiedPolicy,
        cluster: &[u8],
        recipient: Zeroizing<[u8; 32]>,
    ) -> Result<Self, PreparationError> {
        let public = *policy.policy().public();
        if trust::cluster_id(cluster) != Some(public.cluster_id) {
            return Err(PreparationError::WrongCluster);
        }
        if charlotte_launch::operations::recipient_public_key(&recipient).ok()
            != Some(public.recipient_key)
        {
            return Err(PreparationError::RecipientMismatch);
        }
        Ok(Self {
            public,
            verified_digest: Some(policy.digest()),
            recipient,
        })
    }

    /// Explicitly insecure development fixture. Production build admission
    /// remains disabled; this constructor is not a verification fallback.
    pub(crate) fn development(cluster: &[u8]) -> Self {
        const DEVELOPMENT_RECIPIENT_PRIVATE_KEY: [u8; 32] = [
            0xf0, 0x27, 0x76, 0xea, 0x15, 0x74, 0x49, 0x30, 0x94, 0xee, 0xf5, 0xb9, 0x9d, 0xb4,
            0xd9, 0x57, 0x89, 0x0d, 0x0f, 0x48, 0x3c, 0xd9, 0x2b, 0xad, 0xe2, 0x6c, 0xe3, 0xcb,
            0x10, 0x7d, 0x3b, 0x0d,
        ];
        Self {
            public: charlotte_launch::development_admission_trust(cluster)
                .expect("valid development admission trust"),
            verified_digest: None,
            recipient: Zeroizing::new(DEVELOPMENT_RECIPIENT_PRIVATE_KEY),
        }
    }

    /// Rejection returns the entire preparation owner after unlocking. An
    /// installed policy is never replaced, even by the same revision/key.
    #[allow(clippy::result_large_err)] // Bounded inline owner; rejection must not allocate a box.
    pub(crate) fn publish(self, name_service: NameServiceHandle) -> Result<BootTrust, Self> {
        publish_into(&BOOT_TRUST, self, name_service)
    }
}

/// Public view minted only by successful boot publication. The name-service
/// handle borrows the supervisor's bootstrap grant; it does not adopt a cap.
/// Consumers cannot substitute policy or registry independently.
pub(crate) struct BootTrust {
    public: AdmissionTrust,
    verified_digest: Option<[u8; 32]>,
    name_service: NameServiceHandle,
}

impl BootTrust {
    pub(crate) fn public(&self) -> &AdmissionTrust {
        &self.public
    }

    pub(crate) fn name_service(&self) -> &NameServiceHandle {
        &self.name_service
    }

    fn snapshot(&self) -> Self {
        Self {
            public: self.public,
            verified_digest: self.verified_digest,
            name_service: self.name_service,
        }
    }
}

struct InstalledTrust {
    view: BootTrust,
    recipient: Zeroizing<[u8; 32]>,
}

static BOOT_TRUST: Mutex<Option<InstalledTrust>> = Mutex::new(None);

#[allow(clippy::result_large_err)] // Return the inline owner without allocating under the guard.
fn publish_into(
    slot: &Mutex<Option<InstalledTrust>>,
    prepared: PreparedBootTrust,
    name_service: NameServiceHandle,
) -> Result<BootTrust, PreparedBootTrust> {
    let mut installed = slot.lock();
    if installed.is_some() {
        return Err(prepared);
    }
    let view = BootTrust {
        public: prepared.public,
        verified_digest: prepared.verified_digest,
        name_service,
    };
    let result = view.snapshot();
    *installed = Some(InstalledTrust {
        view,
        recipient: prepared.recipient,
    });
    Ok(result)
}

pub(crate) fn configured_admission_trust() -> Option<AdmissionTrust> {
    BOOT_TRUST.lock().as_ref().map(|policy| policy.view.public)
}

pub(crate) fn with_operational_launch_trust<T>(
    f: impl FnOnce(&AdmissionTrust, &[u8; 32], &NameServiceHandle) -> T,
) -> Option<T> {
    with_installed_trust(&BOOT_TRUST, f)
}

fn with_installed_trust<T>(
    slot: &Mutex<Option<InstalledTrust>>,
    f: impl FnOnce(&AdmissionTrust, &[u8; 32], &NameServiceHandle) -> T,
) -> Option<T> {
    let (view, recipient) = {
        let installed = slot.lock();
        let installed = installed.as_ref()?;
        (installed.view.snapshot(), Zeroizing::new(*installed.recipient))
    };
    // Cryptography, spawning and destruction of this temporary key owner are
    // outside the masking policy guard. No private key enters an EL0 manifest.
    Some(f(&view.public, &recipient, &view.name_service))
}

pub(crate) mod tests;
