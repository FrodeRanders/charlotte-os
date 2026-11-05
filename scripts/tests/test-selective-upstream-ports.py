#!/usr/bin/env python3
"""Compile actual address-map, MCFG and bounded traversal controls for the host."""
from pathlib import Path
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]


def run():
    channel = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    rustc = subprocess.check_output(["rustup", "which", "--toolchain", channel, "rustc"], text=True).strip()
    source = r'''
#![feature(step_trait)]
#![allow(dead_code)]
extern crate self as spin;
pub use std::sync::LazyLock;
mod cpu { pub mod isa {
 pub mod interface { pub mod memory {
  #[path="@ROOT@/crates/catten/src/cpu/isa/interface/memory/address.rs"] pub mod address;
 }}
 pub mod memory { pub mod address {
  pub static VADDR_SIG_BITS:crate::LazyLock<u8>=crate::LazyLock::new(||48);
  #[path="@ROOT@/crates/catten/src/cpu/isa/common/memory/address/vaddr.rs"] pub mod vaddr;
 }}
}}
mod klib { #[path="@ROOT@/crates/catten/src/klib/size.rs"] pub mod size; }
mod memory { pub mod linear {
 pub use crate::cpu::isa::memory::address::vaddr::VAddr;
 #[path="@MAP@"] pub mod address_map;
}}
mod device_management { pub mod drivers { pub mod busses { pub mod pci_express { pub mod topology {
 #[path="@ROOT@/crates/catten/src/device_management/drivers/busses/pci_express/topology/enumeration.rs"] pub mod enumeration;
}}}}}
#[path="@ROOT@/crates/catten/src/environment/acpi/sdt/mcfg/regions.rs"] mod regions;

use std::alloc::{GlobalAlloc,Layout,System};
use core::{cell::Cell,sync::atomic::{AtomicUsize,Ordering}};
thread_local! { static TRACK:Cell<bool>=const{Cell::new(false)}; }
static ALLOCATIONS:AtomicUsize=AtomicUsize::new(0);
struct Tracer;
#[global_allocator] static ALLOCATOR:Tracer=Tracer;
unsafe impl GlobalAlloc for Tracer {
 unsafe fn alloc(&self,layout:Layout)->*mut u8 {
  if TRACK.try_with(Cell::get).unwrap_or(false) { ALLOCATIONS.fetch_add(1,Ordering::Relaxed); }
  unsafe {System.alloc(layout)}
 }
 unsafe fn dealloc(&self,pointer:*mut u8,layout:Layout) { unsafe {System.dealloc(pointer,layout)}; }
}
#[test] fn preflight_and_bus_worklist_allocate_no_heap() {
 let mut table=[0u8;60];table[..4].copy_from_slice(b"MCFG");
 table[4..8].copy_from_slice(&60u32.to_le_bytes());
 table[44..52].copy_from_slice(&0xe0000000u64.to_le_bytes());table[55]=255;
 table[9]=0u8.wrapping_sub(table.iter().fold(0u8,|sum,byte|sum.wrapping_add(*byte)));
 ALLOCATIONS.store(0,Ordering::Relaxed);TRACK.with(|flag|flag.set(true));
 let result=regions::Regions::new().append_table(&table,(1u64<<48)-1);
 use device_management::drivers::busses::pci_express::topology::enumeration::{Buses,Budget,MAX_FUNCTIONS};
 let mut work=Buses::new(0,255);let mut count=0;
 while let Some(bus)=work.next() {count+=1;if bus<255 {work.discover(bus+1)}work.discover(0);}
 let mut budget=Budget::new();for _ in 0..MAX_FUNCTIONS {budget.accept_function().unwrap();budget.accept_device().unwrap();}
 TRACK.with(|flag|flag.set(false));
 assert!(result.is_ok());assert_eq!(count,256);assert_eq!(ALLOCATIONS.load(Ordering::Relaxed),0);
}

'''
    with tempfile.TemporaryDirectory(prefix="charlotte-selective-ports-") as temporary:
        directory = Path(temporary)
        path = directory / "controls.rs"
        path.write_text(source.replace("@ROOT@", str(ROOT)).replace("@MAP@", str(ROOT / "crates/catten/src/memory/linear/address_map.rs")))
        binary = directory / "controls"
        subprocess.run([rustc, "--edition=2024", "--test", str(path), "-o", str(binary)], check=True)
        subprocess.run([str(binary)], check=True)
        # Restore the old canonicalizing containment arithmetic against the new
        # layouts. The real boundary controls must reject it after compilation.
        current = (ROOT / "crates/catten/src/memory/linear/address_map.rs").read_text()
        start = current.index("impl LinearMemoryRegion {")
        end = current.index("impl From<LinearMemoryRegion>", start)
        control = (current[:start] + "impl LinearMemoryRegion { pub fn contains(&self, addr:VAddr)->bool { addr>=self.base && addr<(self.base+self.length) } }\n" + current[end:])
        control = control.replace('"address_map/tests.rs"', f'"{ROOT}/crates/catten/src/memory/linear/address_map/tests.rs"')
        unsafe_map = directory / "unsafe-map.rs"
        unsafe_map.write_text(control)
        path.write_text(source.replace("@ROOT@", str(ROOT)).replace("@MAP@", str(unsafe_map)))
        subprocess.run([rustc, "--edition=2024", "--test", str(path), "-o", str(binary)], check=True)
        rejected = subprocess.run([str(binary), "--exact", "memory::linear::address_map::tests::every_software_region_has_exact_boundaries_in_all_layouts", "--nocapture"], capture_output=True, text=True)
        output = rejected.stdout + rejected.stderr
        assert rejected.returncode != 0 and ("assertion failed" in output or "attempt to add with overflow" in output), output
        print("Selective upstream controls: old canonicalizing containment rejected.")


if __name__ == "__main__":
    run()
