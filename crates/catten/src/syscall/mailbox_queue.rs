//! Word queues use admitted namespace nodes and retain the exact operation root.
use core::mem::ManuallyDrop;

use super::*;
use crate::{
    klib::collections::retirement_list::PreparedEntry,
    memory::{
        AddressSpaceHandle,
        operation::AddressSpaceOperation,
    },
};
type Error = mailbox_budget::Error;
pub(super) struct Namespace {
    pub(super) address_space: Option<AddressSpaceHandle>,
    pub(super) mailboxes: ShardMailboxSet<u64>,
}
struct Resources {
    asid: AddressSpaceId,
    captured: MailboxIdentity,
    root: Option<AddressSpaceOperation>,
    node: Option<PreparedEntry<(AddressSpaceId, Namespace)>>,
    queue: Option<Namespace>,
}
/// Abandonment retains even unused storage and its exact root. It is not retry
/// custody. Ordinary completion disposes unused storage after local guards.
#[must_use]
struct Operation(ManuallyDrop<Resources>);
impl Drop for Operation {
    fn drop(&mut self) {}
}
impl Operation {
    fn new(asid: AddressSpaceId, captured: MailboxIdentity) -> Result<Self, Error> {
        // None is confined to the kernel namespace and serialized raw boot
        // fixtures. A real user trap carries a live root, acquired before any
        // queue registry access; the captured generation is never refreshed.
        Ok(Self(ManuallyDrop::new(Resources {
            asid,
            captured,
            root: mailbox_publication::root(asid, captured)?,
            node: None,
            queue: None,
        })))
    }

    fn prepare(&mut self) -> Result<(), Error> {
        tests::boundary(false);
        if tests::reject() {
            return Err(Error::AllocationFailed);
        }
        self.0.node = Some(PreparedEntry::try_new().map_err(|_| Error::AllocationFailed)?);
        // concurrent-queue 2.5 has no fallible bounded constructor. Keep its
        // allocation outside lifecycle/registry guards, owned by this complete
        // operation. General queue-byte/OOM admission remains separate work.
        self.0.queue = Some(Namespace {
            address_space: self.0.captured.address_space,
            mailboxes: ShardMailboxSet::new(256),
        });
        Ok(())
    }

    fn existing(&self, target: LpId, message: u64) -> Result<Option<Result<(), u64>>, Error> {
        let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
        mailbox_publication::validate(self.0.asid, self.0.captured)?;
        let registry = USER_MAILBOX.read();
        let Some(queue) = registry.get(&self.0.asid) else {
            return Ok(None);
        };
        self.validate_queue(queue)?;
        Ok(Some(queue.mailboxes.try_send_to(target, message)))
    }

    fn validate_queue(&self, queue: &Namespace) -> Result<(), Error> {
        if queue.address_space != self.0.captured.address_space {
            return Err(Error::Retired);
        }
        Ok(())
    }

    fn publish_send(&mut self, target: LpId, message: u64) -> Result<(), u64> {
        let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
        if mailbox_publication::validate(self.0.asid, self.0.captured).is_err() {
            return Err(message);
        }
        let mut registry = USER_MAILBOX.write();
        if let Some(queue) = registry.get(&self.0.asid) {
            // A competing creator won. Keep unused storage in this operation
            // until finish, rather than running its fields beneath the guard.
            if self.validate_queue(queue).is_err() {
                return Err(message);
            }
            return queue.mailboxes.try_send_to(target, message);
        }
        registry.insert(self.0.node.take().unwrap(), self.0.asid, self.0.queue.take().unwrap());
        registry.get(&self.0.asid).unwrap().mailboxes.try_send_to(target, message)
    }

    fn receive(&self) -> Option<u64> {
        let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
        mailbox_publication::validate(self.0.asid, self.0.captured).ok()?;
        let registry = USER_MAILBOX.read();
        let queue = registry.get(&self.0.asid)?;
        self.validate_queue(queue).ok()?;
        queue.mailboxes.try_recv_for_current_lp()
    }

    fn finish(mut self) -> Result<(), Error> {
        tests::boundary(true);
        drop(self.0.queue.take());
        drop(self.0.node.take());
        if let Some(root) = self.0.root.take() {
            root.release().map_err(|_| Error::Retired)?;
        }
        Ok(())
    }
}
pub(super) fn send(
    asid: AddressSpaceId,
    target: LpId,
    message: u64,
    captured: MailboxIdentity,
) -> Result<(), u64> {
    if target >= get_lp_count() {
        return Err(message);
    }
    let mut operation = Operation::new(asid, captured).map_err(|_| message)?;
    let result = match operation.existing(target, message) {
        Ok(Some(result)) => result,
        Ok(None) => operation
            .prepare()
            .map_err(|_| message)
            .and_then(|()| operation.publish_send(target, message)),
        Err(_) => Err(message),
    };
    operation.finish().map_err(|_| message)?;
    result
}
pub(super) fn receive(asid: AddressSpaceId, captured: MailboxIdentity) -> Option<u64> {
    let operation = Operation::new(asid, captured).ok()?;
    let result = operation.receive();
    operation.finish().ok()?;
    result
}

#[path = "mailbox_queue_tests.rs"]
pub(super) mod tests;
