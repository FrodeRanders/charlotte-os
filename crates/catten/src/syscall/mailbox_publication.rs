//! Mailbox publication/close own exact roots and admitted metadata together.
use core::mem::ManuallyDrop;

use super::*;
pub(super) use crate::klib::collections::retirement_list::AdmittedMap as Map;
use crate::{
    capability::{
        ObjectKind,
        PreparedReservation,
        Reservation,
        RetiredRecord,
    },
    klib::collections::retirement_list::{
        PreparedEntry,
        RetiredEntry,
    },
    memory::operation::AddressSpaceOperation,
};
type Error = mailbox_budget::Error;
type Payload = RetiredEntry<(MailboxCap, AdmittedMailboxEndpoint)>;

struct Resources {
    asid: AddressSpaceId,
    captured: MailboxIdentity,
    root: Option<AddressSpaceOperation>,
    authority: Option<PreparedReservation>,
    reservation: Option<Reservation>,
    namespace_node: Option<PreparedEntry<(AddressSpaceId, AsMailboxCaps)>>,
    endpoint_node: Option<PreparedEntry<(MailboxCap, AdmittedMailboxEndpoint)>>,
    namespace: Option<AsMailboxCaps>,
    charge: Option<mailbox_budget::Charge>,
}
/// Abandonment retains all implicit fields, including unused nodes and charges.
/// Only explicit completion may refund them and release the exact root last.
#[must_use]
pub(super) struct PreparingMailbox(ManuallyDrop<Resources>);
impl Drop for PreparingMailbox {
    fn drop(&mut self) {
        // Retain every implicit field; ordinary rollback uses finish.
    }
}
impl PreparingMailbox {
    fn new(asid: AddressSpaceId, captured: MailboxIdentity) -> Result<Self, Error> {
        let root = root(asid, captured)?;
        let mut preparation = Self(ManuallyDrop::new(Resources {
            asid,
            captured,
            root,
            authority: None,
            reservation: None,
            namespace_node: None,
            endpoint_node: None,
            namespace: None,
            charge: None,
        }));
        let result = preparation.prepare(asid, captured);
        if let Err(error) = result {
            preparation.finish()?;
            return Err(error);
        }
        Ok(preparation)
    }

    fn prepare(&mut self, asid: AddressSpaceId, captured: MailboxIdentity) -> Result<(), Error> {
        tests::metadata_boundary(false);
        if tests::reject(1) {
            return Err(Error::AllocationFailed);
        }
        self.0.namespace_node =
            Some(PreparedEntry::try_new().map_err(|_| Error::AllocationFailed)?);
        if tests::reject(2) {
            return Err(Error::AllocationFailed);
        }
        self.0.endpoint_node = Some(PreparedEntry::try_new().map_err(|_| Error::AllocationFailed)?);
        if tests::reject(3) {
            return Err(Error::AllocationFailed);
        }
        self.0.namespace = Some(AsMailboxCaps::try_new(captured.address_space)?);
        self.0.authority = Some(
            PreparedReservation::try_new(asid, ObjectKind::Mailbox, captured.address_space)
                .map_err(admission_error)?,
        );
        Ok(())
    }

    fn publish(
        &mut self,
        target: Option<LpId>,
        lifecycle: &crate::capability::LifecycleGuard<'_>,
        tables: &mut Map<AddressSpaceId, AsMailboxCaps>,
    ) -> Result<MailboxCap, Error> {
        let asid = self.0.asid;
        let captured = self.0.captured;
        validate(asid, captured)?;
        if !tables.contains_key(&asid) {
            tables.insert(
                self.0.namespace_node.take().unwrap(),
                asid,
                self.0.namespace.take().unwrap(),
            );
        }
        let caps = tables.get_mut(&asid).unwrap();
        if caps.address_space != captured.address_space {
            return Err(Error::Retired);
        }
        if target.is_none()
            && let Some(cap) = receiver(caps)
        {
            return Ok(cap);
        }
        let platform = asid == crate::memory::KERNEL_ASID
            || (captured.platform.is_some() && captured.platform == caps.address_space);
        self.0.charge = Some(mailbox_budget::reserve(&caps.budget, platform)?);
        self.0.reservation = Some(
            self.0
                .authority
                .as_mut()
                .unwrap()
                .reserve_in_lifecycle(lifecycle)
                .map_err(admission_error)?,
        );
        let reservation = self.0.reservation.as_mut().unwrap();
        let cap = reservation.identity();
        crate::capability::publish_batch(&mut [crate::capability::Publication {
            destination: reservation,
            source: None,
        }])
        .map_err(admission_error)?;
        let endpoint = match target {
            Some(target_lp) => MailboxEndpoint::Sender {
                target_lp,
            },
            None => MailboxEndpoint::Receiver {
                lp: get_lp_id(),
            },
        };
        caps.endpoints.insert(
            self.0.endpoint_node.take().unwrap(),
            cap,
            AdmittedMailboxEndpoint {
                endpoint,
                _charge: self.0.charge.take().unwrap(),
            },
        );
        Ok(cap)
    }

