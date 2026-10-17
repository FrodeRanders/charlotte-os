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

type LimitsRecord = (AddressSpaceId, (AddressSpaceHandle, DomainLimits));
type UsageRecord = (AddressSpaceId, (AddressSpaceHandle, usage::DomainUsage));

struct Resources {
    limits: Option<PreparedEntry<LimitsRecord>>,
    usage: Option<PreparedEntry<UsageRecord>>,
}

/// Abandonment retains both partial allocations without allocator work.
#[must_use]
pub(in crate::memory) struct PreparingMetadata(ManuallyDrop<Resources>);

impl PreparingMetadata {
    pub(in crate::memory) fn try_new() -> Result<Self, AllocError> {
        Self::prepare(|_| Ok(()))
    }

    fn prepare(mut before: impl FnMut(u8) -> Result<(), AllocError>) -> Result<Self, AllocError> {
        let mut owner = Self(ManuallyDrop::new(Resources {
            limits: None,
            usage: None,
        }));
        let result = (|| {
            before(1)?;
            owner.0.limits = Some(PreparedEntry::try_new()?);
            before(2)?;
            owner.0.usage = Some(PreparedEntry::try_new()?);
            before(3)
        })();
        if let Err(error) = result {
            owner.cancel_unpublished();
            return Err(error);
        }
        Ok(owner)
    }

    /// Caller holds lifecycle/table publication serialization with the newly
    /// assigned generation. Neither registry insertion allocates or disposes.
    pub(in crate::memory) fn publish(self, handle: AddressSpaceHandle) {
        self.publish_into(&mut DOMAIN_LIMITS.lock(), &mut usage::DOMAIN_USAGE.lock(), handle);
    }

    fn publish_into(
        mut self,
        limits: &mut DomainLimitsTable,
        usage: &mut usage::DomainUsageTable,
        handle: AddressSpaceHandle,
    ) {
        assert!(!limits.contains_key(&handle.id()), "domain limits survived ASID teardown");
        assert!(!usage.contains_key(&handle.id()), "domain usage survived ASID teardown");
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
    }

    /// Ordinary unused preparation, after allocator and local publication guards.
    pub(in crate::memory) fn cancel_unpublished(mut self) {
        drop(self.0.usage.take());
        drop(self.0.limits.take());
    }
}

/// Both detached nodes remain in ClosingAddressSpace/RetiredAddressSpace until
/// confirmed invalidation. RetiredEntry's fallback retains each allocation.
#[must_use]
pub(in crate::memory) struct RetiredMetadata {
    limits: RetiredEntry<LimitsRecord>,
    usage: RetiredEntry<UsageRecord>,
}

impl RetiredMetadata {
    /// Lifecycle is held and this generation's root admission is sealed.
    pub(in crate::memory) fn detach(handle: AddressSpaceHandle) -> Self {
        let limits = DOMAIN_LIMITS.lock().take(&handle.id()).expect("sealed root lost limits");
        assert_eq!(limits.value().1.0, handle);
        let usage = usage::DOMAIN_USAGE.lock().take(&handle.id()).expect("sealed root lost usage");
        assert_eq!(usage.value().1.0, handle);
        Self {
            limits,
            usage,
        }
    }

    pub(in crate::memory) fn test_assert_retained(&self, handle: AddressSpaceHandle) {
        assert_eq!(self.limits.value().1.0, handle);
        assert_eq!(self.usage.value().1.0, handle);
        assert!(!DOMAIN_LIMITS.lock().contains_key(&handle.id()));
        assert!(!usage::DOMAIN_USAGE.lock().contains_key(&handle.id()));
    }

    pub(in crate::memory) fn release(self) {
        super::super::retirement::metadata_tests::policy_boundary();
        self.usage.release();
        self.limits.release();
    }
}

mod tests;

pub(in crate::memory) fn test_preparation() {
    tests::run();
}
