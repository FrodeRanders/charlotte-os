//! Complete account metadata before the root's publication guards.
use core::{
    alloc::AllocError,
    mem::ManuallyDrop,
};

use super::*;
use crate::{
    klib::{
        collections::retirement_list::PreparedEntry,
        observer::WaitSponsor,
    },
    timers::budget::SchedulerSponsor,
};

mod tests;

pub(super) fn test_preparation() {
    tests::run();
}

#[must_use]
pub(crate) struct PreparingAccount(ManuallyDrop<Resources>);

struct Resources {
    limit: Amount,
    platform: bool,
    node: Option<PreparedEntry<(Identity, DomainAccount)>>,
    waiters: Option<WaitSponsor>,
    timers: Option<SchedulerSponsor>,
}

impl PreparingAccount {
    pub(crate) fn try_new() -> Result<Self, AllocError> {
        Self::prepare(false, |_| Ok(()))
    }

    pub(super) fn try_new_kernel() -> Result<Self, AllocError> {
        Self::prepare(true, |_| Ok(()))
    }

    fn prepare(
        platform: bool,
        mut before: impl FnMut(u8) -> Result<(), AllocError>,
    ) -> Result<Self, AllocError> {
        // Capturing a policy limit does not publish a generation or retain a
        // registry borrow through allocation. Classification is kernel policy.
        let limit = LEDGER.lock().default_limit();
        let mut owner = Self(ManuallyDrop::new(Resources {
            limit,
            platform,
            node: None,
            waiters: None,
            timers: None,
        }));
        let result = (|| {
            before(1)?;
            owner.0.node = Some(PreparedEntry::try_new()?);
            before(2)?;
            owner.0.waiters = Some(WaitSponsor::try_new(platform)?);
            before(3)?;
            owner.0.timers = Some(SchedulerSponsor::try_new(platform)?);
            before(4)?;
            Ok(())
        })();
        if let Err(error) = result {
            owner.cancel_unpublished();
            return Err(error);
        }
        Ok(owner)
    }

    /// Caller holds the root's publication serialization and captured table
    /// generation. This relinks existing storage only; no late lazy account.
    pub(crate) fn publish(self, handle: AddressSpaceHandle) {
        self.publish_into(&mut LEDGER.lock(), (handle.id(), handle.generation()));
    }

    fn publish_into(mut self, ledger: &mut Ledger, identity: Identity) {
        assert!(!ledger.domains.contains_key(&identity), "memory sponsor installed twice");
        assert_eq!(self.0.platform, identity.0 == KERNEL_ASID);
        let account = DomainAccount {
            budget: Budget::new(self.0.limit),
            principal: None,
            platform: self.0.platform,
            retired: false,
            closed: false,
            waiters: self.0.waiters.take().unwrap(),
            timers: self.0.timers.take().unwrap(),
        };
        ledger.domains.insert(self.0.node.take().unwrap(), identity, account);
    }

    /// Ordinary unused metadata disposal after local publication guards leave.
    /// ManuallyDrop retains every partial field on abandonment, without locks,
    /// allocation, refund or implicit sponsor/node destruction.
    pub(crate) fn cancel_unpublished(mut self) {
        drop(self.0.timers.take());
        drop(self.0.waiters.take());
        drop(self.0.node.take());
    }
}
