pub mod uacpi_kernel;

pub fn get_pcie_ecam_bases() -> Vec<(u16, PAddr)> {
    let mcfg_ptr: *const uacpi_raw::uacpi_table;
    unsafe {
        uacpi_raw::uacpi_table_find(
            b"MCFG\0".as_ptr() as *const uacpi_raw::uacpi_char,
            &mut mcfg_ptr,
        );
    }
}
