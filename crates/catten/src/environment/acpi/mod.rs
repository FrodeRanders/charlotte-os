use alloc::vec::Vec;

use crate::environment::boot_protocol::limine::RSDP_REQUEST;
use crate::memory::PAddr;

pub mod aml;
pub mod static_data;

pub enum Error {
    AcpiUnavailable,
}

#[derive(Debug)]
pub enum AcpiTable {
    RSDT,
    XSDT,
    FADT,
    MADT,
    SRAT,
    MCFG,
    DSDT,
    SSDT,
}

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

pub struct Xsdt {}

pub fn get_xsdp() -> Option<*const Xsdp> {
    if let Some(res) = RSDP_REQUEST.response() {
        Some(res.address as *const Xsdp)
    } else {
        None
    }
}

pub fn is_acpi_available() -> bool {
    get_xsdp().is_some()
}

pub fn find_table(table: AcpiTable) -> Result<Vec<PAddr>, Error> {
    if let Some(xsdp_ptr) = get_xsdp() {
        todo!(
            "Parse the XSDP and use it to find the XSDT and then use the pointers in the XSDT to \
             find all instances of the specified table type."
        )
    } else {
        Err(Error::AcpiUnavailable)
    }
}
