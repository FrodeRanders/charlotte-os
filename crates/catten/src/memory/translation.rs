//! Owning zeroed translation-frame preparation. This is publication/lifetime
//! protection, private/shared table admission and physical progress policy.

use super::{
    PAddr,
    PreparingUserFrame,
};

pub(crate) mod account;
pub(crate) use account::Account;
mod shared;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableScope {
    PrivateUser,
    SharedKernel,
}

fn allowed(scope: TableScope, platform: bool, free: u64, usable: u64) -> bool {
    scope == TableScope::SharedKernel
        || platform
        || charlotte_lifecycle::resources::frames_available(free, usable, 1)
}

#[must_use]
pub(crate) struct PreparingTable<'a> {
    frame: Option<PreparingUserFrame>,
    account: Option<&'a mut Account>,
    shared: Option<shared::Charge>,
    state: PreparationState,
}

enum PreparationState {
    Unpublished,
    Publishing,
    Installed,
}

impl<'a> PreparingTable<'a> {
    pub(crate) fn allocate(scope: TableScope, account: Option<&'a mut Account>) -> Option<Self> {
        Self::allocate_with(scope, account, |platform| {
            PreparingUserFrame::allocate_with_policy(|free, usable| {
                allowed(scope, platform, free, usable)
            })
        })
    }

    fn allocate_with(
        scope: TableScope,
        account: Option<&'a mut Account>,
        allocate: impl FnOnce(bool) -> Option<PreparingUserFrame>,
    ) -> Option<Self> {
        assert_eq!(scope == TableScope::PrivateUser, account.is_some());
        let mut preparation = Self {
            frame: None,
            account,
            shared: None,
            state: PreparationState::Unpublished,
        };
        if let Some(account) = preparation.account.as_deref_mut()
            && account.reserve().is_err()
        {
            // No reservation exists for Drop to retain.
            preparation.state = PreparationState::Installed;
            return None;
        }
        if scope == TableScope::SharedKernel {
            let Some(charge) = shared::Charge::reserve() else {
                preparation.state = PreparationState::Installed;
                return None;
            };
            preparation.shared = Some(charge);
        }
        let Some(frame) =
            allocate(preparation.account.as_deref().is_some_and(Account::is_platform))
        else {
            // Ordinary allocation rejection explicitly cancels unused admission.
            preparation.rollback_with(|_| unreachable!("no table backing allocated")).unwrap();
            return None;
        };
        preparation.frame = Some(frame);
        preparation.frame.as_ref().unwrap().zero();
        Some(preparation)
    }

    pub(crate) fn frame(&self) -> PAddr {
        self.frame.as_ref().unwrap().frame()
    }

    /// Final architecture boundary: all fallible preparation precedes this
    /// consuming call. Adopt into a parent link or owning root exactly once.
    /// Disarm before invocation so interrupted publication cannot recycle a
    /// potentially hardware-reachable table. No retry/recovery bypass exists.
    pub(crate) fn publish<T>(mut self, publish: impl FnOnce(PAddr) -> T) -> T {
        let frame = self.frame();
        self.state = PreparationState::Publishing;
        self.frame.take().unwrap().quarantine();
        let result = publish(frame);
        if let Some(charge) = self.shared.take() {
            charge.publish();
        }
        self.state = PreparationState::Installed;
        result
    }

    /// Cancel only definitely unpublished backing. Ordinary cancellation is
    /// explicit; abandoning this owner retains its original admission instead.
    pub(crate) fn cancel_unpublished(mut self) -> Result<(), super::physical::Error> {
        self.rollback_with(|frame| super::PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame))
    }

    fn rollback_with(
        &mut self,
        deallocate: impl FnOnce(PAddr) -> Result<(), super::physical::Error>,
    ) -> Result<(), super::physical::Error> {
        assert!(matches!(self.state, PreparationState::Unpublished));
        // Disarm retry and retain admission before invoking the deallocator.
        self.state = PreparationState::Installed;
        if let Some(frame) = self.frame.take() {
            if let Some(account) = self.account.as_deref_mut() {
                account.quarantine_unpublished();
            }
            frame.release_with(deallocate)?;
            if let Some(account) = self.account.as_deref_mut() {
                account.refund_quarantined();
            }
        } else if let Some(account) = self.account.as_deref_mut() {
            account.refund_unpublished();
        }
        if let Some(charge) = self.shared.take() {
            charge.refund();
        }
        Ok(())
    }

    fn retain_abandoned(&mut self) {
        if !matches!(self.state, PreparationState::Installed) {
            if let Some(account) = self.account.as_deref_mut() {
                account.quarantine_unpublished();
            }
            self.state = PreparationState::Installed;
        }
        if let Some(frame) = self.frame.take() {
            frame.quarantine();
        }
        // Shared Charge::drop only records atomic quarantine; its original
        // pool reservation remains consumed without taking the pool guard.
    }

    pub(crate) fn test_policy() {
        assert!(!allowed(TableScope::PrivateUser, false, 10, 80));
        assert!(allowed(TableScope::PrivateUser, false, 11, 80));
        assert!(!allowed(TableScope::PrivateUser, false, 0, 80));
        assert!(allowed(TableScope::PrivateUser, true, 10, 80));
        assert!(allowed(TableScope::SharedKernel, false, 10, 80));
        // Shared-kernel policy admits a request, but the real allocator still
        // rejects exhaustion. This does not manufacture free frames.
        assert!(allowed(TableScope::SharedKernel, false, 0, 80));
    }
}

impl Drop for PreparingTable<'_> {
    fn drop(&mut self) {
        self.retain_abandoned();
    }
}

mod admission_tests;
mod shared_tests;
pub(crate) mod tests;