    fn finish(mut self) -> Result<(), Error> {
        tests::metadata_boundary(true);
        if let Some(authority) = self.0.authority.take() {
            authority.finish();
        }
        drop(self.0.reservation.take());
        drop(self.0.charge.take());
        drop(self.0.endpoint_node.take());
        drop(self.0.namespace_node.take());
        drop(self.0.namespace.take());
        if let Some(root) = self.0.root.take() {
            root.release().map_err(|_| Error::Retired)?;
        }
        Ok(())
    }
}

pub(super) fn root(
    asid: AddressSpaceId,
    captured: MailboxIdentity,
) -> Result<Option<AddressSpaceOperation>, Error> {
    if asid == crate::memory::KERNEL_ASID {
        return Ok(None);
    }
    captured
        .address_space
        .map(AddressSpaceOperation::acquire)
        .transpose()
        .map_err(|_| Error::Retired)
}
pub(super) fn validate(asid: AddressSpaceId, captured: MailboxIdentity) -> Result<(), Error> {
    if captured.address_space != crate::memory::current_address_space_handle(asid)
        || captured.address_space.is_some_and(|handle| !crate::memory::budget::accepting(handle))
    {
        return Err(Error::Retired);
    }
    if let Some(handle) = captured.address_space {
        let table = crate::memory::ADDRESS_SPACE_TABLE.lock();
        if table.is_closing(handle.id()).unwrap_or(true) {
            return Err(Error::Retired);
        }
    }
    Ok(())
}
fn receiver(caps: &AsMailboxCaps) -> Option<MailboxCap> {
    caps.endpoints.iter().find_map(|(cap, entry)| {
        matches!(entry.endpoint,
        MailboxEndpoint::Receiver { lp } if lp == get_lp_id())
        .then_some(*cap)
    })
}
pub(super) fn open(
    asid: AddressSpaceId,
    target: Option<LpId>,
    captured: MailboxIdentity,
) -> Result<MailboxCap, Error> {
    // Existing receivers need neither fresh storage nor spare admission.
    if target.is_none() {
        let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
        validate(asid, captured)?;
        if let Some(caps) = USER_MAILBOX_CAPS.read().get(&asid)
            && caps.address_space == captured.address_space
            && let Some(cap) = receiver(caps)
        {
            return Ok(cap);
        }
    }
    let mut preparation = PreparingMailbox::new(asid, captured)?;
    let result = {
        let lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
        let mut tables = USER_MAILBOX_CAPS.write();
        preparation.publish(target, &lifecycle, &mut tables)
    };
    preparation.finish()?;
    result
}

struct CloseResources {
    root: Option<AddressSpaceOperation>,
    payload: Option<Payload>,
    authority: Option<RetiredRecord>,
}
#[must_use]
pub(super) struct RetiredMailbox(ManuallyDrop<CloseResources>);
impl Drop for RetiredMailbox {
    fn drop(&mut self) {
        // Detached authority is not a completion proof or a retry owner.
    }
}
impl RetiredMailbox {
    fn prepare(asid: AddressSpaceId, cap: MailboxCap) -> Result<Self, Error> {
        let captured = capture_mailbox_identity(asid);
        let mut owner = Self(ManuallyDrop::new(CloseResources {
            root: root(asid, captured)?,
            payload: None,
            authority: None,
        }));
        let result = {
            let _lifecycle = crate::memory::ADDRESS_SPACE_LIFECYCLE.lock();
            let mut tables = USER_MAILBOX_CAPS.write();
            // Retired sponsorship still allows release, but a staged close or
            // reused generation cannot detach a predecessor's endpoint.
            if captured.address_space != crate::memory::current_address_space_handle(asid) {
                Err(Error::Retired)
            } else if let Some(caps) = tables.get_mut(&asid)
                && caps.address_space == captured.address_space
                && let Some(payload) = caps.endpoints.take(&cap)
            {
                owner.0.payload = Some(payload);
                owner.0.authority = Some(
                    crate::capability::detach(asid, cap, ObjectKind::Mailbox)
                        .expect("mailbox payload capability was absent from unified table"),
                );
                Ok(())
            } else {
                Err(Error::Retired)
            }
        };
        if let Err(error) = result {
            owner.finish()?;
            return Err(error);
        }
        Ok(owner)
    }

    fn finish(mut self) -> Result<(), Error> {
        tests::metadata_boundary(true);
        if let Some(payload) = self.0.payload.take() {
            payload.release();
        }
        if let Some(authority) = self.0.authority.take() {
            authority.release();
        }
        if let Some(root) = self.0.root.take() {
            root.release().map_err(|_| Error::Retired)?;
        }
        Ok(())
    }
}
pub(super) fn close(asid: AddressSpaceId, cap: MailboxCap) -> Result<(), Error> {
    RetiredMailbox::prepare(asid, cap)?.finish()
}
fn admission_error(error: crate::capability::AllocationError) -> Error {
    match error {
        crate::capability::AllocationError::IdentityExhausted => Error::IdentityExhausted,
        crate::capability::AllocationError::Retired => Error::Retired,
        crate::capability::AllocationError::AllocationFailed => Error::AllocationFailed,
        _ => Error::ResourceLimit,
    }
}

#[path = "mailbox_publication_tests.rs"]
pub(super) mod tests;
