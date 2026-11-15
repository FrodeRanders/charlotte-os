//! Early AArch64 serial console: firmware-described PL011 or byte-MMIO 16550.
//!
//! SPCR selects the register family, access width and physical address. Only a
//! missing SPCR permits the historical QEMU-virt PL011 fallback. Firmware owns
//! initial UART programming; this adapter maps its window and transmits without
//! allocation or interrupt-driven waits. Unsupported/failed consoles disable
//! output so that framebuffer diagnostics and boot can still make progress.

use core::{
    fmt::{
        self,
        Write,
    },
    sync::atomic::{
        AtomicBool,
        AtomicU8,
        AtomicUsize,
        Ordering,
    },
};

use spin::Mutex;

use crate::{
    cpu::isa::interface::memory::address::PhysicalAddress,
    environment::serial_port::{
        SerialPort,
        UartKind,
    },
    memory::PAddr,
};

const PL011: u8 = 1;
const MMIO_16550: u8 = 2;
const PL011_FR: usize = 0x18;
const PL011_TXFF: u32 = 1 << 5;
const PL011_RXFE: u32 = 1 << 4;
const LSR: usize = 5;
const THRE: u8 = 1 << 5;
const DATA_READY: u8 = 1;
// Early/panic logging has no initialized IRQ/CQ notification contract. Bound
// each transmitter observation; a stuck/missing UART disables this sink rather
// than holding the logger/IRQ mask indefinitely. Never enable IRQs to wait.
const TRANSMIT_POLLS: usize = 100_000;

static BASE: AtomicUsize = AtomicUsize::new(0);
static KIND: AtomicU8 = AtomicU8::new(0);
static READY: AtomicBool = AtomicBool::new(false);

pub static SERIAL: Mutex<SerialConsole> = Mutex::new(SerialConsole);

pub fn init() {
    if READY.load(Ordering::Acquire) {
        return;
    }
    let fallback = SerialPort {
        kind: UartKind::Pl011,
        base: 0x0900_0000,
    };
    #[cfg(feature = "acpi")]
    let port = match crate::environment::acpi::sdt::discovery::spcr_console_uart() {
        Ok(Some(port)) => port,
        Ok(None) => fallback,
        Err(_) => return,
    };
    #[cfg(not(feature = "acpi"))]
    let port = fallback;
    if crate::memory::KERNEL_AS.lock().map_mmio_region(port.base, 0x1000).is_err() {
        return;
    }
    BASE.store(port.base, Ordering::Relaxed);
    KIND.store(
        match port.kind {
            UartKind::Pl011 => PL011,
            UartKind::Mmio16550 => MMIO_16550,
        },
        Ordering::Relaxed,
    );
    READY.store(true, Ordering::Release);
}

pub struct SerialConsole;

impl SerialConsole {
    fn pointer(offset: usize) -> *mut u8 {
        // Kernel hardware adapter: the validated firmware window was mapped
        // before READY publication and remains mapped for the boot lifetime.
        unsafe { PAddr::from((BASE.load(Ordering::Relaxed) + offset) as u64).into_hhdm_mut::<u8>() }
    }

    fn put_byte(&self, byte: u8) -> bool {
        if !READY.load(Ordering::Acquire) {
            return false;
        }
        for _ in 0..TRANSMIT_POLLS {
            let ready = match KIND.load(Ordering::Relaxed) {
                PL011 => {
                    let flags = unsafe { Self::pointer(PL011_FR).cast::<u32>().read_volatile() };
                    if flags == u32::MAX {
                        break;
                    }
                    flags & PL011_TXFF == 0
                }
                MMIO_16550 => {
                    let status = unsafe { Self::pointer(LSR).read_volatile() };
                    if status == u8::MAX {
                        break;
                    }
                    status & THRE != 0
                }
                _ => break,
            };
            if ready {
                unsafe {
                    if KIND.load(Ordering::Relaxed) == PL011 {
                        Self::pointer(0).cast::<u32>().write_volatile(byte as u32);
                    } else {
                        Self::pointer(0).write_volatile(byte);
                    }
                }
                return true;
            }
            core::hint::spin_loop();
        }
        READY.store(false, Ordering::Release);
        false
    }

    pub fn try_get_byte(&self) -> Option<u8> {
        if !READY.load(Ordering::Acquire) {
            return None;
        }
        match KIND.load(Ordering::Relaxed) {
            PL011 => {
                let flags = unsafe { Self::pointer(PL011_FR).cast::<u32>().read_volatile() };
                (flags != u32::MAX && flags & PL011_RXFE == 0)
                    .then(|| unsafe { Self::pointer(0).cast::<u32>().read_volatile() as u8 })
            }
            MMIO_16550 => {
                let status = unsafe { Self::pointer(LSR).read_volatile() };
                (status != u8::MAX && status & DATA_READY != 0)
                    .then(|| unsafe { Self::pointer(0).read_volatile() })
            }
            _ => None,
        }
    }
}

impl Write for SerialConsole {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if !READY.load(Ordering::Acquire) {
            return Ok(());
        }
        for byte in text.bytes() {
            if byte == b'\n' && !self.put_byte(b'\r') {
                return Err(fmt::Error);
            }
            if !self.put_byte(byte) {
                return Err(fmt::Error);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(align(4))]
    struct Registers([u8; 64]);

    fn select(registers: &mut Registers, kind: u8) {
        BASE.store(registers.0.as_mut_ptr() as usize, Ordering::Relaxed);
        KIND.store(kind, Ordering::Relaxed);
        READY.store(true, Ordering::Release);
    }

    #[test]
    fn transmit_uses_the_selected_register_width_and_status_offset() {
        let mut registers = Registers([0; 64]);
        registers.0[LSR] = THRE;
        registers.0[1] = 0xa5;
        select(&mut registers, MMIO_16550);
        SerialConsole.write_str("A").unwrap();
        assert_eq!(&registers.0[..2], &[b'A', 0xa5]);

        registers.0[PL011_FR] = 0;
        select(&mut registers, PL011);
        SerialConsole.write_str("B").unwrap();
        assert_eq!(&registers.0[..4], &[b'B', 0, 0, 0]);
    }

    #[test]
    fn receive_requires_family_specific_data_ready() {
        let mut registers = Registers([0; 64]);
        registers.0[0] = b'R';
        select(&mut registers, MMIO_16550);
        assert_eq!(SerialConsole.try_get_byte(), None);
        registers.0[LSR] = DATA_READY;
        assert_eq!(SerialConsole.try_get_byte(), Some(b'R'));
        select(&mut registers, PL011);
        registers.0[PL011_FR] = PL011_RXFE as u8;
        assert_eq!(SerialConsole.try_get_byte(), None);
        unsafe { registers.0.as_mut_ptr().add(PL011_FR).write_volatile(0) };
        assert_eq!(SerialConsole.try_get_byte(), Some(b'R'));
    }

    #[test]
    fn stuck_or_missing_transmitter_disables_output() {
        for (kind, status) in
            [(PL011, PL011_TXFF), (PL011, u32::MAX), (MMIO_16550, 0), (MMIO_16550, 0xff)]
        {
            let mut registers = Registers([0; 64]);
            if kind == PL011 {
                registers.0[PL011_FR..PL011_FR + 4].copy_from_slice(&status.to_le_bytes());
            } else {
                registers.0[LSR] = status as u8;
            }
            select(&mut registers, kind);
            assert!(SerialConsole.write_str("X").is_err());
            assert!(!READY.load(Ordering::Acquire));
            assert_eq!(registers.0[0], 0);
            assert_eq!(SerialConsole.write_str("disabled"), Ok(()));
        }
    }
}
