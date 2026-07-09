mod overlay;
mod redirection_table_entry;

use overlay::*;
use redirection_table_entry::*;

use crate::{
    cpu::isa::lp::LpId,
    klib::bitwise::mask_shift_read,
};

const IOAPIC_ID_SHIFT: u8 = 24;
const IOAPIC_ID_MASK: u32 = 0xfu32 << IOAPIC_ID_SHIFT;

const IOAPIC_VER_SHIFT: u8 = 0;
const IOAPIC_VER_MASK: u32 = 0xffu32 << IOAPIC_VER_SHIFT;

const IOAPIC_MAX_REDIR_SHIFT: u8 = 16;
const IOAPIC_MAX_REDIR_MASK: u32 = 0xffu32 << IOAPIC_MAX_REDIR_SHIFT;

pub enum Error {
    InvalidDeliveryMode(u8),
    LpIdOutOfRange(LpId),
    RedirIndexOutOfRange(RedirIdx),
}

#[repr(transparent)]
pub struct IoApic(*mut IoApicOverlay);

pub type RedirIdx = u32;
impl IoApic {
    //const ARB_REG_IDX: u32 = 2;
    const ID_REG_IDX: u32 = 0;
    const REDIR_TABLE_BASE_IDX: u32 = 16;
    const VER_ENTRY_MAX_REG_IDX: u32 = 1;

    pub fn get_id(&self) -> u32 {
        let ioapic_id_reg = unsafe { (*self.0).read32(Self::ID_REG_IDX) };
        mask_shift_read(ioapic_id_reg, IOAPIC_ID_MASK, IOAPIC_ID_SHIFT)
    }

    pub fn get_version(&self) -> u32 {
        let ioapic_ver_reg = unsafe { (*self.0).read32(Self::VER_ENTRY_MAX_REG_IDX) };
        mask_shift_read(ioapic_ver_reg, IOAPIC_VER_MASK, IOAPIC_VER_SHIFT)
    }

    pub fn get_max_redirection_entry(&self) -> u32 {
        let ioapic_entry_max_reg = unsafe { (*self.0).read32(Self::VER_ENTRY_MAX_REG_IDX) };
        mask_shift_read(ioapic_entry_max_reg, IOAPIC_MAX_REDIR_MASK, IOAPIC_MAX_REDIR_SHIFT)
    }

    pub fn get_redirection_entry(&self, index: RedirIdx) -> IoApicRedirEntry {
        let redir_entry = unsafe { (*self.0).read64(Self::REDIR_TABLE_BASE_IDX + index * 2) };
        IoApicRedirEntry::from(redir_entry)
    }

    pub fn set_redirection_entry(
        &mut self,
        index: RedirIdx,
        entry: IoApicRedirEntry,
    ) -> Result<(), Error> {
        const REDIR_SIZE_IN_IOAPIC_REGS: u32 = 2;

        if index > self.get_max_redirection_entry() {
            Err(Error::RedirIndexOutOfRange(index))
        } else {
            unsafe {
                (*self.0).write64(
                    Self::REDIR_TABLE_BASE_IDX + index * REDIR_SIZE_IN_IOAPIC_REGS,
                    entry.0,
                );
            };
            Ok(())
        }
    }
}
