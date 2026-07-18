//! # National Semiconductor 16x50 Series Compatible UART Driver

use alloc::vec::Vec;
use core::{
    fmt::{
        self,
        Write,
    },
    result::Result,
};

use crate::{
    cpu::isa::{
        interface::io::{
            IReg8Ifce,
            OReg8Ifce,
        },
        io::IoReg8,
    },
    klib::io::Read,
};

pub struct Ns16x50Driver {
    #[allow(dead_code)]
    ports: Vec<Ns16x50>,
}

#[derive(Copy, Clone, Debug)]
pub enum IfceType {
    Ns16550,
    Ns16550A,
    Ns16650,
    Ns16750,
    Ns16850,
    Ns16950,
}

impl IfceType {
    #[allow(dead_code)]
    fn queue_size(&self) -> usize {
        match self {
            IfceType::Ns16550 => 0, // Original 16550 had a broken FIFO
            IfceType::Ns16550A => 16,
            IfceType::Ns16650 => 32,
            IfceType::Ns16750 => 64,
            IfceType::Ns16850 => 128,
            IfceType::Ns16950 => 256,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Ns16x50 {
    #[allow(dead_code)]
    ifce: IfceType,
    base: IoReg8,
}
#[derive(Debug, Clone, Copy)]
pub enum Error {
    FailedSelfTest,
}

impl Ns16x50 {
    fn is_transmit_empty(&self) -> i32 {
        (unsafe { (self.base + 5).read() } & 0x20).into()
    }

    fn received(&self) -> bool {
        (unsafe { (self.base + 5).read() } & 1) != 0
    }

    fn read_char(&self) -> char {
        while !self.received() {}
        unsafe { (self.base).read() as char }
    }

    #[allow(dead_code)]
    fn try_new(ifce: IfceType, base: IoReg8) -> Result<Self, Error> {
        let port = Ns16x50 {
            ifce: ifce, // Use the provided interface type
            base: base,
        };
        unsafe {
            (port.base + 1).write(0x00); // Disable all interrupts
            (port.base + 3).write(0x80); // Enable DLAB (set baud rate divisor)
            (port.base + 0).write(0x01); // Set divisor to 1 (lo byte) 115200 baud
            (port.base + 1).write(0x00); //                  (hi byte)
            (port.base + 3).write(0x03); // 8 bits, no parity, one stop bit
            (port.base + 2).write(0xc7); // Enable FIFO, clear them, with 14-byte threshold
            (port.base + 4).write(0x0b); // IRQs enabled, RTS/DSR set
            (port.base + 4).write(0x1e); // Set in loopback mode, test the serial chip
            (port.base + 0).write(0xae); // Test serial chip (send byte 0xAE and check if serial returns same byte)

            if port.base.read() != 0xae {
                Err(Error::FailedSelfTest)
            } else {
                (port.base + 4).write(0x0f);
                Ok(port)
            }
        }
    }
}

impl Write for Ns16x50 {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for c in s.chars() {
            self.write_char(c)?
        }
        Ok(())
    }

    fn write_char(&mut self, c: char) -> fmt::Result {
        while self.is_transmit_empty() == 0 {}
        if c.is_ascii() {
            if c == '\n' {
                unsafe {
                    (self.base).write('\r' as u8);
                    (self.base).write('\n' as u8);
                }
            } else {
                unsafe {
                    (self.base).write(c as u8);
                }
            }
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}

impl Read for Ns16x50 {
    fn read(&mut self, buf: &mut [u8]) -> usize {
        for i in 0..buf.len() {
            buf[i] = self.read_char() as u8;
        }
        buf.len()
    }
}

unsafe impl Sync for Ns16x50 {}
