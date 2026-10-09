//! Never hardware-published domain ownership, carried by the complete grant.
//! This is an unstarted rollback proof, not a hardware retirement receipt.
mod tests;

use super::{
    detached_domain::DetachedDomain,
    dma::Error,
    dma_tables::Tables,
};

#[must_use]
pub(super) enum PrivateDomain {
    #[cfg(target_arch = "x86_64")]
    Vtd(DetachedDomain<super::vt_d::Domain>),
    #[cfg(target_arch = "x86_64")]
    AmdVi(DetachedDomain<super::amd_vi::Domain>),
    #[cfg(target_arch = "aarch64")]
    Smmu(DetachedDomain<super::smmu::Domain>),
}

impl PrivateDomain {
    fn tables(&mut self) -> &mut Tables {
        match self {
            #[cfg(target_arch = "x86_64")]
            Self::Vtd(domain) => domain.value_mut().private_tables(),
            #[cfg(target_arch = "x86_64")]
            Self::AmdVi(domain) => domain.value_mut().private_tables(),
            #[cfg(target_arch = "aarch64")]
            Self::Smmu(domain) => domain.value_mut().private_tables(),
        }
    }

    /// Consuming one-shot cancellation, invoked only after the enclosing
    /// lifecycle/device/backend/config guards leave. A failed walk returns the
    /// whole typed owner with frozen original charges; Drop retains every field.
    #[allow(clippy::result_large_err)] // Preserve inline ownership without allocation.
    pub(super) fn cancel(self) -> Result<(), Self> {
        self.cancel_with(|tables| tables.cancel_private())
    }

    #[allow(clippy::result_large_err)] // Preserve the complete rejected payload.
    fn cancel_with(
        mut self,
        release: impl FnOnce(&mut Tables) -> Result<(), Error>,
    ) -> Result<(), Self> {
        super::recovery_tests::probe_private_release();
        if release(self.tables()).is_err() {
            return Err(self);
        }
        super::recovery_tests::probe_private_release();
        self.dispose();
        Ok(())
    }

    fn dispose(self) {
        match self {
            #[cfg(target_arch = "x86_64")]
            Self::Vtd(domain) => drop(domain.into_inner()),
            #[cfg(target_arch = "x86_64")]
            Self::AmdVi(domain) => drop(domain.into_inner()),
            #[cfg(target_arch = "aarch64")]
            Self::Smmu(domain) => drop(domain.into_inner()),
        }
    }
}

// Variant payloads use ManuallyDrop through DetachedDomain. No automatic table,
// BTreeMap, vector, pin or ledger destruction runs on error/abandonment.

pub(super) fn test_admission() {
    tests::run();
}
