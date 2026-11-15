#!/usr/bin/env python3
"""Run the production SPCR parser and UART accesses against host register memory."""
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
SOURCE = r'''
extern crate self as spin;
pub struct Mutex<T>(std::sync::Mutex<T>);
impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self { Self(std::sync::Mutex::new(value)) }
    pub fn lock(&self) -> std::sync::MutexGuard<'_, T> { self.0.lock().unwrap() }
}
mod cpu { pub mod isa { pub mod interface { pub mod memory { pub mod address {
    pub trait PhysicalAddress { unsafe fn into_hhdm_mut<T>(self) -> *mut T; }
}}}}}
mod environment {
    #[path = "@ROOT@/crates/catten/src/environment/serial_port.rs"]
    pub mod serial_port;
}
mod memory {
    pub struct PAddr(u64);
    impl From<u64> for PAddr { fn from(value: u64) -> Self { Self(value) } }
    impl crate::cpu::isa::interface::memory::address::PhysicalAddress for PAddr {
        unsafe fn into_hhdm_mut<T>(self) -> *mut T { self.0 as *mut T }
    }
    pub struct Memory;
    impl Memory { pub fn map_mmio_region(&mut self, _: usize, _: usize) -> Result<(), ()> { Ok(()) } }
    pub static KERNEL_AS: crate::Mutex<Memory> = crate::Mutex::new(Memory);
}
#[allow(dead_code)]
#[path = "@ROOT@/crates/catten/src/log/serial.rs"]
mod serial;
'''

with tempfile.TemporaryDirectory(prefix="charlotte-arm-console-") as temporary:
    directory = Path(temporary)
    source = directory / "console.rs"
    source.write_text(SOURCE.replace("@ROOT@", str(ROOT)))
    subprocess.run(["rustc", "--edition=2024", "--test", "-D", "warnings", str(source),
                    "-o", str(directory / "console")], cwd=ROOT, check=True)
    subprocess.run([str(directory / "console"), "--test-threads=1"], check=True, timeout=10)
