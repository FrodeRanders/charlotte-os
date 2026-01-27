use crate::cpu::isa::memory::address::vaddr::VAddr;
use crate::memory::AddressSpaceId;

pub fn inval_range_kernel(base: VAddr, size: usize) {
    todo!()
}

pub fn inval_range_user(asid: AddressSpaceId, base: VAddr, size: usize) {
    todo!()
}

pub fn inval_asid(asid: AddressSpaceId) {
    todo!()
}
