//! # Memory Management Subsystem

pub mod allocators;
pub mod linear;
pub mod physical;

pub use linear::VAddr;
pub use physical::{MemoryInterface, PAddr, PhysicalFrameAllocator};
pub use spin::{LazyLock, Mutex, RwLock};

pub use crate::cpu::isa::interface::memory::AddressSpaceInterface;
pub use crate::cpu::isa::memory::paging::AddressSpace;
use crate::environment::boot_protocol::limine::{HHDM_REQUEST, MEMORY_MAP_REQUEST};
pub use crate::klib::collections::id_table::IdTable;

pub type AddressSpaceId = usize;

/*The kernel address space is always ASID 0 and it is handled differently from userspace address
 * spaces because it needs to be initialized and accessible before the kernel allocator is
 * constructed and initialized.
 */
/// The kernel address space ID.
pub const KERNEL_ASID: AddressSpaceId = 0;
/// The kernel address space. It is initialized to the current address space when this static is
/// first accessed. Which should happen during the BSP init process.
pub static KERNEL_AS: LazyLock<Mutex<AddressSpace>> =
    LazyLock::new(|| Mutex::new(AddressSpace::get_current()));
/// Holds all address spaces, indexed by their kernel assigned AddressSpaceId.
///
/// Index 0 ([`KERNEL_ASID`]) is reserved for the kernel address space and is
/// pre-populated on first access, so user address spaces are always assigned
/// non-zero ids. This is essential: `Thread::new` treats `asid == KERNEL_ASID`
/// as a kernel thread (runs at EL1/ring 0), so a user AS must never be given
/// id 0.
type AddressSpaceTable = IdTable<AddressSpace>;
pub static ADDRESS_SPACE_TABLE: LazyLock<Mutex<AddressSpaceTable>> = LazyLock::new(|| {
    let mut table = AddressSpaceTable::new();
    // Reserve id 0 for the kernel address space.
    let kernel_id = table.add_element(AddressSpace::get_current());
    debug_assert_eq!(kernel_id, KERNEL_ASID, "kernel AS must occupy id 0");
    Mutex::new(table)
});
/// The starting virtual address of the higher half direct mapping region created by the bootloader.
/// This should be remapped by the VMM during BSP init to be placed at the address specified by the
/// kernel virtual memory map at which point this address should be updated to reflect the new
/// location.
pub static HHDM_BASE: LazyLock<VAddr> = LazyLock::new(|| {
    let offset = HHDM_REQUEST
        .response()
        .expect("Limine failed to provide a higher half direct mapping region.")
        .offset as usize;
    // The HHDM offset is already a valid, bootloader-chosen higher-half virtual
    // address and must be stored verbatim. It must NOT go through
    // `VAddr::from`, whose x86-style canonical sign-extension (treating bit 47
    // as the sign bit) zeroes AArch64's TTBR1 base of 0xffff_0000_0000_0000,
    // because that address has bit 47 clear.
    unsafe { VAddr::from_raw_unchecked(offset) }
});
/// The physical frame allocator instance used by the kernel.
pub static PHYSICAL_FRAME_ALLOCATOR: LazyLock<Mutex<PhysicalFrameAllocator>> =
    LazyLock::new(|| {
        Mutex::new(PhysicalFrameAllocator::from(
            MEMORY_MAP_REQUEST.response().expect("Limine failed to provide a memory map."),
        ))
    });
