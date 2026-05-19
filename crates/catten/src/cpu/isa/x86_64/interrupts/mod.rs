//! # x86_64 Interrupt Management

pub mod fixed;
pub mod idt;
pub mod x2apic;

use idt::*;
use spin::{LazyLock, Mutex};

use crate::cpu::isa::interface::interrupts::InterruptManagerIfce;
use crate::cpu::isa::lp::LpId;
use crate::memory::IdTable;

pub type LocalIntCtlr = x2apic::X2Apic;

pub static BSP_IDT: Mutex<Idt> = Mutex::new(Idt::new());
pub static IDT_TABLE: LazyLock<IdTable<Mutex<Idt>>> = LazyLock::new(IdTable::new);

pub struct IsrDesc {
    pub target_lp: LpId,
    pub vector: u8,
    pub handler: extern "C" fn(),
}

pub struct InterruptManager;

#[derive(Debug)]
pub enum Error {}

impl InterruptManagerIfce for InterruptManager {
    type Error = Error;
    type IntDispatchNum = u8;
    type IsrDesc = IsrDesc;
    type LocalIntCtlr = x2apic::X2Apic;

    fn register_interrupt_handler(isrd: &Self::IsrDesc) -> Result<(), Self::Error> {
        todo!()
    }
}
