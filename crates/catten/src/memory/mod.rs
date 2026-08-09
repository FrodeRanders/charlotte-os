//! # Memory Management Subsystem

pub mod allocators;
pub mod linear;
pub mod object;
pub mod physical;

pub use linear::VAddr;
pub use physical::{
    MemoryInterface,
    PAddr,
    PhysicalFrameAllocator,
};
pub use spin::{
    LazyLock,
    RwLock,
};

// Memory-global locks are acquired from both preemptible kernel threads and
// synchronous EL0 exception paths. A plain spin::Mutex permits its owner to be
// timer-preempted; if every LP then enters a synchronous exception and spins
// for that lock with IRQs masked, the owner can never be scheduled again.
// Mask local interrupts for the complete ownership interval instead.
pub use crate::cpu::multiprocessor::spin::mutex::Mutex;
use crate::environment::boot_protocol::limine::{
    HHDM_REQUEST,
    MEMORY_MAP_REQUEST,
};
pub use crate::{
    cpu::isa::{
        interface::memory::AddressSpaceInterface,
        memory::paging::AddressSpace,
    },
    klib::collections::id_table::IdTable,
};

pub type AddressSpaceId = usize;

/// Stable identity for one occupancy of an address-space table slot.
///
/// The numeric ASID is intentionally reusable. Long-lived authorities and
/// lifecycle operations must retain this handle so a delayed operation for a
/// dead domain cannot act on a replacement that inherited the same ASID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressSpaceHandle {
    id: AddressSpaceId,
    generation: usize,
}

impl AddressSpaceHandle {
    pub const fn id(self) -> AddressSpaceId {
        self.id
    }

    pub const fn generation(self) -> usize {
        self.generation
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpaceCloseError {
    KernelAddressSpace,
    AddressSpaceMissing,
    StaleHandle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpaceRegistrationError {
    HardwareAsidExhausted,
    /// The image failed cluster signature verification (unsigned or
    /// invalidly signed); loading it is refused.
    SignatureVerificationFailed,
}

/// Serializes allocation and teardown across resource cleanup. This prevents
/// an ASID slot from being reused while cleanup keyed by its numeric id is in
/// progress.
pub(crate) static ADDRESS_SPACE_LIFECYCLE: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[cfg(target_arch = "aarch64")]
fn prepare_user_address_space(
    address_space: &mut AddressSpace,
) -> Result<(), AddressSpaceRegistrationError> {
    address_space.ensure_hw_asid().ok_or(AddressSpaceRegistrationError::HardwareAsidExhausted)?;
    Ok(())
}

#[cfg(not(target_arch = "aarch64"))]
fn prepare_user_address_space(
    _address_space: &mut AddressSpace,
) -> Result<(), AddressSpaceRegistrationError> {
    Ok(())
}

/// Add an address space and return the generation-bearing identity of this
/// particular slot occupancy.
pub fn register_user_address_space(
    mut address_space: AddressSpace,
) -> Result<AddressSpaceHandle, AddressSpaceRegistrationError> {
    let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
    prepare_user_address_space(&mut address_space)?;
    let mut table = ADDRESS_SPACE_TABLE.lock();
    let id = table.add_element(address_space);
    debug_assert_ne!(id, KERNEL_ASID);
    let generation = table.generation(id).expect("new address space missing generation");
    Ok(AddressSpaceHandle {
        id,
        generation,
    })
}

/// Return the identity currently occupying `asid`.
pub fn current_address_space_handle(asid: AddressSpaceId) -> Option<AddressSpaceHandle> {
    let table = ADDRESS_SPACE_TABLE.lock();
    table.generation(asid).ok().map(|generation| AddressSpaceHandle {
        id: asid,
        generation,
    })
}

/// Whether `handle` still denotes the active occupant of its ASID slot.
pub fn address_space_handle_is_current(handle: AddressSpaceHandle) -> bool {
    ADDRESS_SPACE_TABLE.lock().generation(handle.id).ok() == Some(handle.generation)
}

/// Close one exact address-space lifetime, rejecting a handle left behind by
/// ASID reuse.
pub fn close_user_address_space_handle(
    handle: AddressSpaceHandle,
) -> Result<(), AddressSpaceCloseError> {
    let _lifecycle = ADDRESS_SPACE_LIFECYCLE.lock();
    close_user_address_space_locked(handle)
}

fn close_user_address_space_locked(
    handle: AddressSpaceHandle,
) -> Result<(), AddressSpaceCloseError> {
    let asid = handle.id;
    if asid == KERNEL_ASID {
        return Err(AddressSpaceCloseError::KernelAddressSpace);
    }

    match ADDRESS_SPACE_TABLE.lock().generation(asid) {
        Ok(generation) if generation == handle.generation => {}
        Ok(_) => return Err(AddressSpaceCloseError::StaleHandle),
        Err(_) => return Err(AddressSpaceCloseError::AddressSpaceMissing),
    }

    // DMA mappings must be revoked before memory-object teardown releases
    // their pinned frames.
    crate::device::close_address_space(asid);
    object::close_address_space(asid);
    crate::ipc::close_address_space(asid);
    crate::completion::close_address_space(asid);
    crate::syscall::close_mailbox_address_space(asid);
    crate::capability::close_address_space(asid);

    ADDRESS_SPACE_TABLE
        .lock()
        .remove_element(asid)
        .map_err(|_| AddressSpaceCloseError::AddressSpaceMissing)
}
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
