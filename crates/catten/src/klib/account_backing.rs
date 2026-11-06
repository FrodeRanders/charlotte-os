//! Shared actual-layout funding for fixed typed account bundles. The family
//! reservation spans both allocations; the captured sponsor funds each layout.
use core::alloc::AllocError;

use super::{
    admission::{
        BundleCharge,
        ByteSponsor,
        Family,
        SponsoredAllocator,
    },
    charged_allocator::{
        ChargedAllocator,
        PreAdmitted,
    },
};

pub(crate) type Allocator<F, S> =
    ChargedAllocator<BundleCharge<F>, PreAdmitted, SponsoredAllocator<S>>;

pub(crate) fn allocator<F: Family<1>, S: ByteSponsor>(
    platform: bool,
    sponsor: S,
) -> Result<Allocator<F, S>, AllocError> {
    ChargedAllocator::try_new_with_backing(
        BundleCharge::reserve(platform)?,
        SponsoredAllocator::new(sponsor),
    )
}
