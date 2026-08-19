mod redirection_table_entry;

use redirection_table_entry::*;

use crate::{
    cpu::isa::{
        interface::interrupts::ExternalInterruptControllerIfce,
        io::{
            IReg32Ifce,
            IoReg32,
            OReg32Ifce,
        },
        lp::{
            InterruptVectorNum,
            LpId,
        },
    },
    klib::bitwise::{
        mask_from_len,
        mask_shift_read,
        splice_into,
    },
};

/* The bitwise left shifts of various values within their containing registers */
const IOAPIC_ID_SHIFT: u8 = 24;
const IOAPIC_ID_MASK: u32 = 0xfu32 << IOAPIC_ID_SHIFT;

const IOAPIC_VER_SHIFT: u8 = 0;
const IOAPIC_VER_MASK: u32 = 0xffu32 << IOAPIC_VER_SHIFT;

const IOAPIC_MAX_REDIR_SHIFT: u8 = 16;
const IOAPIC_MAX_REDIR_MASK: u32 = 0xffu32 << IOAPIC_MAX_REDIR_SHIFT;

/// IOAPIC Error type
#[derive(Debug)]
pub enum Error {
    InvalidDeliveryMode(u8),
    LpIdOutOfRange(LpId),
    RedirIndexOutOfRange(RedirIdx),
}

#[repr(transparent)]
/// The IOAPIC struct is a transparent struct containing the base MMIO address of the IOAPIC
/// programming interface.
///
/// This interface uses indexed register access via two actual 32-bit MMIO
/// registers: The IOREGSEL register is used to select the register to access, and the actual data
/// read/write is performed using the 32-bit IOWIN register located immediately after it. 64-bit
/// registers are accessed by accessing their sequential lower and higher 32-bit halves in two
/// separate transactions one after another.
///
/// Ref: [IOAPIC - OSDev Wiki](https://wiki.osdev.org/IOAPIC)
pub struct IoApic(IoReg32);

type IoApicRegIdx = u32;
pub type RedirIdx = u32;
impl IoApic {
    //const ARB_REG_IDX: u32 = 2;
    const ID_REG_IDX: u32 = 0;
    const IOWIN_MMIO_BYTE_OFFSET: u16 = 0x10;
    const REDIR_TABLE_BASE_IDX: u32 = 16;
    const REG_BITS: u8 = 32;
    const VER_ENTRY_MAX_REG_IDX: u32 = 1;

    fn read32(&self, reg_idx: IoApicRegIdx) -> u32 {
        unsafe {
            self.0.write(reg_idx);
            (self.0 + Self::IOWIN_MMIO_BYTE_OFFSET).read()
        }
    }

    fn write32(&mut self, reg_idx: IoApicRegIdx, value: u32) {
        unsafe {
            self.0.write(reg_idx);
            (self.0 + Self::IOWIN_MMIO_BYTE_OFFSET).write(value);
        }
    }

    fn read64(&self, reg_idx: IoApicRegIdx) -> u64 {
        let low = self.read32(reg_idx) as u64;
        let high = self.read32(reg_idx + 1) as u64;
        let mut result = low;
        splice_into(&mut result, high, mask_from_len(Self::REG_BITS), Self::REG_BITS)
    }

    fn write64(&mut self, reg_idx: IoApicRegIdx, value: u64) {
        let low = mask_shift_read(value, mask_from_len(Self::REG_BITS), 0) as u32;
        let high = mask_shift_read(value, mask_from_len(Self::REG_BITS), Self::REG_BITS) as u32;
        self.write32(reg_idx, low);
        self.write32(reg_idx + 1, high);
    }

    pub fn get_id(&self) -> u32 {
        let ioapic_id_reg = self.read32(Self::ID_REG_IDX);
        mask_shift_read(ioapic_id_reg, IOAPIC_ID_MASK, IOAPIC_ID_SHIFT)
    }

