#![no_std]

pub const CONFIG_VADDR: usize = 0x0000_0000_0001_0000;
pub const CONFIG_PAGE_SIZE: u32 = 4096;
pub const CQ_VADDR: usize = 0x0000_0000_0001_1000;
pub const CQ_ENTRIES: u32 = 32;
pub const INPUT_VADDR: usize = 0x0000_0000_0001_2000;
pub const INPUT_CAPACITY: usize = 4096;
pub const HEAP_VADDR: usize = 0x0000_0000_0001_3000;
pub const HEAP_SIZE: usize = 0xd000;
/// Mutable program status/output page, deliberately separate from launch
/// configuration so applications cannot overwrite their launch contract.
// Kept below the per-shard CQ reservation at 0x0080_0000 and well above the
// linked application image, which begins at 0x0002_0000.
pub const STATUS_VADDR: usize = 0x0000_0000_007f_0000;
pub const STATUS_PAGE_SIZE: u32 = 4096;

pub const LAUNCH_HEADER_OFFSET: usize = 2112;
pub const CAPABILITY_VECTOR_OFFSET: usize = 2224;
pub const CAPABILITY_VECTOR_CAPACITY: usize = 32;
pub const LAUNCH_MAGIC: u64 = 0x4348_4152_4c4f_5454; // "CHARLOTT"
pub const LAUNCH_ABI_MAJOR: u16 = 1;
pub const LAUNCH_ABI_MINOR: u16 = 2;

pub const ARGC_OFFSET: usize = 24;
pub const ARGS_OFFSET: usize = 32;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LaunchHeader {
    pub magic: u64,
    pub abi_major: u16,
    pub abi_minor: u16,
    pub header_size: u16,
    pub reserved: u16,
    pub config_size: u32,
    pub flags: u32,
    pub args_offset: u32,
    pub args_count: u32,
    pub capabilities_offset: u32,
    pub capabilities_count: u32,
    pub heap_base: u64,
    pub heap_size: u64,
    pub input_base: u64,
    pub input_size: u32,
    pub cq_entries: u32,
    pub cq_base: u64,
    pub status_base: u64,
    pub status_size: u32,
    pub reserved2: u32,
}

impl LaunchHeader {
    pub const fn new() -> Self {
        Self {
            magic: LAUNCH_MAGIC,
            abi_major: LAUNCH_ABI_MAJOR,
            abi_minor: LAUNCH_ABI_MINOR,
            header_size: core::mem::size_of::<Self>() as u16,
            reserved: 0,
            config_size: CONFIG_PAGE_SIZE,
            flags: 0,
            args_offset: ARGS_OFFSET as u32,
            args_count: 0,
            capabilities_offset: CAPABILITY_VECTOR_OFFSET as u32,
            capabilities_count: 0,
            heap_base: HEAP_VADDR as u64,
            heap_size: HEAP_SIZE as u64,
            input_base: INPUT_VADDR as u64,
            input_size: INPUT_CAPACITY as u32,
            cq_entries: CQ_ENTRIES,
            cq_base: CQ_VADDR as u64,
            status_base: STATUS_VADDR as u64,
            status_size: STATUS_PAGE_SIZE,
            reserved2: 0,
        }
    }

    pub const fn is_compatible(&self) -> bool {
        let args_end = self.args_offset as usize + self.args_count as usize * 4;
        let capabilities_end = self.capabilities_offset as usize
            + self.capabilities_count as usize * core::mem::size_of::<CapabilityRecord>();
        self.magic == LAUNCH_MAGIC
            && self.abi_major == LAUNCH_ABI_MAJOR
            && self.abi_minor >= LAUNCH_ABI_MINOR
            && self.header_size as usize >= core::mem::size_of::<Self>()
            && self.config_size == CONFIG_PAGE_SIZE
            && self.args_offset as usize >= ARGS_OFFSET
            && self.capabilities_offset as usize >= CAPABILITY_VECTOR_OFFSET
            && self.capabilities_count as usize <= CAPABILITY_VECTOR_CAPACITY
            && args_end <= CONFIG_PAGE_SIZE as usize
            && capabilities_end <= CONFIG_PAGE_SIZE as usize
            && self.heap_base != 0
            && self.heap_size != 0
            && self.input_size as usize <= INPUT_CAPACITY
            && self.cq_entries != 0
            && self.status_base != 0
            && self.status_size != 0
            && self.status_size <= STATUS_PAGE_SIZE
    }
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityKind {
    Bootstrap = 1,
    Mmio = 2,
    Interrupt = 3,
    HandoffState = 4,
    HandoffEndpoint = 5,
}

impl CapabilityKind {
    pub const fn from_raw(raw: u16) -> Option<Self> {
        match raw {
            1 => Some(Self::Bootstrap),
            2 => Some(Self::Mmio),
            3 => Some(Self::Interrupt),
            4 => Some(Self::HandoffState),
            5 => Some(Self::HandoffEndpoint),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CapabilityRecord {
    pub kind: u16,
    pub rights: u16,
    pub flags: u32,
    pub handle: u64,
}

const _: [(); 96] = [(); core::mem::size_of::<LaunchHeader>()];
const _: [(); 16] = [(); core::mem::size_of::<CapabilityRecord>()];
