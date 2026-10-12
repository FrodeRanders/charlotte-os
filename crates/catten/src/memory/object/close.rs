//! Exact transfer-fence readiness for explicit memory close. Ordinary loan,
//! DMA/copy and mapping pins still reject; this is not a cleanup-completion
//! subscription or authority to retry uncertain physical work.

use alloc::sync::Weak;
use core::mem::ManuallyDrop;

use super::*;
use crate::{
    cpu::scheduler::deferred,
    klib::{
        collections::retirement_list::PreparedEntry,
        observer::{
            Observable,
            Observer,
            WaitRegistration,
            WaitSponsor,
            registration::{
                ObserverList,
                RegistrationError,
            },
            waiter_budget,
        },
    },
};

type List = deferred::WaiterList;

#[derive(Debug)]
struct SourcePayload {
    list: List,
    disposal: PreparedEntry<List>,
}

/// Complete source allocation plus its admitted disposal storage. Unexpected
/// field destruction retains both; it cannot free under a registry guard.
#[derive(Debug)]
pub(super) struct Source(ManuallyDrop<SourcePayload>);

impl Source {
    fn prepare(platform: bool) -> Result<Self, MemoryObjectError> {
        let disposal = PreparedEntry::try_new().map_err(|_| MemoryObjectError::ResourceLimit)?;
        let list = ObserverList::try_new(waiter_budget::SOURCE_LIMIT, platform)
            .map_err(|_| MemoryObjectError::ResourceLimit)?;
        Ok(Self(ManuallyDrop::new(SourcePayload {
            list,
            disposal,
        })))
    }

    fn interest(&self) -> Interest {
        Interest(self.0.list.clone())
    }

    fn cancel(self) {
        // Ordinary losing/unused preparation is disposed after local guards.
        drop(ManuallyDrop::into_inner(self.0));
    }

    pub(super) fn finish(self) {
        let payload = ManuallyDrop::into_inner(self.0);
        let waiters = payload.list.close();
        deferred::publish(waiters, Some(payload.disposal.publish(payload.list)));
    }
}

pub(super) struct Interest(List);

impl Interest {
    pub(super) fn completed(&self) -> bool {
        self.0.is_closed()
    }
}

impl Observable for Interest {
    fn try_register_waiter(
        &self,
        observer: Weak<dyn Observer>,
        sponsor: &WaitSponsor,
    ) -> Result<WaitRegistration, RegistrationError> {
        if self.completed() {
            return Ok(WaitRegistration::ready());
        }
        match sponsor.register(&self.0, observer) {
            Err(RegistrationError::Closed) => Ok(WaitRegistration::ready()),
            other => other,
        }
    }
}

struct ClosePayload {
    root: Option<AddressSpaceOperation>,
    object: MemoryObjectId,
    interest: Option<Interest>,
    platform: bool,
}

/// Keep the exact root and transient interest together. Fallback retains all
/// fields and the root count; only ordinary consuming completion releases it.
struct Close(ManuallyDrop<ClosePayload>);

impl Close {
    fn finish(self, result: Result<(), MemoryObjectError>) -> Result<(), MemoryObjectError> {
        let payload = ManuallyDrop::into_inner(self.0);
        drop(payload.interest);
        if let Some(root) = payload.root {
            root.release().map_err(revocation::operation_error)?;
        }
        result
    }

    fn run(
        &mut self,
        asid: AddressSpaceId,
        cap: MemoryObjectCap,
        mut wait: impl FnMut(&Interest) -> Result<(), MemoryObjectError>,
        mut prepare: impl FnMut(bool) -> Result<Source, MemoryObjectError>,
    ) -> Result<(), MemoryObjectError> {
        loop {
            let registry = MEMORY_OBJECTS.lock();
            let entry = registry
                .caps
                .get(&asid)
                .and_then(|caps| caps.caps.get(&cap))
                .filter(|entry| entry.object == self.0.object)
                .ok_or(MemoryObjectError::UnknownCapability)?;
            if !entry.transfer_in_flight {
                // Unified lookup still rejects queued/unobserved delivery.
                registry.lookup(asid, cap)?;
                return prepare_close_cap(registry, asid, cap)?.release();
            }
            let object =
                registry.objects.get(&self.0.object).ok_or(MemoryObjectError::UnknownCapability)?;
            if let Some(source) = &object.close_interest {
                let interest = source.interest();
                drop(registry);
                self.0.interest = Some(interest);
            } else {
                drop(registry);
                // Prepare outside the payload registry and lifecycle/IPC.
                let mut prepared = Some(prepare(self.0.platform)?);
                let mut registry = MEMORY_OBJECTS.lock();
                let active =
                    registry.caps.get(&asid).and_then(|caps| caps.caps.get(&cap)).is_some_and(
                        |entry| entry.object == self.0.object && entry.transfer_in_flight,
                    );
                let interest = if active {
                    let object = registry
                        .objects
                        .get_mut(&self.0.object)
                        .expect("active transfer object missing");
                    if object.close_interest.is_none() {
                        object.close_interest = prepared.take();
                    }
                    Some(object.close_interest.as_ref().unwrap().interest())
                } else {
                    None
                };
                drop(registry);
                if let Some(unused) = prepared {
                    unused.cancel();
                }
                if !active {
                    continue;
                }
                self.0.interest = interest;
            }
            wait(self.0.interest.as_ref().unwrap())?;
        }
    }
}

pub(super) fn run(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    wait: impl FnMut(&Interest) -> Result<(), MemoryObjectError>,
) -> Result<(), MemoryObjectError> {
    run_with_prepare(asid, cap, wait, Source::prepare)
}

fn run_with_prepare(
    asid: AddressSpaceId,
    cap: MemoryObjectCap,
    wait: impl FnMut(&Interest) -> Result<(), MemoryObjectError>,
    prepare: impl FnMut(bool) -> Result<Source, MemoryObjectError>,
) -> Result<(), MemoryObjectError> {
    // Lease only a wait: nonwaiting fixture callers may already serialize
    // lifecycle/table access and keep their existing path. The active transfer
    // stores its exact prepared source root; do not re-resolve a numeric ASID.
    let registry = MEMORY_OBJECTS.lock();
    if !transfer_in_flight(&registry, asid, cap) {
        registry.lookup(asid, cap)?;
        return prepare_close_cap(registry, asid, cap)?.release();
    }
    let object = registry.caps[&asid].caps[&cap].object;
    let handle =
        registry.objects[&object].transfer_root.expect("active source transfer root missing");
    drop(registry);
    let root = if asid == crate::memory::KERNEL_ASID {
        None // The permanent kernel namespace has no reusable slot.
    } else {
        Some(AddressSpaceOperation::acquire(handle).map_err(revocation::operation_error)?)
    };
    let platform = asid == crate::memory::KERNEL_ASID
        || root.as_ref().is_some_and(|root| {
            super::super::budget::platform_identity(asid) == Some(root.handle())
        });
    let mut operation = Close(ManuallyDrop::new(ClosePayload {
        root,
        object,
        interest: None,
        platform,
    }));
    let result = operation.run(asid, cap, wait, prepare);
    operation.finish(result)
}

pub(super) mod tests;
