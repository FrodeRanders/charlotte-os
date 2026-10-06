//! # Memory Management Subsystem

pub mod allocators;
pub(crate) mod backing_budget;
pub(crate) mod backing_release_tests;
pub mod budget;
pub mod linear;
pub mod object;
pub(crate) mod operation;
pub mod physical;
pub(crate) mod preparation;
pub(crate) mod retirement;
pub(crate) mod thread_stack;
pub(crate) mod translation;
pub mod usage;

pub use linear::VAddr;
pub use physical::{
    MemoryInterface,
    PAddr,
    PhysicalFrameAllocator,
};
pub(crate) use preparation::{
    BackingPreparationError,
    PreparingUserBacking,
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

/// Kernel-authenticated policy identity assigned by the trusted loader from
/// signed artifact metadata. IPC snapshots this record when a message is
/// enqueued, so receivers never trust an ASID or principal supplied in the
/// request body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainAuthority {
    pub address_space: AddressSpaceHandle,
    pub principal: u64,
    pub roles: u32,
    /// Exact deployment policy admitted for this occupancy, never caller-set.
    pub launch_descriptor_digest: Option<[u8; 32]>,
}

static DOMAIN_AUTHORITIES: LazyLock<
    Mutex<alloc::collections::BTreeMap<AddressSpaceId, DomainAuthority>>,
> = LazyLock::new(|| Mutex::new(alloc::collections::BTreeMap::new()));

/// Resource limits inherited by every thread in one userspace domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainLimits {
    pub user_stack_pages: usize,
    /// Maximum active threads, including the bootstrap thread.
    pub max_threads: usize,
}

