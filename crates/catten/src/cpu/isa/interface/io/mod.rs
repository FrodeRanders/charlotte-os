pub trait IReg8Ifce {
    /// # Safety
    /// The register must be valid and safe to read at the current privilege level.
    unsafe fn read(&self) -> u8;
}

pub trait IReg16Ifce {
    /// # Safety
    /// The register must be valid, aligned, and safe to read.
    unsafe fn read(&self) -> u16;
}

pub trait IReg32Ifce {
    /// # Safety
    /// The register must be valid, aligned, and safe to read.
    unsafe fn read(&self) -> u32;
}

pub trait IReg64Ifce {
    /// # Safety
    /// The register must be valid, aligned, and safe to read.
    unsafe fn read(&self) -> u64;
}

pub trait OReg8Ifce {
    /// # Safety
    /// The register must be valid and `value` must be permitted by the device.
    unsafe fn write(&self, value: u8);
}

pub trait OReg16Ifce {
    /// # Safety
    /// The register must be valid, aligned, and accept `value`.
    unsafe fn write(&self, value: u16);
}

pub trait OReg32Ifce {
    /// # Safety
    /// The register must be valid, aligned, and accept `value`.
    unsafe fn write(&self, value: u32);
}

pub trait OReg64Ifce {
    /// # Safety
    /// The register must be valid, aligned, and accept `value`.
    unsafe fn write(&self, value: u64);
}
