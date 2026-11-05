//! Principal records outlive root slots. This value captures sponsorship, not
//! a root lease or permission to release physical memory.
use super::*;
use crate::klib::admission::{
    ByteAllocator,
    ByteSponsor,
    SnapshotBuffers,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Sponsor {
    principal: Option<u64>,
}

impl Sponsor {
    pub(in crate::memory) const fn unidentified() -> Self {
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

pub(crate) type TelemetryAllocator = ByteAllocator<SnapshotBuffers, Sponsor>;

/// Capture identity and trusted class under the same root-table hold, before
/// source locks or any allocation. Signed roots are prebound by the verified
/// loader. Unidentified kernel/raw fixture roots keep node admission only;
/// this is not a fallback for an authenticated application or a caller ID.
pub(crate) fn telemetry_allocator(
    asid: crate::memory::AddressSpaceId,
) -> Result<TelemetryAllocator, AllocError> {
    let table = crate::memory::ADDRESS_SPACE_TABLE.lock();
    let root = table.get(asid).map_err(|_| AllocError)?;
    let sponsor = Sponsor {
        principal: root.table_account.principal(),
    };
    let platform = asid == crate::memory::KERNEL_ASID || root.table_account.is_platform();
    Ok(ByteAllocator::with_sponsor(platform, sponsor))
}

#[cfg(any(test, feature = "kernel_tests"))]
mod tests;
#[cfg(any(test, feature = "kernel_tests"))]
pub(crate) use tests::run as test_admission;

#[cfg(any(test, feature = "kernel_tests"))]
pub(in crate::memory) fn test_used(principal: u64) -> u64 {
    HISTORY.lock().get(&principal).map_or(0, |entry| entry.kernel_buffers.used()[0])
}
#[cfg(feature = "kernel_tests")]
pub(in crate::memory) fn test_limit(principal: u64, bytes: u64) {
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
