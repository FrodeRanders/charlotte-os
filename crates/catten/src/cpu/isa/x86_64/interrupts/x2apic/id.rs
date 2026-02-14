use alloc::collections::btree_map::BTreeMap;

use crate::cpu::isa::constants::msrs;
use crate::cpu::isa::lp::LpId;

pub(super) static mut X2APIC_ID_TABLE: BTreeMap<LpId, LapicId> = BTreeMap::new();

#[derive(Clone, Copy, Debug)]
#[repr(C, packed)]
pub struct LapicId {
    pub physical: PhysicalLapicId,
    pub logical:  LogicalLapicId,
}

impl LapicId {
    pub fn get_local() -> Self {
        let physical: PhysicalLapicId;
        let logical: u32;
        unsafe {
            core::arch::asm! {
                "mov ecx, {x2apic_id_reg:e}", // x2APIC ID Register
                "rdmsr",
                "mov [{physical:e}], eax",
                "mov ecx, {x2apic_logical_dest_reg:e}", // x2APIC Logical Destination Register
                "rdmsr",
                "mov [{logical:e}], eax",
                x2apic_id_reg = in(reg) msrs::x2apic::ID_REG,
                x2apic_logical_dest_reg = in(reg) msrs::x2apic::LOGICAL_DEST_REG,
                physical = out(reg) physical,
                logical = out(reg) logical,
            }
        }
        LapicId {
            physical,
            logical: unsafe { core::mem::transmute(logical) },
        }
    }
}
pub(super) type PhysicalLapicId = u32;
#[derive(Clone, Copy, Debug)]
#[repr(C, packed)]
pub(super) struct LogicalLapicId {
    cluster_id: u16,
    apic_bitmask: u16,
}
