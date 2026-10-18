//! Serialized boot fixtures: raw status writes construct a trusted launch-page
//! ABI record. No domain thread is published and every loaded root is closed.
use super::*;
use crate::memory::{
    self,
    allocators::global_allocator::PRIMARY_ALLOCATOR,
};

pub(super) fn run() {
    let image = crate::service::store::service_elf(b"ns").unwrap();
    let loaded = crate::service::loader::try_load_domain(image).unwrap();
    let handle = loaded.address_space;
    let base: *mut u8 = loaded.status_frame.into();
    use charlotte_launch::heap_status;
    unsafe {
        (base.add(heap_status::MAGIC_OFFSET) as *mut u64).write_volatile(heap_status::MAGIC);
        (base.add(heap_status::VERSION_OFFSET) as *mut u64).write_volatile(heap_status::VERSION);
        (base.add(heap_status::PEAK_OFFSET) as *mut u64).write_volatile(8192);
    }
    {
        let _heap = PRIMARY_ALLOCATOR.lock();
        assert_eq!(domain_heap_status(handle).unwrap().peak_bytes, 8192);
    }
    register_status_frame(handle.id(), loaded.config_frame);
    assert!(domain_heap_status(handle).is_none(), "foreign runtime mapping must reject");
    register_status_frame(handle.id(), loaded.status_frame);
    let closing = memory::retirement::ClosingAddressSpace::begin(handle).unwrap();
    assert!(domain_heap_status(handle).is_none(), "ordinary admission must reject closing root");
    assert_eq!(closing_heap_status(&closing).unwrap().peak_bytes, 8192);
    // Leave the real NS principal's adaptive sizing history untouched.
    unsafe { (base.add(heap_status::PEAK_OFFSET) as *mut u64).write_volatile(0) };
    assert!(matches!(closing.poll().unwrap(), memory::retirement::CloseProgress::Complete));
    assert!(domain_heap_status(handle).is_none());
    let next =
        memory::register_user_address_space(memory::AddressSpace::try_new_user().unwrap()).unwrap();
    assert_eq!(next.id(), handle.id());
    assert_ne!(next.generation(), handle.generation());
    assert!(domain_heap_status(handle).is_none(), "stale handle cannot sample successor");
    assert!(domain_heap_status(next).is_none(), "absent status is unavailable");
    memory::close_user_address_space_handle(next).unwrap();
}
