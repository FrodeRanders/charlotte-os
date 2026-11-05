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
