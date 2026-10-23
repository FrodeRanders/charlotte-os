//! Policy and telemetry nodes follow the existing registration/root owners.
use core::{
    alloc::AllocError,
    mem::ManuallyDrop,
};

use super::super::*;
use crate::klib::collections::retirement_list::{
    PreparedEntry,
    RetiredEntry,
};

pub(in crate::memory) mod admission;

type Storage<T> = PreparedEntry<T, admission::Charge>;
type Retired<T> = RetiredEntry<T, admission::Charge>;

type LimitsRecord = (AddressSpaceId, (AddressSpaceHandle, DomainLimits));
type UsageRecord = (AddressSpaceId, (AddressSpaceHandle, usage::DomainUsage));
type AuthorityRecord = (AddressSpaceId, DomainAuthoritySlot);

struct Resources {
    limits: Option<Storage<LimitsRecord>>,
    usage: Option<Storage<UsageRecord>>,
    authority: Option<Storage<AuthorityRecord>>,
    history: Option<usage::PreparingHistory>,
}

/// Abandonment retains every partial allocation without allocator work.
#[must_use]
pub(in crate::memory) struct PreparingMetadata(ManuallyDrop<Resources>);

impl PreparingMetadata {
    pub(in crate::memory) fn try_new(platform: bool) -> Result<Self, AllocError> {
        Self::prepare(platform, |_| Ok(()))
    }

    fn prepare(
        platform: bool,
        mut before: impl FnMut(u8) -> Result<(), AllocError>,
    ) -> Result<Self, AllocError> {
        let mut owner = Self(ManuallyDrop::new(Resources {
            limits: None,
            usage: None,
            authority: None,
            history: None,
        }));
        let result = (|| {
            before(1)?;
            owner.0.limits = Some(admission::prepare(platform)?);
            before(2)?;
            owner.0.usage = Some(admission::prepare(platform)?);
            before(3)?;
            owner.0.authority = Some(admission::prepare(platform)?);
            before(4)?;
            owner.0.history = Some(usage::PreparingHistory::try_new(platform)?);
            before(5)
        })();
        if let Err(error) = result {
            owner.cancel_unpublished();
            return Err(error);
        }
        Ok(owner)
    }

    /// Caller holds lifecycle/table publication serialization with the newly
    /// assigned generation. Registry insertion only relinks existing nodes.
    pub(in crate::memory) fn publish(self, handle: AddressSpaceHandle) {
        self.publish_into(
            &mut DOMAIN_LIMITS.lock(),
            &mut usage::DOMAIN_USAGE.lock(),
            &mut DOMAIN_AUTHORITIES.lock(),
            handle,
        );
    }

    fn publish_into(
        mut self,
        limits: &mut DomainLimitsTable,
        usage: &mut usage::DomainUsageTable,
        authority: &mut DomainAuthoritiesTable,
        handle: AddressSpaceHandle,
    ) {
        assert!(!limits.contains_key(&handle.id()), "domain limits survived ASID teardown");
        assert!(!usage.contains_key(&handle.id()), "domain usage survived ASID teardown");
        assert!(!authority.contains_key(&handle.id()), "domain authority survived ASID teardown");
        limits.insert(
            self.0.limits.take().unwrap(),
            handle.id(),
            (handle, DomainLimits::default()),
        );
        usage.insert(
            self.0.usage.take().unwrap(),
            handle.id(),
            (handle, usage::DomainUsage::default()),
        );
        authority.insert(
            self.0.authority.take().unwrap(),
            handle.id(),
            DomainAuthoritySlot {
                handle,
                authority: None,
                history: self.0.history.take().unwrap(),
            },
        );
    }

    /// Ordinary unused preparation, after allocator and local publication guards.
    pub(in crate::memory) fn cancel_unpublished(mut self) {
        if let Some(history) = self.0.history.take() {
            history.cancel_unpublished();
        }
        drop(self.0.authority.take());
        drop(self.0.usage.take());
        drop(self.0.limits.take());
    }
}

/// All detached nodes remain in ClosingAddressSpace/RetiredAddressSpace until
/// confirmed invalidation. RetiredEntry's fallback retains each allocation.
#[must_use]
pub(in crate::memory) struct RetiredMetadata {
    limits: Retired<LimitsRecord>,
    usage: Retired<UsageRecord>,
    authority: Retired<AuthorityRecord>,
}

impl RetiredMetadata {
    /// Lifecycle is held and this generation's root admission is sealed.
    pub(in crate::memory) fn detach(handle: AddressSpaceHandle) -> Self {
        let limits = DOMAIN_LIMITS.lock().take(&handle.id()).expect("sealed root lost limits");
        assert_eq!(limits.value().1.0, handle);
        let usage = usage::DOMAIN_USAGE.lock().take(&handle.id()).expect("sealed root lost usage");
        assert_eq!(usage.value().1.0, handle);
        let authority =
            DOMAIN_AUTHORITIES.lock().take(&handle.id()).expect("sealed root lost authority slot");
        assert_eq!(authority.value().1.handle, handle);
        Self {
            limits,
            usage,
            authority,
        }
    }

    pub(in crate::memory) fn test_assert_retained(&self, handle: AddressSpaceHandle) {
        assert_eq!(self.limits.value().1.0, handle);
        assert_eq!(self.usage.value().1.0, handle);
        assert_eq!(self.authority.value().1.handle, handle);
        assert!(!DOMAIN_LIMITS.lock().contains_key(&handle.id()));
        assert!(!usage::DOMAIN_USAGE.lock().contains_key(&handle.id()));
        assert!(!DOMAIN_AUTHORITIES.lock().contains_key(&handle.id()));
    }

    pub(in crate::memory) fn test_authority(&self) -> Option<DomainAuthority> {
        self.authority.value().1.authority
    }

    pub(in crate::memory) fn release(self) {
        super::super::retirement::metadata_tests::policy_boundary();
        self.authority.release_with(|(_, slot)| slot.history.cancel_unpublished());
        self.usage.release();
        self.limits.release();
    }
}

mod tests;

pub(in crate::memory) fn test_preparation() {
    tests::run();
}
