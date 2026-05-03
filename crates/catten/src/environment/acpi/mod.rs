use alloc::vec::Vec;
use core::ptr::NonNull;

use hashbrown::HashMap;

use crate::environment::boot_protocol::limine::RSDP_REQUEST;
use crate::memory::PAddr;
use crate::memory::physical::PhysicalAddress;

pub mod aml;
pub mod static_data;

pub enum Error {
    AcpiUnavailable,
    InvalidXsdp,
    XsdtNotFound,
}

#[derive(Debug)]
pub enum AcpiTableType {
    RSDT,
    XSDT,
    FADT,
    MADT,
    SRAT,
    MCFG,
    DSDT,
    SSDT,
}

#[repr(C)]
#[derive(Debug)]
pub struct Xsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [char; 6],
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
                sum += *ptr.add(i);
            }
        }
        sum == 0
    }
}

#[repr(C)]
#[derive(Debug)]
pub struct SdtHeader {
    signature: [char; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [char; 6],
    oem_table_id: [char; 8],
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

pub fn find_table(table: AcpiTableType) -> Result<Vec<PAddr>, Error> {
    if let Some(xsdp_ptr) = get_xsdp() {
        let xsdp: &Xsdp = unsafe { xsdp_ptr.as_ref() };
        if !xsdp.validate() {
            return Err(Error::InvalidXsdp);
        }
        let xsdt_addr: PAddr;
        // try the xsdt_address first
        if xsdp.xsdt_address == 0 {
            panic!("ACPI Extended System Description Table (XSDT) required but not found");
        } else {
            xsdt_addr = PAddr::from(xsdp.xsdt_address);
        }
        let tables = parse_xsdt(xsdt_addr);
        todo!(
            "Find all instances of the requested table type in the table HashMap and return their \
             addresses"
        );
    } else {
        Err(Error::AcpiUnavailable)
    }
}

fn parse_xsdt(xsdt_addr: PAddr) -> HashMap<AcpiTableType, Vec<PAddr>> {
    let mut tables = HashMap::new();

    // Note: HHDM pointers are extremely unsafe as they live entirely outside of Rust's memory model
    // Try to keep their use read-only and never use the HHDM to access data that can be accessed
    // through proper memory mappings. ACPI tables and Device Tree Nodes are an acceptable use case
    // because they are only ever read from.
    let xsdt_data_ptr = unsafe {
        NonNull::new_unchecked((xsdt_addr + size_of::<SdtHeader>()).into_hhdm_mut::<u64>())
    };

    todo!(
        "Parse the XSDT and populate the tables HashMap with the addresses of all available ACPI \
         tables"
    );

    tables
}
