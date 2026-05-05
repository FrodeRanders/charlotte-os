use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ptr::NonNull;

use hashbrown::HashMap;
use spin::Lazy;

use crate::cpu::isa::interface::memory::address::Address;
use crate::environment::boot_protocol::limine::RSDP_REQUEST;
use crate::memory::PAddr;
use crate::memory::physical::PhysicalAddress;
use crate::println;

pub mod aml;
pub mod static_data;

static TABLE_MAP: Lazy<HashMap<AcpiTableType, Vec<PAddr>>> = Lazy::new(|| {
    if let Some(xsdp_ptr) = get_xsdp() {
        let xsdp: &Xsdp = unsafe { xsdp_ptr.as_ref() };
        if !xsdp.validate() {
            panic!("Invalid ACPI Extended System Description Pointer (XSDP)");
        }
        let xsdt_addr = PAddr::from(xsdp.xsdt_address);
        if xsdt_addr.is_null() {
            panic!("ACPI Extended System Description Table (XSDT) required but not found");
        }
        parse_xsdt(xsdt_addr)
    } else {
        panic!("ACPI not available");
    }
});

pub fn print_table_map() {
    let mut output = String::new();
    output.push_str("ACPI Table Map:\n");
    for (table_type, addrs) in TABLE_MAP.iter() {
        output.push_str(&format!("{:?}: {:?}\n", table_type, addrs));
    }
    println!("{output}");
}

#[derive(Debug)]
pub enum Error {
    AcpiUnavailable,
    InvalidXsdp,
    XsdtNotFound,
    TableNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AcpiTableType {
    RSDT,
    XSDT,
    FADT,
    MADT,
    SRAT,
    MCFG,
    HPET,
    WAET,
    BGRT,
    DSDT,
    SSDT,
}

impl AcpiTableType {
    fn from_signature(signature: [u8; 4]) -> Option<Self> {
        match signature {
            [b'R', b'S', b'D', b'T'] => Some(Self::RSDT),
            [b'X', b'S', b'D', b'T'] => Some(Self::XSDT),
            [b'F', b'A', b'C', b'P'] => Some(Self::FADT),
            [b'A', b'P', b'I', b'C'] => Some(Self::MADT),
            [b'S', b'R', b'A', b'T'] => Some(Self::SRAT),
            [b'M', b'C', b'F', b'G'] => Some(Self::MCFG),
            [b'H', b'P', b'E', b'T'] => Some(Self::HPET),
            [b'W', b'A', b'E', b'T'] => Some(Self::WAET),
            [b'B', b'G', b'R', b'T'] => Some(Self::BGRT),
            [b'D', b'S', b'D', b'T'] => Some(Self::DSDT),
            [b'S', b'S', b'D', b'T'] => Some(Self::SSDT),
            _ => None,
        }
    }
}

#[repr(C, packed)]
#[derive(Debug)]
pub struct Xsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32, // deprecated since version 2.0

    length: u32,
    xsdt_address: u64,
    extended_checksum: u8,
    reserved: [u8; 3],
}

impl Xsdp {
    fn validate(&self) -> bool {
        let mut sum = 0u8;
        unsafe {
            let ptr = &raw const *self as *const u8;
            for i in 0..self.length as usize {
                sum = sum.wrapping_add(*ptr.add(i));
            }
        }
        sum == 0
    }
}

#[repr(C, packed)]
#[derive(Debug)]
pub struct SdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

pub fn get_xsdp() -> Option<NonNull<Xsdp>> {
    if let Some(res) = RSDP_REQUEST.response() {
        NonNull::new(res.address as *mut Xsdp)
    } else {
        None
    }
}

pub fn is_acpi_available() -> bool {
    get_xsdp().is_some()
}

pub fn find_table_type(table: AcpiTableType) -> Result<Vec<PAddr>, Error> {
    if let Some(table_addrs) = TABLE_MAP.get(&table) {
        Ok(table_addrs.clone())
    } else {
        Err(Error::TableNotFound)
    }
}

fn parse_xsdt(xsdt_addr: PAddr) -> HashMap<AcpiTableType, Vec<PAddr>> {
    let mut tables = HashMap::new();
    println!(
        "[ACPI] Parsing XSDT at address {:?} with header size {}",
        xsdt_addr,
        size_of::<SdtHeader>()
    );
    let xsdt_header: &SdtHeader =
        unsafe { (xsdt_addr.into_hhdm_ptr::<SdtHeader>()).as_ref().unwrap() };
    println!("[ACPI] XSDT header located.");
    // Note: HHDM pointers are extremely unsafe as they live entirely outside of Rust's memory model
    // Try to keep their use read-only and never use the HHDM to access data that can be accessed
    // through proper memory mappings. ACPI tables and Device Tree Nodes are an acceptable use case
    // because they are only ever read from.
    let xsdt_data_ptr = unsafe {
        NonNull::new_unchecked((xsdt_addr + size_of::<SdtHeader>()).into_hhdm_mut::<u64>())
    };
    let data_length = xsdt_header.length as usize - size_of::<SdtHeader>();
    let num_entries = data_length / size_of::<u64>();
    println!(
        "[ACPI] XSDT data physical address: {:?}, number of entries: {}",
        xsdt_data_ptr, num_entries
    );
    let mut table_addrs = Vec::<PAddr>::with_capacity(num_entries);
    for i in 0..num_entries {
        let entry_addr = unsafe { xsdt_data_ptr.as_ptr().add(i).read_unaligned() };
        table_addrs.push(PAddr::from(entry_addr));
    }

    for table_addr in &table_addrs {
        if let Some(signature) = get_table_signature(*table_addr) {
            if let Some(table_type) = AcpiTableType::from_signature(signature) {
                tables.entry(table_type).or_insert_with(Vec::new).push(*table_addr);
            } else {
                println!(
                    "[ACPI] Warning: Unrecognized ACPI table with signature {:?} at address {:?}",
                    signature, table_addr
                );
            }
        }
    }
    println!("[ACPI] Finished parsing XSDT. Found {} tables.", tables.len());
    tables
}

fn get_table_signature(table_addr: PAddr) -> Option<[u8; 4]> {
    if table_addr.is_null() {
        return None;
    } else {
        let header = unsafe { (table_addr.into_hhdm_ptr::<SdtHeader>()).read() };
        Some(header.signature)
    }
}
