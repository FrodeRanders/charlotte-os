//! Serialized boot controls use real backing and private principal ceilings.
//! They qualify aggregate rejection and retained charges, not concurrent EL0 flood fairness.
use super::*;
use crate::memory::{
    self,
    ADDRESS_SPACE_TABLE,
    AddressSpace,
    AddressSpaceHandle,
    AddressSpaceInterface,
    PHYSICAL_FRAME_ALLOCATOR,
    PreparingUserBacking,
    backing_budget::{
        self,
        Kind,
    },
    linear::PageType,
    object::{
        self,
        MemoryObjectError,
    },
};

fn amount(pages: u64, objects: u64) -> Amount {
    Amount {
        pages,
        objects,
    }
}
fn new_domain(principal: u64) -> AddressSpaceHandle {
    let handle =
        memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    memory::register_domain_authority(handle, principal, 0).unwrap();
    handle
}
fn set_limit(principal: u64, pages: u64, objects: u64) {
    HISTORY.lock().get_mut(&principal).unwrap().backing.set_limit(amount(pages, objects)).unwrap();
}
fn image_page(handle: AddressSpaceHandle, vaddr: usize) -> Result<(), ()> {
    let mut table = ADDRESS_SPACE_TABLE.lock();
    let root = table.get_mut(handle.id()).unwrap();
    PreparingUserBacking::new(root, Kind::Image)
        .map_err(|_| ())?
        .map_with(vaddr.into(), PageType::UserData, |root, mapping| {
            root.map_existing_page(mapping).is_ok()
        })
        .map(|_| ())
}

fn root_pages(handle: AddressSpaceHandle) -> u64 {
    let table = ADDRESS_SPACE_TABLE.lock();
    let root = table.get(handle.id()).unwrap();
    root.heap_account.pages() + root.image_account.pages() + root.table_account.pages()
}

pub(in crate::memory) fn run() {
    mixed_backing_and_retired_sender();
    binding_is_atomic();
    object_count_is_independent();
    crate::logln!(
        "[principal backing admission] shared heap/image/object ceiling, copy sponsorship, \
         promotion rejection, 256 denied requests with unrelated progress, retained transfer \
         across exit/reuse, and atomic pre-launch binding passed"
    );
}

fn mixed_backing_and_retired_sender() {
    let principal = charlotte_launch::artifact_principal_id(b"aggregate-backing-control");
    let other_principal = charlotte_launch::artifact_principal_id(b"aggregate-backing-other");
    let first = new_domain(principal);
    let second = new_domain(principal);
    let other = new_domain(other_principal);
    assert!(memory::commit_user_heap_page_handle(first, charlotte_launch::HEAP_VADDR));
    image_page(second, 0x20000).unwrap();
    assert!(memory::commit_user_heap_page_handle(other, charlotte_launch::HEAP_VADDR));
    let initial = backing_used(principal).pages;
    let other_initial = backing_used(other_principal);
    let limit = initial + 4;
    set_limit(principal, limit, 2);
    let source = object::allocate(first.id(), 2).unwrap();
    let moved = object::move_to(first.id(), source, other.id()).unwrap();
    let second_object = object::allocate(second.id(), 2).unwrap();
    assert_eq!(backing_used(principal), amount(limit, 2));
    assert_eq!(backing_used(other_principal), other_initial);
    // A copy is sponsored by the copying caller, even when the full principal
    // receives its new capability. A move retains its original sponsor instead.
    let copied = object::copy_to(other.id(), moved, second.id()).unwrap();
    assert_eq!(backing_used(principal), amount(limit, 2));
    assert_eq!(
        backing_used(other_principal),
        amount(other_initial.pages + 2, other_initial.objects + 1)
    );
    object::close_cap(second.id(), copied).unwrap();
    assert_eq!(backing_used(other_principal), other_initial);
    memory::budget::mark_platform(second);
    assert_eq!(backing_used(principal), amount(limit, 2));
    let before = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..256 {
        assert_eq!(object::allocate(second.id(), 1), Err(MemoryObjectError::ResourceLimit));
        assert!(!memory::commit_user_heap_page_handle(second, charlotte_launch::HEAP_VADDR));
        assert!(image_page(second, 0x21000).is_err());
        let healthy = object::allocate(other.id(), 1).unwrap();
        object::close_cap(other.id(), healthy).unwrap();
    }
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), before);
    assert_eq!(backing_used(principal), amount(limit, 2));
    assert!(memory::commit_user_heap_page_handle(other, charlotte_launch::HEAP_VADDR + 4096));
    // Removing the sponsor root refunds its heap and private tables. The moved object still
    // consumes the same principal's ceiling after its source generation closes.
    memory::close_user_address_space_handle(first).unwrap();
    let restarted = new_domain(principal);
    assert_eq!(restarted.id(), first.id());
    assert_ne!(restarted, first);
    assert_eq!(backing_used(principal), amount(root_pages(second) + root_pages(restarted) + 4, 2));
    object::close_cap(second.id(), second_object).unwrap();
    memory::close_user_address_space_handle(second).unwrap();
    let restart_baseline = amount(root_pages(restarted) + 2, 1);
    assert_eq!(backing_used(principal), restart_baseline);
    let fill = object::allocate(restarted.id(), (limit - restart_baseline.pages) as usize).unwrap();
    assert_eq!(object::allocate(restarted.id(), 1), Err(MemoryObjectError::ResourceLimit));
    object::close_cap(restarted.id(), fill).unwrap();
    assert!(memory::commit_user_heap_page_handle(restarted, charlotte_launch::HEAP_VADDR));
    object::close_cap(other.id(), moved).unwrap();
    assert_eq!(backing_used(principal), amount(root_pages(restarted), 0));
    assert_eq!(memory::budget::used(restarted), Amount::default());
    memory::close_user_address_space_handle(restarted).unwrap();
    memory::close_user_address_space_handle(other).unwrap();
    assert_eq!(backing_used(principal), Amount::default());
    assert_eq!(backing_used(other_principal), Amount::default());
}

