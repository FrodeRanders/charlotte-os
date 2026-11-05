//! Principal records outlive root slots. This value captures sponsorship, not
//! a root lease or permission to release physical memory.
use super::*;
use crate::klib::admission::{
    ByteAllocator,
    ByteCharge,
    ByteSponsor,
    SnapshotBuffers,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Sponsor {
    principal: Option<u64>,
}

impl Sponsor {
    pub(crate) const fn unidentified() -> Self {
        Self {
            principal: None,
        }
    }

    pub(in crate::memory) fn from_account(account: &crate::memory::translation::Account) -> Self {
        Self {
            principal: account.principal(),
        }
    }

    /// The verified authority slot has already prepared this history record.
    pub(in crate::memory) fn verified(principal: u64) -> Self {
        assert_ne!(principal, 0);
        Self {
            principal: Some(principal),
        }
    }

    pub(in crate::memory) fn principal(self) -> Option<u64> {
        self.principal
    }
}

impl ByteSponsor for Sponsor {
    fn reserve(&self, bytes: u64) -> Result<(), AllocError> {
        if let Some(principal) = self.principal {
            HISTORY
                .lock()
                .get_mut(&principal)
                .ok_or(AllocError)?
                .kernel_buffers
                .reserve([bytes])
                .map_err(|_| AllocError)?;
        }
        Ok(())
    }

    fn release(&self, bytes: u64) {
        if let Some(principal) = self.principal {
            HISTORY
                .lock()
                .get_mut(&principal)
                .expect("buffer principal missing")
                .kernel_buffers
                .release([bytes])
                .expect("principal buffer underflow");
        }
    }
}

pub(crate) type SnapshotAllocator = ByteAllocator<SnapshotBuffers, Sponsor>;

/// Metadata-only capture. It owns no root lease or resource grant; the calling
/// operation retains its exact root separately. Refund uses only this original
/// boot-lifetime principal, never a later ASID or worker lookup.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CapturedStorage {
    platform: bool,
    sponsor: Sponsor,
}
impl CapturedStorage {
    pub(crate) fn sponsor(self) -> Sponsor {
        self.sponsor
    }

    pub(crate) fn allocator(self) -> SnapshotAllocator {
        ByteAllocator::with_sponsor(self.platform, self.sponsor)
    }

    pub(in crate::memory) fn platform(self) -> bool {
        self.platform
    }

    pub(in crate::memory) fn reserve_bytes(
        self,
        bytes: u64,
    ) -> Result<ByteCharge<Sponsor>, AllocError> {
        ByteCharge::try_new(self.sponsor, bytes)
    }
}

fn captured(
    root: &crate::memory::AddressSpace,
    asid: crate::memory::AddressSpaceId,
) -> CapturedStorage {
    CapturedStorage {
        platform: asid == crate::memory::KERNEL_ASID || root.table_account.is_platform(),
        sponsor: Sponsor::from_account(&root.table_account),
    }
}

pub(crate) fn capture_storage(
    origin: crate::memory::AddressSpaceHandle,
) -> Result<CapturedStorage, AllocError> {
    let table = crate::memory::ADDRESS_SPACE_TABLE.lock();
    if table.generation(origin.id()).ok() != Some(origin.generation()) {
        return Err(AllocError);
    }
    let root = table.get(origin.id()).map_err(|_| AllocError)?;
    Ok(captured(root, origin.id()))
}

/// Capture identity and trusted class under the same root-table hold, before
/// source locks or any allocation. Signed roots are prebound by the verified
/// loader. Unidentified kernel/raw fixture roots keep node admission only;
/// this is not a fallback for an authenticated application or a caller ID.
pub(crate) fn telemetry_allocator(
    asid: crate::memory::AddressSpaceId,
) -> Result<SnapshotAllocator, AllocError> {
    let table = crate::memory::ADDRESS_SPACE_TABLE.lock();
    let root = table.get(asid).map_err(|_| AllocError)?;
    Ok(captured(root, asid).allocator())
}

#[cfg(any(test, feature = "kernel_tests"))]
mod tests;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) use tests::run as test_admission;

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_used(principal: u64) -> u64 {
    HISTORY.lock().get(&principal).map_or(0, |entry| entry.kernel_buffers.used()[0])
}
#[cfg(feature = "kernel_tests")]
pub(crate) fn test_limit(principal: u64, bytes: u64) {
    let mut history = HISTORY.lock();
    let budget = &mut history.get_mut(&principal).unwrap().kernel_buffers;
    let mut candidate = VectorBudget::new([bytes]);
    candidate.reserve(budget.used()).unwrap(); // Preserve every existing charge.
    *budget = candidate;
}
#[cfg(feature = "kernel_tests")]
pub(in crate::memory) fn test_with_backing_limit(principal: u64, pages: u64, work: impl FnOnce()) {
    let previous = {
        let mut history = HISTORY.lock();
        let entry = history.get_mut(&principal).unwrap();
        let previous = entry.backing.limit();
        entry
            .backing
            .set_limit(Amount {
                pages,
                objects: previous.objects,
            })
            .unwrap();
        previous
    };
    work();
    HISTORY.lock().get_mut(&principal).unwrap().backing.set_limit(previous).unwrap();
}

#[cfg(feature = "kernel_tests")]
pub(crate) fn test_with_history_locked<R>(work: impl FnOnce() -> R) -> R {
    let _history = HISTORY.lock();
    work()
}
