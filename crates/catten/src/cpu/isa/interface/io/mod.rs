pub trait IReg8Ifce {
    unsafe fn read(&self) -> u8;
}

pub trait IReg16Ifce {
    unsafe fn read(&self) -> u16;
}

pub trait IReg32Ifce {
    unsafe fn read(&self) -> u32;
}

pub trait IReg64Ifce {
    unsafe fn read(&self) -> u64;
}

pub trait OReg8Ifce {
    unsafe fn write(&self, value: u8);
}

pub trait OReg16Ifce {
    unsafe fn write(&self, value: u16);
}

pub trait OReg32Ifce {
    unsafe fn write(&self, value: u32);
}

pub trait OReg64Ifce {
    unsafe fn write(&self, value: u64);
}
