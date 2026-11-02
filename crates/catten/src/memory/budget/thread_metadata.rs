//! Requested bytes and bundle counts for stable thread contexts and their
//! preallocated retirement nodes. The existing principal history sponsors all
//! generations; no ASID lookup is needed at final post-guard deallocation.
use crate::{
    klib::admission::{
        self,
        ThreadMetadata,
    },
    memory::usage::principal_history::ThreadMetadataCharge,
};

#[derive(Debug)]
#[must_use]
pub(crate) struct Charge {
    _node: admission::Reservation<ThreadMetadata, 2>,
    // Node reservation releases its guard before the principal field drops. No root/generation
    // lookup can redirect a late refund to a replacement domain or principal.
    _principal: Option<ThreadMetadataCharge>,
}

impl Charge {
    pub(super) fn reserve(
        principal: Option<u64>,
        platform: bool,
        bytes: usize,
    ) -> Result<Self, super::Error> {
        let amount = [u64::try_from(bytes).map_err(|_| super::Error::Limit)?, 1];
        let principal = principal
            .map(|principal| ThreadMetadataCharge::try_new(principal, amount))
            .transpose()
            .map_err(|_| super::Error::Limit)?;
        let node =
            admission::Reservation::reserve(platform, amount).map_err(|_| super::Error::Limit)?;
        Ok(Self {
            _node: node,
            _principal: principal,
        })
    }
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn snapshot() -> ([u64; 2], [u64; 2]) {
    admission::used::<ThreadMetadata, 2>()
}

/// Serialized boot qualification: fallback must not enter this counter lock.
#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory) fn with_locked(work: impl FnOnce()) {
    admission::with_locked::<ThreadMetadata, 2, _>(work)
}
