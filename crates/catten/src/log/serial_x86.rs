//! # x86_64 16550 UART Serial Console (COM1)
//!
//! A minimal driver for the standard PC 16550-compatible UART at COM1
//! (I/O port `0x3F8`), used as the kernel's headless log sink on x86_64. Unlike
//! the AArch64 PL011 (which is MMIO), the PC UART uses port I/O (`in`/`out`),
//! so no page mapping is required — it is usable from the very first
//! instruction.
//!
//! [`init`] performs the standard 16550 initialization (disable interrupts,
//! set 38400 8N1, enable FIFO). Output is dropped until `init` has run.

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

use spin::Mutex;

/// COM1 base I/O port.
const COM1: u16 = 0x3F8;

/// Register offsets from the base port.
const DATA: u16 = 0; // Data register (DLAB=0)
const IER: u16 = 1; // Interrupt Enable Register (DLAB=0)
const DLL: u16 = 0; // Divisor Latch Low (DLAB=1)
const DLH: u16 = 1; // Divisor Latch High (DLAB=1)
const FCR: u16 = 2; // FIFO Control Register
const LCR: u16 = 3; // Line Control Register
const MCR: u16 = 4; // Modem Control Register
const LSR: u16 = 5; // Line Status Register

/// LSR bit: transmit holding register empty.
const LSR_THRE: u8 = 1 << 5;

/// The global serial console instance.
pub static SERIAL: Mutex<Uart16550> = Mutex::new(Uart16550);

static READY: AtomicBool = AtomicBool::new(false);

#[inline]
unsafe fn outb(port: u16, value: u8) {
    unsafe {
        core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack, preserves_flags));
    }
}

#[inline]
unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        core::arch::asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack, preserves_flags));
    }
    value
}

/// Initialize COM1: 38400 baud, 8N1, FIFO enabled. Safe to call more than once.
pub fn init() {
    unsafe {
        outb(COM1 + IER, 0x00); // disable interrupts
        outb(COM1 + LCR, 0x80); // enable DLAB (set baud divisor)
        outb(COM1 + DLL, 0x03); // divisor low  = 3 → 38400 baud
        outb(COM1 + DLH, 0x00); // divisor high = 0
        outb(COM1 + LCR, 0x03); // 8 bits, no parity, one stop bit; DLAB=0
        outb(COM1 + FCR, 0xC7); // enable FIFO, clear, 14-byte threshold
        outb(COM1 + MCR, 0x0B); // RTS/DSR set, OUT2 (IRQ enable line)
    }
    READY.store(true, Ordering::Release);
}

pub struct Uart16550;

impl Uart16550 {
    #[inline]
    fn is_tx_ready() -> bool {
        unsafe { inb(COM1 + LSR) & LSR_THRE != 0 }
    }

    #[inline]
    pub fn put_byte(&self, byte: u8) {
        while !Self::is_tx_ready() {
            core::hint::spin_loop();
        }
        unsafe {
            outb(COM1 + DATA, byte);
        }
    }

    /// Transmit a string, translating newlines to CRLF. Output is dropped until
    /// [`init`] has run.
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

impl Write for Uart16550 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.write_bytes(s);
        Ok(())
    }
}