impl Default for DomainLimits {
    fn default() -> Self {
        Self {
            user_stack_pages: charlotte_launch::DEFAULT_USER_STACK_PAGES,
            max_threads: charlotte_launch::DEFAULT_USER_MAX_THREADS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainLimitError {
    StaleAddressSpace,
    InvalidUserStackPages,
    InvalidMaxThreads,
}

/// Limits are keyed by a generation-bearing handle so ASID reuse cannot
/// accidentally transfer one application's policy to its successor.
static DOMAIN_LIMITS: LazyLock<
    Mutex<alloc::collections::BTreeMap<AddressSpaceId, (AddressSpaceHandle, DomainLimits)>>,
> = LazyLock::new(|| Mutex::new(alloc::collections::BTreeMap::new()));

impl AddressSpaceHandle {
    pub const fn id(self) -> AddressSpaceId {
        self.id
    }

    pub const fn generation(self) -> usize {
        self.generation
    }
}

/// Install authority metadata for one exact address-space lifetime.
///
/// This is called only by the signed ELF loader before the domain starts.
#[cfg_attr(not(target_arch = "aarch64"), allow(dead_code))]
pub(crate) fn register_domain_authority(
    address_space: AddressSpaceHandle,
    principal: u64,
    roles: u32,
) {
    assert_ne!(principal, 0, "domain principal zero is reserved");
    assert!(
        address_space_handle_is_current(address_space),
        "cannot authorize a stale address-space handle"
    );
    let previous = DOMAIN_AUTHORITIES.lock().insert(
        address_space.id(),
        DomainAuthority {
            address_space,
            principal,
            roles,
            launch_descriptor_digest: None,
        },
    );
    assert!(previous.is_none(), "domain authority installed twice for one ASID");
}

/// Resolve the current supervisor-assigned authority for `asid`.
pub fn domain_authority(asid: AddressSpaceId) -> Option<DomainAuthority> {
    let authority = {
        let authorities = DOMAIN_AUTHORITIES.lock();
        authorities.get(&asid).copied()?
    };
    address_space_handle_is_current(authority.address_space).then_some(authority)
}

pub(crate) fn install_launch_descriptor(address_space: AddressSpaceHandle, bytes: &[u8]) {
    assert!(address_space_handle_is_current(address_space));
    let mut authorities = DOMAIN_AUTHORITIES.lock();
    let authority = authorities.get_mut(&address_space.id()).expect("launch authority missing");
    assert_eq!(authority.address_space, address_space);
    assert!(authority.launch_descriptor_digest.is_none());
    authority.launch_descriptor_digest = Some(charlotte_launch::sha256::digest(bytes));
}

/// Compare immutable admitted policy against one exact live occupancy. The
/// syscall additionally restricts use of this attestation to the controller.
pub(crate) fn launch_descriptor_matches(
    asid: AddressSpaceId,
    generation: u64,
    digest: &[u8; 32],
) -> bool {
    domain_authority(asid).is_some_and(|authority| {
        authority.address_space.generation() as u64 == generation
            && authority.launch_descriptor_digest.as_ref() == Some(digest)
    })
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
    RetirementMetadataAllocationFailed,
    QuiescenceFailed,
    OperationsInFlight,
    CloseInProgress,
    OperationDrainTimedOut,
    IpcCleanupFailed,
    DeviceCleanupFailed,
    MemoryCleanupFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddressSpaceRegistrationError {
    RootAllocationFailed,
    HardwareAsidExhausted,
    CapabilityNamespaceAllocationFailed,
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
    let namespace = crate::capability::prepare_namespace()
        .map_err(|_| AddressSpaceRegistrationError::CapabilityNamespaceAllocationFailed)?;
    prepare_user_address_space(&mut address_space)?;
    let mut table = ADDRESS_SPACE_TABLE.lock();
    let id = table.add_element(address_space);
    debug_assert_ne!(id, KERNEL_ASID);
    let generation = table.generation(id).expect("new address space missing generation");
    let handle = AddressSpaceHandle {
        id,
        generation,
    };
    namespace.publish(handle);
    let previous = DOMAIN_LIMITS.lock().insert(id, (handle, DomainLimits::default()));
    debug_assert!(previous.is_none(), "domain limits survived ASID teardown");
    usage::register_domain(handle);
    Ok(handle)
}

/// Replace the launch limits for one not-yet-running userspace domain.
pub fn set_domain_limits(
    handle: AddressSpaceHandle,
    limits: DomainLimits,
) -> Result<(), DomainLimitError> {
    if !(1..=charlotte_launch::MAX_USER_STACK_PAGES).contains(&limits.user_stack_pages) {
        return Err(DomainLimitError::InvalidUserStackPages);
    }
    if !(1..=charlotte_launch::MAX_USER_THREADS).contains(&limits.max_threads) {
        return Err(DomainLimitError::InvalidMaxThreads);
    }
    if !address_space_handle_is_current(handle) {
        return Err(DomainLimitError::StaleAddressSpace);
    }
    let mut configured = DOMAIN_LIMITS.lock();
    let Some((current, stored)) = configured.get_mut(&handle.id()) else {
        return Err(DomainLimitError::StaleAddressSpace);
    };
    if *current != handle {
        return Err(DomainLimitError::StaleAddressSpace);
    }
    *stored = limits;
    Ok(())
}

/// Resolve the stack limit inherited by a new thread in `asid`.
pub fn domain_limits(asid: AddressSpaceId) -> DomainLimits {
    DOMAIN_LIMITS.lock().get(&asid).map(|(_, limits)| *limits).unwrap_or_default()
}

/// Commit one zeroed page on demand inside the fixed user heap window.
///
/// The loader reserves the heap's virtual range but does not back it with
/// frames; the first touch of each page faults, commits a frame owned by the
/// address space (so teardown releases it with the other owned frames), and
/// returns to retry. Returns `false` outside the window or when a frame cannot
/// be obtained or mapped, leaving the fatal fault path to retire the domain.
pub(crate) fn commit_user_heap_page(asid: AddressSpaceId, fault_addr: usize) -> bool {
    let Some(handle) = current_address_space_handle(asid) else {
        return false;
    };
    commit_user_heap_page_handle(handle, fault_addr)
}

/// Uncharged provisional frame for translation-root preparation and foreign
/// backing fixtures. Stack preparation may use it only inside an owning
/// stack admission transaction. Heap/image preparation uses PreparingUserBacking.
pub(crate) struct PreparingUserFrame(Option<PAddr>);

impl Drop for PreparingUserFrame {
    fn drop(&mut self) {
        if let Some(frame) = self.0.take() {
            let released = PHYSICAL_FRAME_ALLOCATOR.lock().deallocate_frame(frame);
            if let Err(error) = released {
                crate::logln!(
                    "[frame preparation] uncharged release rejected frame={:#x}: {:?}",
                    usize::from(frame),
                    error
                );
            }
        }
    }
}

impl PreparingUserFrame {
    pub(crate) fn allocate_zeroed() -> Option<Self> {
        let preparation = Self::allocate()?;
        preparation.zero();
        Some(preparation)
    }

    fn allocate() -> Option<Self> {
        Self::allocate_with_policy(|free, usable| {
            charlotte_lifecycle::resources::frames_available(free, usable, 1)
        })
    }

    fn allocate_with_policy(allow: impl FnOnce(u64, u64) -> bool) -> Option<Self> {
        let page_size = crate::cpu::isa::memory::paging::PAGE_SIZE;
        let mut allocator = PHYSICAL_FRAME_ALLOCATOR.lock();
        if !allow(allocator.free_frames() as u64, allocator.usable_bytes() / page_size as u64) {
            return None;
        }
        let frame = allocator.allocate_frame().ok()?;
        drop(allocator);
        Some(Self(Some(frame)))
    }

    fn zero(&self) {
        let page_ptr: *mut u8 = self.frame().into();
        unsafe {
            core::ptr::write_bytes(page_ptr, 0, crate::cpu::isa::memory::paging::PAGE_SIZE);
        }
    }

    pub(crate) fn frame(&self) -> PAddr {
        self.0.unwrap()
    }

    /// Caller has prepared one tracking slot before allocating the frame.
    fn install(&mut self, address_space: &mut AddressSpace) {
        address_space.register_user_frame(self.frame());
        self.0 = None;
    }

    fn quarantine(mut self) {
        self.0 = None;
    }

    /// Consume before invoking the allocator: rejected or interrupted release
    /// must not trigger a second deallocation through this owner's Drop.
    fn release_with(
        mut self,
        deallocate: impl FnOnce(PAddr) -> Result<(), physical::Error>,
    ) -> Result<(), physical::Error> {
        deallocate(self.0.take().unwrap())
    }
}

pub(crate) fn commit_user_heap_page_handle(handle: AddressSpaceHandle, fault_addr: usize) -> bool {
    commit_user_heap_page_with_mapper(handle, fault_addr, |address_space, mapping| {
        address_space.map_existing_page(mapping).is_ok()
    })
}

// Kernel-only adapter for testing failure before leaf publication without
// exhausting the physical allocator. Production always uses the real mapper.
pub(crate) fn commit_user_heap_page_with_mapper(
    handle: AddressSpaceHandle,
    fault_addr: usize,
    map: impl FnOnce(&mut AddressSpace, linear::MemoryMapping) -> bool,
) -> bool {
    if handle.id() == KERNEL_ASID {
        return false;
    }
    let mut table = ADDRESS_SPACE_TABLE.lock();
    if table.generation(handle.id()).ok() != Some(handle.generation()) {
        return false;
    }
    let asid = handle.id();
    let page_size = crate::cpu::isa::memory::paging::PAGE_SIZE;
    let start = charlotte_launch::HEAP_VADDR;
    let capacity = usage::domain_heap_capacity(asid).unwrap_or(charlotte_launch::HEAP_SIZE);
    let end = start + capacity;
    if !(start..end).contains(&fault_addr) {
        return false;
    }
    let page = fault_addr & !(page_size - 1);
    let address_space = table.get_mut(asid).unwrap();
    if !address_space.heap_account.accepting() {
        return false;
    }
    // Concurrent first touches must not allocate a replacement or double-charge.
    match address_space.is_mapped(VAddr::from(page)) {
        Ok(true) => return true,
        Ok(false) => {}
        Err(_) => return false,
    }
    let Ok(preparation) = PreparingUserBacking::new(address_space, backing_budget::Kind::Heap)
    else {
        return false;
    };
    if preparation.map_with(VAddr::from(page), linear::PageType::UserData, map).is_err() {
        return false;
    }
    usage::note_owned_frame(asid);
    drop(table);
    crate::cpu::isa::memory::tlb::inval_range_user(asid, VAddr::from(page), 1);
    true
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
    let closing = retirement::ClosingAddressSpace::begin_ready(handle)?;
    match closing.prepare_retirement()? {
        retirement::RetirementProgress::Pending(_) => {
            // Preflight was ready, but a peer cleanup may have leased this root
            // after the fence. Abandonment retains that fence and exact root.
            Err(AddressSpaceCloseError::OperationsInFlight)
        }
        retirement::RetirementProgress::Ready(retired) => retired.release(),
    }
}

// Caller retains lifecycle and has drained existing operations. This phase
// stops resource sponsorship before owned device/IPC cleanup starts.
fn begin_user_address_space_cleanup(handle: AddressSpaceHandle) {
    let asid = handle.id();
    ADDRESS_SPACE_TABLE.lock().get_mut(asid).unwrap().heap_account.retire();
    ADDRESS_SPACE_TABLE.lock().get_mut(asid).unwrap().image_account.retire();
    ADDRESS_SPACE_TABLE.lock().get_mut(asid).unwrap().table_account.retire();
    crate::capability::retire_address_space(asid);
    budget::retire(handle);
}

// Caller retains lifecycle, has drained cleanup leases and sealed admission.
// IPC loan cleanup and namespace removal have already completed outside locks.
fn finish_user_address_space_cleanup(
    handle: AddressSpaceHandle,
    closing: crate::klib::collections::id_table::ClosingSlot,
) -> Result<retirement::RetiredAddressSpace, AddressSpaceCloseError> {
    let asid = handle.id();

    object::close_scratch_address_space(asid);
    crate::completion::close_address_space(asid);
    crate::syscall::close_mailbox_address_space(asid);
    crate::capability::close_address_space(asid);

    // Retain this generation's stack high-water mark and heap peak for the
    // service principal before the authority and accounting entries
    // disappear. The launch path uses them to size the principal's next
    // generation.
    if let Some(principal) =
        DOMAIN_AUTHORITIES.lock().get(&asid).map(|authority| authority.principal)
    {
        if let Some(usage) = usage::domain_usage(asid) {
            usage::remember_principal_stack_high_water(
                principal,
                usage.stack_pages_used_high_water,
            );
        }
        if let Some(status) = usage::domain_heap_status(asid) {
            usage::remember_principal_heap_peak(principal, status.peak_bytes);
        }
    }

    let removed_authority = DOMAIN_AUTHORITIES.lock().remove(&asid);
    if let Some(authority) = removed_authority {
        debug_assert_eq!(authority.address_space, handle);
    }

    DOMAIN_LIMITS.lock().remove(&asid);
    usage::unregister_domain(asid);

    let entry = {
        let mut table = ADDRESS_SPACE_TABLE.lock();
        table
            .retire_closing(closing)
            .expect("preflighted address-space retirement lost its serialized slot")
    };
    Ok(retirement::RetiredAddressSpace::new(handle, entry))
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
