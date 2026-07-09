//! # AArch64 PL011 UART Serial Console
//!
//! A minimal driver for the ARM PrimeCell PL011 UART, used as the kernel's
//! early and headless log sink on AArch64. On the QEMU `virt` machine the first
//! PL011 lives at physical address `0x0900_0000`; Limine has already configured
//! and enabled it before handing control to the kernel, so we only need to push
//! bytes into the transmit FIFO.
//!
//! The device is reached through the higher half direct map (HHDM). Because the
//! kernel requests Limine base revision 0 on AArch64, the low 4 GiB (including
//! this MMIO) is HHDM-mapped from the very first instruction of `bsp_main`,
//! which is what makes this usable as an *early* console.
//!
//! The MMIO base is the fixed QEMU `virt` default for now; once device-tree
//! parsing is implemented the UART should be discovered from the `/pl011` node.
//!
//! See the ARM PrimeCell UART (PL011) Technical Reference Manual (ARM DDI 0183).

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex;

use crate::cpu::isa::memory::address::paddr::PAddr;
use crate::cpu::isa::interface::memory::address::PhysicalAddress;

/// QEMU `virt` PL011 UART0 MMIO physical base address.
const PL011_BASE: usize = 0x0900_0000;

/// Data register: writing a byte transmits it.
const UARTDR: usize = 0x00;
/// Flag register.
const UARTFR: usize = 0x18;
/// Flag register bit: transmit FIFO full.
const UARTFR_TXFF: u32 = 1 << 5;

/// The global serial console instance guarding ordered access to the UART.
pub static SERIAL: Mutex<Pl011> = Mutex::new(Pl011);

/// Map the PL011 MMIO page into the kernel address space so it is reachable
/// through the HHDM, then mark the console ready.
///
/// This must be called before any `log`/`logln` output on AArch64. It is safe to
/// call more than once (subsequent calls are no-ops). Until it has run, output
/// is silently dropped rather than faulting on the unmapped MMIO page.
pub fn init() {
    use crate::cpu::isa::interface::memory::AddressSpaceInterface;
    use crate::memory::KERNEL_AS;
    KERNEL_AS
        .lock()
        .map_mmio_region(PL011_BASE, 0x1000)
        .expect("Failed to map PL011 UART MMIO region");
    READY.store(true, Ordering::Release);
}

/// Whether [`init`] has mapped the UART MMIO and the console may be used.
static READY: AtomicBool = AtomicBool::new(false);

pub struct Pl011;

impl Pl011 {
    #[inline]
    fn reg_ptr(offset: usize) -> *mut u32 {
        // SAFETY: PL011_BASE is a valid MMIO physical address that Limine maps
        // into the HHDM, and `offset` is within the device's register window.
        unsafe { PAddr::from(PL011_BASE as u64).into_hhdm_mut::<u32>().byte_add(offset) }
    }

    #[inline]
    fn is_tx_full() -> bool {
        unsafe { core::ptr::read_volatile(Self::reg_ptr(UARTFR)) & UARTFR_TXFF != 0 }
    }

    /// Transmit a single byte, spinning while the FIFO is full.
    #[inline]
    pub fn put_byte(&self, byte: u8) {
        while Self::is_tx_full() {
            core::hint::spin_loop();
        }
        unsafe {
            core::ptr::write_volatile(Self::reg_ptr(UARTDR), byte as u32);
        }
    }

    /// Transmit a string, translating newlines to CRLF so terminals render the
    /// output correctly. Output is dropped until [`init`] has mapped the UART.
    pub fn write_bytes(&self, s: &str) {
        if !READY.load(Ordering::Acquire) {
            return;
        }
        for byte in s.bytes() {
            if byte == b'\n' {
                self.put_byte(b'\r');
            }
            self.put_byte(byte);
        }
    }
}

impl Write for Pl011 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_bytes(s);
        Ok(())
    }
}