fn object_count_is_independent() {
    let principal = charlotte_launch::artifact_principal_id(b"aggregate-object-count-control");
    let first = new_domain(principal);
    let second = new_domain(principal);
    assert!(memory::commit_user_heap_page_handle(second, charlotte_launch::HEAP_VADDR));
    let initial = backing_used(principal).pages;
    set_limit(principal, initial + 4, 1);
    let cap = object::allocate(first.id(), 1).unwrap();
    assert_eq!(object::allocate(second.id(), 1), Err(MemoryObjectError::ResourceLimit));
    assert!(memory::commit_user_heap_page_handle(second, charlotte_launch::HEAP_VADDR + 4096));
    assert_eq!(backing_used(principal), amount(initial + 2, 1));
    object::close_cap(first.id(), cap).unwrap();
    let replacement = object::allocate(second.id(), 1).unwrap();
    assert_eq!(backing_used(principal), amount(initial + 2, 1));
    object::close_cap(second.id(), replacement).unwrap();
    memory::close_user_address_space_handle(first).unwrap();
    memory::close_user_address_space_handle(second).unwrap();
    assert_eq!(backing_used(principal), Amount::default());
}

fn binding_is_atomic() {
    let principal = charlotte_launch::artifact_principal_id(b"aggregate-binding-control");
    let mut prepared = PreparingHistory::try_new().unwrap();
    prepared.publish(principal);
    prepared.cancel_unpublished();
    let root = memory::register_user_address_space(AddressSpace::try_new_user().unwrap()).unwrap();
    let cap = object::allocate(root.id(), 3).unwrap();
    assert!(memory::commit_user_heap_page_handle(root, charlotte_launch::HEAP_VADDR));
    image_page(root, 0x20000).unwrap();
    let pages = root_pages(root);
    set_limit(principal, pages + 2, 2);
    assert_eq!(
        memory::register_domain_authority(root, principal, 0),
        Err(memory::DomainAuthorityError::ResourceLimit)
    );
    assert_eq!(memory::domain_authority(root.id()), None);
    assert_eq!(backing_used(principal), Amount::default());
    object::close_cap(root.id(), cap).unwrap();
    let smaller = object::allocate(root.id(), 1).unwrap();
    memory::register_domain_authority(root, principal, 0).unwrap();
    assert_eq!(backing_used(principal), amount(pages + 1, 1));
    // Existing generation-sponsored charges enter the aggregate at binding,
    // including an object charge originally created before authority install.
    object::close_cap(root.id(), smaller).unwrap();
    assert_eq!(backing_used(principal), amount(pages, 0));
    memory::close_user_address_space_handle(root).unwrap();
    assert_eq!(backing_used(principal), Amount::default());
}

fn fault_principal() -> u64 {
    charlotte_launch::artifact_principal_id(b"aggregate-backing-faults")
}

/// Raw unregistered roots in existing destructor fixtures have no authority slot.
/// Bind their backing owners directly to the same prepared principal record.
pub(in crate::memory) fn bind_raw_fixture(root: &mut AddressSpace) {
    let principal = fault_principal();
    let mut preparation = PreparingHistory::try_new().unwrap();
    preparation.publish(principal);
    preparation.cancel_unpublished();
    root.heap_account.bind_principal(BackingCharge::try_new(principal, Amount::default()).unwrap());
    root.image_account
        .bind_principal(BackingCharge::try_new(principal, Amount::default()).unwrap());
    root.table_account.bind_principal(
        BackingCharge::try_new(principal, amount(root.table_account.pages(), 0)).unwrap(),
    );
}

pub(in crate::memory) fn bind_fixture(handle: AddressSpaceHandle) {
    memory::register_domain_authority(handle, fault_principal(), 0).unwrap();
}

/// Existing fault controls verify real physical outcomes and node charges. Each
/// now additionally requires exact matching principal retention/refund, including
/// preparation Drop beneath table/allocator/pool guards and final-root failures.
pub(in crate::memory) fn check_retention(work: impl FnOnce()) {
    let before = backing_used(fault_principal());
    let pages = backing_budget::test_used_pages(Kind::Heap)
        + backing_budget::test_used_pages(Kind::Image)
        + memory::translation::account::test_used_pages()
        + memory::thread_stack::test_used_pages();
    work();
    let after = backing_budget::test_used_pages(Kind::Heap)
        + backing_budget::test_used_pages(Kind::Image)
        + memory::translation::account::test_used_pages()
        + memory::thread_stack::test_used_pages();
    assert_eq!(
        backing_used(fault_principal()),
        amount(before.pages + after - pages, before.objects)
    );
}
