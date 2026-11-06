//! Principal records outlive root slots. This value captures sponsorship, not
//! a root lease or permission to release physical memory.
use super::*;
use crate::klib::admission::{
    ByteAllocator,
    ByteCharge,
    ByteSponsor,
    SnapshotBuffers,
};

pub(crate) enum ByteBudgetKind {
    Buffer,
    NamespaceAccount,
    ReactorAccount,
}
pub(crate) trait ByteBudget: Clone + Copy + core::fmt::Debug {
    const KIND: ByteBudgetKind;
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct BufferBudget;
impl ByteBudget for BufferBudget {
    const KIND: ByteBudgetKind = ByteBudgetKind::Buffer;
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct NamespaceAccountBudget;
impl ByteBudget for NamespaceAccountBudget {
    const KIND: ByteBudgetKind = ByteBudgetKind::NamespaceAccount;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReactorAccountBudget;
impl ByteBudget for ReactorAccountBudget {
    const KIND: ByteBudgetKind = ByteBudgetKind::ReactorAccount;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PrincipalBytes<F: ByteBudget> {
    principal: Option<u64>,
    family: core::marker::PhantomData<F>,
}
pub(crate) type Sponsor = PrincipalBytes<BufferBudget>;
pub(crate) type NamespaceSponsor = PrincipalBytes<NamespaceAccountBudget>;
pub(crate) type ReactorSponsor = PrincipalBytes<ReactorAccountBudget>;

impl<F: ByteBudget> PrincipalBytes<F> {
    pub(crate) const fn unidentified() -> Self {
        Self {
            principal: None,
            family: core::marker::PhantomData,
        }
    }

    pub(in crate::memory) fn from_account(account: &crate::memory::translation::Account) -> Self {
        Self {
            principal: account.principal(),
            family: core::marker::PhantomData,
        }
    }

    /// The verified authority slot has already prepared this history record.
    pub(in crate::memory) fn verified(principal: u64) -> Self {
        assert_ne!(principal, 0);
        Self {
            principal: Some(principal),
            family: core::marker::PhantomData,
        }
    }

    pub(in crate::memory) fn principal(self) -> Option<u64> {
        self.principal
    }
}

fn byte_budget<F: ByteBudget>(entry: &mut History) -> &mut VectorBudget<1> {
    match F::KIND {
        ByteBudgetKind::Buffer => &mut entry.kernel_buffers,
        ByteBudgetKind::NamespaceAccount => &mut entry.namespace_account_bytes,
        ByteBudgetKind::ReactorAccount => &mut entry.reactor_account_bytes,
    }
}
impl<F: ByteBudget> ByteSponsor for PrincipalBytes<F> {
    fn reserve(&self, bytes: u64) -> Result<(), AllocError> {
        if let Some(principal) = self.principal {
            let mut history = HISTORY.lock();
            byte_budget::<F>(history.get_mut(&principal).ok_or(AllocError)?)
                .reserve([bytes])
                .map_err(|_| AllocError)?;
        }
        Ok(())
    }

    fn release(&self, bytes: u64) {
        if let Some(principal) = self.principal {
            let mut history = HISTORY.lock();
            byte_budget::<F>(history.get_mut(&principal).expect("byte principal missing"))
                .release([bytes])
                .expect("principal byte underflow");
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
    /// Permanent kernel and trusted raw preparation/fixtures only. Signed
    /// application construction must capture its prebound translation account.
    pub(crate) const fn unidentified(platform: bool) -> Self {
        Self {
            platform,
            sponsor: Sponsor::unidentified(),
        }
    }

    pub(crate) fn namespace_sponsor(self) -> NamespaceSponsor {
        NamespaceSponsor {
            principal: self.sponsor.principal,
            family: core::marker::PhantomData,
        }
    }

    pub(crate) fn reactor_sponsor(self) -> ReactorSponsor {
        ReactorSponsor {
            principal: self.sponsor.principal,
            family: core::marker::PhantomData,
        }
    }

    pub(crate) fn sponsor(self) -> Sponsor {
        self.sponsor
    }

    pub(crate) fn allocator(self) -> SnapshotAllocator {
        ByteAllocator::with_sponsor(self.platform, self.sponsor)
    }

    pub(crate) fn platform(self) -> bool {
        self.platform
    }

    pub(in crate::memory) fn reserve_bytes(
        self,
        bytes: u64,
    ) -> Result<ByteCharge<Sponsor>, AllocError> {
        ByteCharge::try_new(self.sponsor, bytes)
    }
}

pub(in crate::memory) fn capture_unpublished(
    root: &crate::memory::AddressSpace,
) -> CapturedStorage {
    CapturedStorage {
        platform: root.table_account.is_platform(),
        sponsor: Sponsor::from_account(&root.table_account),
    }
}

#[cfg(any(test, feature = "kernel_tests"))]
impl From<bool> for CapturedStorage {
    fn from(platform: bool) -> Self {
        Self::unidentified(platform)
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
mod reactor_tests;
#[cfg(any(test, feature = "kernel_tests"))]
mod tests;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_admission() {
    tests::run();
    reactor_tests::run();
}

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

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_account_used(principal: u64) -> u64 {
    HISTORY.lock().get(&principal).map_or(0, |entry| entry.namespace_account_bytes.used()[0])
}
#[cfg(feature = "kernel_tests")]
pub(crate) fn test_account_limit(principal: u64, bytes: u64) {
    let mut history = HISTORY.lock();
    let budget = &mut history.get_mut(&principal).unwrap().namespace_account_bytes;
    let mut candidate = VectorBudget::new([bytes]);
    candidate.reserve(budget.used()).unwrap();
    *budget = candidate;
}

#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) fn test_reactor_used(principal: u64) -> u64 {
    HISTORY.lock().get(&principal).map_or(0, |entry| entry.reactor_account_bytes.used()[0])
}
#[cfg(feature = "kernel_tests")]
pub(crate) fn test_reactor_limit(principal: u64, bytes: u64) {
    let mut history = HISTORY.lock();
    let budget = &mut history.get_mut(&principal).unwrap().reactor_account_bytes;
    let mut candidate = VectorBudget::new([bytes]);
    candidate.reserve(budget.used()).unwrap();
    *budget = candidate;
}