    pub fn get_version(&self) -> u32 {
        let ioapic_ver_reg = self.read32(Self::VER_ENTRY_MAX_REG_IDX);
        mask_shift_read(ioapic_ver_reg, IOAPIC_VER_MASK, IOAPIC_VER_SHIFT)
    }

    pub fn get_max_redirection_entry(&self) -> u32 {
        let ioapic_entry_max_reg = self.read32(Self::VER_ENTRY_MAX_REG_IDX);
        mask_shift_read(ioapic_entry_max_reg, IOAPIC_MAX_REDIR_MASK, IOAPIC_MAX_REDIR_SHIFT)
    }

    pub fn get_redirection_entry(&self, index: RedirIdx) -> IoApicRedirEntry {
        let redir_entry = self.read64(Self::REDIR_TABLE_BASE_IDX + index * 2);
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
            self.write64(Self::REDIR_TABLE_BASE_IDX + index * REDIR_SIZE_IN_IOAPIC_REGS, entry.0);
            Ok(())
        }
    }
}

impl IoApic {
    /// Construct an I/O APIC controller at its MMIO base address.
    pub fn new(address: u64) -> Self {
        IoApic(IoReg32::Mmio(crate::memory::PAddr::from(address)))
    }

    /// Map the controller's MMIO register block into the kernel address space.
    /// Limine only HHDM-maps RAM, so the redirection table must be reachable
    /// before the first access.
    pub fn map_mmio(&mut self) {
        use crate::memory::KERNEL_AS;
        if let IoReg32::Mmio(address) = self.0 {
            let base = <crate::memory::PAddr as Into<usize>>::into(address);
            KERNEL_AS.lock().map_mmio_region(base, 0x1000).expect("failed to map IOAPIC MMIO");
        }
    }

    /// Route a GSI (a redirection-table pin) to `vector` on `target_lp`.
    pub fn route_gsi(
        &mut self,
        gsi: u32,
        target_lp: LpId,
        vector: InterruptVectorNum,
        active_low: bool,
        level_triggered: bool,
    ) -> Result<(), Error> {
        let dest = super::x2apic::X2Apic::physical_apic_id(target_lp)
            .ok_or(Error::LpIdOutOfRange(target_lp))?;
        let mut entry = IoApicRedirEntry::default();
        entry.set_vector(vector);
        entry.set_delivery_mode(IoApicDeliveryMode::Fixed);
        entry.set_dest_mode(false);
        entry.set_pin_polarity(active_low);
        entry.set_trigger_mode(level_triggered);
        entry.set_destination(dest as LpId)?;
        entry.set_mask_state(false);
        self.set_redirection_entry(gsi, entry)
    }
}

impl ExternalInterruptControllerIfce for IoApic {
    type EicPinNum = u32;
    type Error = Error;

    fn init(&mut self) {
        self.map_mmio();
    }

    fn setup_ext_int(
        &mut self,
        lp: LpId,
        vector: InterruptVectorNum,
        pin_num: Self::EicPinNum,
        active_low: bool,
        level_triggered: bool,
        mask_state: bool,
    ) -> Result<(), Self::Error> {
        let dest = super::x2apic::X2Apic::physical_apic_id(lp).ok_or(Error::LpIdOutOfRange(lp))?;
        let mut entry = IoApicRedirEntry::default();
        entry.set_vector(vector);
        entry.set_delivery_mode(IoApicDeliveryMode::Fixed);
        entry.set_dest_mode(false);
        entry.set_pin_polarity(active_low);
        entry.set_trigger_mode(level_triggered);
        entry.set_destination(dest as LpId)?;
        entry.set_mask_state(mask_state);
        self.set_redirection_entry(pin_num, entry)
    }

    fn set_ext_int_mask_state(
        &mut self,
        pin_num: Self::EicPinNum,
        mask_state: bool,
    ) -> Result<(), Self::Error> {
        let mut entry = self.get_redirection_entry(pin_num);
        entry.set_mask_state(mask_state);
        self.set_redirection_entry(pin_num, entry)
    }
}
