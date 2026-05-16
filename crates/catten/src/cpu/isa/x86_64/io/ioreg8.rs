use core::arch::asm;
use core::ops::Add;

pub use crate::cpu::isa::interface::io::{IReg8Ifce, OReg8Ifce};
use crate::memory::PAddr;
use crate::memory::physical::PhysicalAddress;

#[derive(Copy, Clone, Debug)]
pub enum IoReg8 {
    IoPort(u16),
    Mmio(PAddr),
    PcieCfg {
        ecam_base: PAddr,
        bus: u8,
        device: u8,
        function: u8,
        offset: u16,
    },
}

impl IReg8Ifce for IoReg8 {
    unsafe fn read(&self) -> u8 {
        match self {
            IoReg8::IoPort(port) => {
                let value: u8;
                unsafe {
                    asm!(
                        "in al, dx",
                        in("dx") *port,
                        out("al") value,
                    );
                }
                value
            }
            IoReg8::Mmio(address) => unsafe { core::ptr::read_volatile(address.into_hhdm_ptr()) },
            IoReg8::PcieCfg {
                ecam_base,
                bus,
                device,
                function,
                offset,
            } => {
                let phys_addr = *ecam_base
                    + ((*bus as usize) << 20)
                    + ((*device as usize) << 15)
                    + ((*function as usize) << 12)
                    + (*offset as usize);
                unsafe { core::ptr::read_volatile(phys_addr.into_hhdm_ptr()) }
            }
        }
    }
}

impl OReg8Ifce for IoReg8 {
    unsafe fn write(&self, value: u8) {
        match self {
            IoReg8::IoPort(port) => unsafe {
                asm!(
                    "out dx, al",
                    in("dx") *port,
                    in("al") value,
                );
            },
            IoReg8::Mmio(address) => unsafe {
                core::ptr::write_volatile(address.into_hhdm_mut(), value)
            },
            IoReg8::PcieCfg {
                ecam_base,
                bus,
                device,
                function,
                offset,
            } => {
                let phys_addr = *ecam_base
                    + ((*bus as usize) << 20)
                    + ((*device as usize) << 15)
                    + ((*function as usize) << 12)
                    + (*offset as usize);
                unsafe { core::ptr::write_volatile(phys_addr.into_hhdm_mut(), value) }
            }
        }
    }
}

impl Add<u16> for IoReg8 {
    type Output = IoReg8;

    fn add(self, rhs: u16) -> Self::Output {
        match self {
            IoReg8::IoPort(port) => IoReg8::IoPort(port.wrapping_add(rhs)),
            IoReg8::Mmio(address) => IoReg8::Mmio(address + rhs as usize),
            IoReg8::PcieCfg {
                ecam_base,
                bus,
                device,
                function,
                offset,
            } => IoReg8::PcieCfg {
                ecam_base,
                bus,
                device,
                function,
                offset: offset.wrapping_add(rhs),
            },
        }
    }
}
