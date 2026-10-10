//! Final mailbox payload joins the exact detached root's existing receipt.
use core::mem::ManuallyDrop;

use super::*;
use crate::{
    capability::LifecycleGuard,
    klib::collections::retirement_list::RetiredEntry,
    memory::AddressSpaceHandle,
};

struct Storage {
    namespace: Option<RetiredEntry<(AddressSpaceId, AsMailboxCaps)>>,
    queue: Option<ShardMailboxSet<u64>>,
}
/// Payload and its original charges remain together through final root
/// invalidation. The root owns the unified authority namespace alongside this.
#[must_use]
pub(crate) struct RetiredMailboxes(ManuallyDrop<Storage>);
impl Drop for RetiredMailboxes {
    fn drop(&mut self) {
        // Retain queue backing, admitted nodes and original family charges.
        // Only the complete root receipt authorizes explicit release.
    }
}

/// Caller holds lifecycle after thread/operation/peer cleanup and sponsorship
/// retirement. Captured validation rejects stale roots before registry mutation.
pub(crate) fn detach(
    handle: AddressSpaceHandle,
    _lifecycle: &LifecycleGuard<'_>,
) -> Result<RetiredMailboxes, mailbox_budget::Error> {
    if crate::memory::current_address_space_handle(handle.id()) != Some(handle)
        || crate::memory::budget::accepting(handle)
    {
        return Err(mailbox_budget::Error::Retired);
    }
    let mut owner = RetiredMailboxes(ManuallyDrop::new(Storage {
        namespace: None,
        queue: None,
    }));
    {
        let mut caps = USER_MAILBOX_CAPS.write();
        if let Some(namespace) = caps.get(&handle.id()) {
            if namespace.address_space != Some(handle) {
                return Err(mailbox_budget::Error::Retired);
            }
            namespace.budget.retire();
        }
        owner.0.namespace = caps.take(&handle.id());
    }
    // Legacy BTreeMap node removal still deallocates under its registry. Only
    // queue payload backing is qualified here; admission/map storage is separate.
    owner.0.queue = USER_MAILBOX.write().remove(&handle.id());
    Ok(owner)
}
impl RetiredMailboxes {
    pub(crate) fn release(mut self) {
        crate::memory::retirement::metadata_tests::boundary(false);
        if let Some(mut namespace) = self.0.namespace.take() {
            let caps = &mut namespace.value_mut().1;
            while let Some(cap) = caps.endpoints.first_key_value().map(|(cap, _)| *cap) {
                // Authority remains charged in the root's detached namespace
                // until this complete payload batch has been released.
                caps.endpoints.take(&cap).unwrap().release();
            }
            namespace.release();
        }
        drop(self.0.queue.take());
    }
}

pub(crate) mod tests {
    use super::*;
    const WORD: u64 = 0x6d61_696c_726f_6f74;

    /// Raw fixture IDs are borrowed from the owned unstarted namespace. No
    /// userspace owners are reconstructed; final-root cleanup owns all grants.
    pub(crate) fn populate(handle: AddressSpaceHandle) -> impl Fn() -> usize {
        let captured = capture_mailbox_identity(handle.id());
        open_mailbox_endpoint(handle.id(), Some(get_lp_id()), captured).unwrap();
        open_mailbox_endpoint(handle.id(), None, captured).unwrap();
        crate::service::supervisor::grant_system_observer(handle).unwrap();
        assert_eq!(user_mailbox_send(handle.id(), get_lp_id(), WORD), Ok(()));
        let budget = USER_MAILBOX_CAPS.read().get(&handle.id()).unwrap().budget.clone();
        move || budget.used()
    }
    pub(crate) fn assert_hidden(handle: AddressSpaceHandle) {
        assert!(!USER_MAILBOX_CAPS.read().contains_key(&handle.id()));
        assert!(!USER_MAILBOX.read().contains_key(&handle.id()));
    }
    pub(crate) fn take_fixture_word(owner: &RetiredMailboxes) {
        assert_eq!(owner.0.namespace.as_ref().unwrap().value().1.endpoints.iter().count(), 2);
        assert_eq!(owner.0.queue.as_ref().unwrap().try_recv_for_current_lp(), Some(WORD));
    }
    pub(crate) fn assert_guards_available() {
        let deadline = crate::self_test::results::Deadline::after_millis(1000);
        while USER_MAILBOX_CAPS.try_write().is_none() || USER_MAILBOX.try_write().is_none() {
            deadline.assert_pending("root mailbox metadata registries");
            core::hint::spin_loop();
        }
    }
    pub(crate) fn under_registry_guards(action: impl FnOnce()) {
        let _caps = USER_MAILBOX_CAPS.write();
        let _queue = USER_MAILBOX.write();
        action();
    }
}
