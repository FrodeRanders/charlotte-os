//! # Multiple APIC Description Table (MADT)

mod ioapic;
mod nmi_source;

use alloc::vec::Vec;
use core::ptr::NonNull;

use crate::cpu::isa::interface::memory::address::VirtualAddress;
use crate::environment::acpi::SdtHeader;
use crate::memory::VAddr;

type GlobalSystemInterruptVector = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MadtEntryType {
    LocalApic = 0x0,
    IoApic = 0x1,
    InterruptSourceOverride = 0x2,
    NmiSource = 0x3,
    LocalApicNmi = 0x4,
    LocalApicAddressOverride = 0x5,
    IoSapic = 0x6,
    LocalSapic = 0x7,
    PlatformInterruptSource = 0x8,
    ProcessorLocalX2Apic = 0x9,
    LocalX2ApicNmi = 0xa,
    GicCpuInterface = 0xb,
    GicDistributor = 0xc,
    GicMsiFrame = 0xd,
    GicRedistributor = 0xe,
    GicInterruptTranslationService = 0xf,
    MultiprocessorWakeup = 0x10,
    CoreProgrammableInterruptController = 0x11,
    LegacyIoProgrammableInterruptController = 0x12,
    HyperTransportProgrammableInterruptController = 0x13,
    ExtendIoProgrammableInterruptController = 0x14,
    MsiProgrammableInterruptController = 0x15,
    BridgeIoProgrammableInterruptController = 0x16,
    LowPinCountProgrammableInterruptController = 0x17,
    RiscVHartLocalInterruptController = 0x18,
    RiscVIncomingMsiController = 0x19,
    RiscVAdvancedPlatformLevelInterruptController = 0x1a,
    RiscVPlatformLevelInterruptController = 0x1b,
}

#[derive(Debug)]
#[repr(C, packed)]
struct MadtEntryGeneric {
    entry_type: u8,
    entry_length: u8,
}

struct MadtEntryIter {
    ptr: Option<NonNull<MadtEntryGeneric>>,
    end_ptr: VAddr,
}

impl MadtEntryIter {
    pub fn new(madt_ptr: *const Madt) -> Self {
        Self {
            ptr: unsafe {
                NonNull::new((madt_ptr as *const u8).add(core::mem::size_of::<Madt>())
                    as *mut MadtEntryGeneric)
            },
            end_ptr: VAddr::from_ptr(unsafe {
                (madt_ptr as *const u8).add((*madt_ptr).header.length as usize)
            }),
        }
    }
}

impl Iterator for MadtEntryIter {
    type Item = NonNull<MadtEntryGeneric>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(nn_ptr) = self.ptr {
            let entry_length = unsafe { nn_ptr.read() }.entry_length;
            if VAddr::from_ptr(unsafe { nn_ptr.as_ptr().add(entry_length as usize) }) > self.end_ptr
            {
                self.ptr = None;
            } else {
                self.ptr = NonNull::new(unsafe { nn_ptr.as_ptr().add(entry_length as usize) });
            }
        }
        self.ptr
    }
}

const NUM_ENTRY_TYPES: usize = 28usize;

pub struct MadtEntryIndex {
    ptr_matrix: [Vec<NonNull<MadtEntryGeneric>>; NUM_ENTRY_TYPES],
}

impl MadtEntryIndex {
    pub fn get_type(&self, entry_type: MadtEntryType) -> &Vec<NonNull<MadtEntryGeneric>> {
        &self.ptr_matrix[entry_type as usize]
    }
}

#[derive(Debug)]
#[repr(C, packed)]
pub struct Madt {
    header: SdtHeader,
    lapic_address: u32,
    flags: u32,
}

impl Madt {
    pub fn parse(&self) -> MadtEntryIndex {
        let mut ptr_matrix: [Vec<NonNull<MadtEntryGeneric>>; NUM_ENTRY_TYPES] = Default::default();
        let iter = MadtEntryIter::new(self);
        for entry_ptr in iter {
            let entry_type = unsafe { entry_ptr.as_ref() }.entry_type as usize;
            if entry_type < NUM_ENTRY_TYPES {
                ptr_matrix[entry_type].push(entry_ptr);
            }
        }
        MadtEntryIndex {
            ptr_matrix,
        }
    }
}
