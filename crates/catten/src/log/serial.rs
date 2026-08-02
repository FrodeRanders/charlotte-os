//! # AArch64 PL011 UART Serial Console
//!
//! A minimal driver for the ARM PrimeCell PL011 UART, used as the kernel's
//! early and headless log sink on AArch64. On the QEMU `virt` machine the first
//! PL011 lives at physical address `0x0900_0000`; Limine has already configured
//! and enabled it before handing control to the kernel, so we only need to push
//! bytes into the transmit FIFO.
//!
//! The device is reached through the higher half direct map (HHDM). The kernel
//! requests Limine base revision 6, whose HHDM does *not* cover MMIO, so
//! [`init`] maps the PL011 page explicitly before any output; the very first
//! bytes of `bsp_main` are therefore the console becoming usable as an *early*
//! console. The UART base is discovered from the SPCR ACPI table where
//! available (e.g. QEMU `sbsa-ref`, real ARM servers) and falls back to the
//! QEMU `virt` default otherwise.
//!
//! See the ARM PrimeCell UART (PL011) Technical Reference Manual (ARM DDI 0183).

use core::{
    fmt::{
        self,
        Write,
    },
    sync::atomic::{
        AtomicBool,
        AtomicUsize,
        Ordering,
    },
};

use spin::Mutex;

use crate::cpu::isa::{
    interface::memory::address::PhysicalAddress,
    memory::address::paddr::PAddr,
};

/// Fallback: QEMU `virt` PL011 UART0 MMIO physical base address.
const PL011_BASE_FALLBACK: usize = 0x0900_0000;

/// The resolved PL011 MMIO base, discovered from the SPCR ACPI table (or the
/// QEMU `virt` default when ACPI is absent). Read via atomics because `reg_ptr`
/// runs from `early_logln!` before any `init()` has completed.
static PL011_BASE: AtomicUsize = AtomicUsize::new(PL011_BASE_FALLBACK);

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
    // Discover the console UART base from the SPCR table before mapping. This
    // runs before the kernel heap is ready, so discovery walks the XSDT without
    // allocating. On QEMU `virt` SPCR reports the same `0x0900_0000`; on
    // `sbsa-ref` and real ARM servers the console PL011 lives elsewhere.
    #[cfg(feature = "acpi")]
    if let Some(base) = crate::environment::acpi::sdt::discovery::spcr_uart_base() {
        PL011_BASE.store(base as usize, Ordering::Release);
    }
    use crate::memory::KERNEL_AS;
    KERNEL_AS
        .lock()
        .map_mmio_region(PL011_BASE.load(Ordering::Acquire), 0x1000)
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
        unsafe {
            PAddr::from(PL011_BASE.load(Ordering::Acquire) as u64)
                .into_hhdm_mut::<u32>()
                .byte_add(offset)
        }
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
