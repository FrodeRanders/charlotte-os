//! Real private tables and full stack-capacity reservations share the existing
//! principal budget. Serialized fixtures use fresh, never-dispatched roots.
use super::*;
use crate::memory::{
    self,
    ADDRESS_SPACE_TABLE,
    AddressSpace,
    AddressSpaceHandle,
    AddressSpaceInterface,
    PHYSICAL_FRAME_ALLOCATOR,
    PreparingUserFrame,
    linear::{
        MemoryMapping,
        PageType,
    },
    thread_stack::{
        PreparingStackPage,
        Stacks,
    },
};

fn root(principal: u64) -> AddressSpaceHandle {
    let root = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    memory::register_domain_authority(root, principal, 0).unwrap();
    memory::set_domain_limits(
        root,
        memory::DomainLimits {
            user_stack_pages: 4,
            max_threads: 1,
        },
    )
    .unwrap();
    root
}
fn limit(principal: u64, pages: u64) {
    HISTORY
        .lock()
        .get_mut(&principal)
        .unwrap()
        .backing
        .set_limit(Amount {
            pages,
            objects: 32,
        })
        .unwrap();
}
fn table_pages(root: AddressSpaceHandle) -> u64 {
    ADDRESS_SPACE_TABLE.lock().get(root.id()).unwrap().table_account.pages()
}
fn map(root: AddressSpaceHandle, address: usize, frame: memory::PAddr) -> bool {
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(root.id())
        .unwrap()
        .map_existing_page(MemoryMapping {
            vaddr: address.into(),
            paddr: frame,
            page_type: PageType::UserData,
        })
        .is_ok()
}
fn unmap(root: AddressSpaceHandle, address: usize, frame: memory::PAddr) {
    assert_eq!(
        ADDRESS_SPACE_TABLE.lock().get_mut(root.id()).unwrap().unmap_page(address.into()).unwrap(),
        frame
    );
    crate::cpu::isa::memory::tlb::try_inval_range_user(root.id(), address.into(), 1).unwrap();
}

pub(super) fn run() {
    initial_root_admission();
    sparse_roots();
    maximum_stack_reservation();
    crate::logln!(
        "[principal structural admission] sparse roots share table ceiling; rejected prefixes \
         stay charged and cached branches reuse; full user/kernel stack capacity shares quota \
         before allocation, growth pays once, unrelated principal progresses, confirmed \
         cancellation/refund and root teardown passed"
    );
}

fn initial_root_admission() {
    let principal = charlotte_launch::artifact_principal_id(b"principal-constructor-control");
    let sponsor = memory::prepare_principal_backing(principal, false).unwrap();
    limit(principal, 0);
    let frame = PreparingUserFrame::allocate_zeroed().unwrap();
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    let result = AddressSpace::try_new_principal_user(false, sponsor);
    #[cfg(target_arch = "x86_64")]
    assert!(result.is_err());
    #[cfg(target_arch = "aarch64")]
    {
        let mut lazy = result.unwrap();
        assert!(
            lazy.map_existing_page(MemoryMapping {
                vaddr: 0x4000_0000usize.into(),
                paddr: frame.frame(),
                page_type: PageType::UserData
            })
            .is_err()
        );
        assert_eq!(lazy.table_account.pages(), 0);
        drop(lazy);
    }
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(backing_used(principal), Amount::default());
    frame.release().unwrap();
}

fn sparse_roots() {
    let principal = charlotte_launch::artifact_principal_id(b"principal-sparse-control");
    let first = root(principal);
    let second = root(principal);
    // This foreign frame is owned by the kernel fixture. Only the private
    // hierarchy belongs to either root; the frame is explicitly released last.
    let frame = PreparingUserFrame::allocate_zeroed().unwrap();
    assert!(map(first, 0x4000_0000, frame.frame()));
    unmap(first, 0x4000_0000, frame.frame());
    let initial = backing_used(principal).pages;
    limit(principal, initial + 1);
    assert!(!map(second, 0x8000_0000, frame.frame()));
    assert_eq!(backing_used(principal).pages, initial + 1);
    let tables = table_pages(first) + table_pages(second);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..64 {
        assert!(!map(second, 0x8000_0000, frame.frame()));
        assert!(map(first, 0x4000_0000, frame.frame()));
        unmap(first, 0x4000_0000, frame.frame());
    }
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(table_pages(first) + table_pages(second), tables);
    memory::budget::mark_platform(second);
    assert!(!map(second, 0x8000_0000, frame.frame()));
    assert_eq!(backing_used(principal).pages, initial + 1);
    memory::close_user_address_space_handle(second).unwrap();
    assert_eq!(backing_used(principal).pages, table_pages(first));
    memory::close_user_address_space_handle(first).unwrap();
    assert_eq!(backing_used(principal), Amount::default());
    frame.release().unwrap();
}

fn maximum_stack_reservation() {
    let principal = charlotte_launch::artifact_principal_id(b"principal-stack-control");
    let other_principal = charlotte_launch::artifact_principal_id(b"principal-stack-other");
    let first = root(principal);
    let second = root(principal);
    let other = root(other_principal);
    // Warm each private branch once, so later rejection isolates stack capacity
    // from table-prefix admission and actual commitment remains one user page.
    for root in [first, second, other] {
        let mut stack = Stacks::user(root, 4).unwrap();
        stack.release().unwrap();
    }
    let initial = backing_used(principal).pages;
    limit(principal, initial + 20); // four user pages plus sixteen kernel pages
    let mut stack = Stacks::user(first, 4).unwrap();
    assert_eq!(stack.committed_pages(), 1);
    assert_eq!(backing_used(principal).pages, initial + 20);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..64 {
        assert!(Stacks::user(second, 4).is_err());
        assert!(!memory::commit_user_heap_page_handle(second, charlotte_launch::HEAP_VADDR));
        let mut healthy = Stacks::user(other, 4).unwrap();
        healthy.release().unwrap();
    }
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    let low = stack.user_stack().unwrap().base_addr();
    let mut growth = stack.begin_growth(low).unwrap();
    growth.run();
    assert_eq!(
        stack.finish_growth(growth).unwrap_or_else(|_| panic!("structural growth handback")),
        Some(low)
    );
    assert_eq!(stack.committed_pages(), 4);
    assert_eq!(backing_used(principal).pages, initial + 20);
    stack.release().unwrap();
    assert_eq!(backing_used(principal).pages, initial);
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert!(PreparingStackPage::with_frame(second, || None).is_err());
    let preparation = PreparingStackPage::reserve(second).unwrap();
    let invalid = preparation.base() - 4096;
    assert!(preparation.map(invalid).is_err());
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(backing_used(principal).pages, initial);
    for root in [first, second, other] {
        memory::close_user_address_space_handle(root).unwrap();
    }
    assert_eq!(backing_used(principal), Amount::default());
    assert_eq!(backing_used(other_principal), Amount::default());
}
