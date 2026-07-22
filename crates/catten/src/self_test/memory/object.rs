use crate::{
    cpu::isa::{
        interface::memory::{
            AddressSpaceInterface,
            address::PhysicalAddress,
        },
        memory::paging::AddressSpace,
    },
    logln,
    memory::{
        ADDRESS_SPACE_TABLE,
        KERNEL_AS,
        close_user_address_space,
        linear::VAddr,
        object::{
            self,
            MemoryObjectError,
        },
    },
};

fn create_memory_object_test_address_space(label: &str) -> usize {
    let user_as = {
        let _kas = KERNEL_AS.lock();
        #[cfg_attr(not(target_arch = "aarch64"), allow(unused_mut))]
        let mut as_ = AddressSpace::get_current();
        #[cfg(target_arch = "aarch64")]
        as_.set_ttbr0(0);
        as_
    };
    let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
    logln!("[memory object] {} AS asid={}", label, asid);
    asid
}

pub fn test_memory_objects() {
    logln!("Testing first-class memory objects...");

    let owner = create_memory_object_test_address_space("owner");
    let target = create_memory_object_test_address_space("target");
    let reader = create_memory_object_test_address_space("reader");
    let writer = create_memory_object_test_address_space("writer");

    let cap = object::allocate(owner, 2).expect("memory object: allocation failed");
    let initial = object::info(owner, cap).expect("memory object: missing owner cap");
    assert_eq!(initial.owner, owner);
    assert_eq!(initial.pages, 2);
    assert!(!initial.mapped);

    let owner_base = VAddr::from(0x33000usize);
    object::map(owner, cap, owner_base, true).expect("memory object: owner map failed");
    let mapped = object::info(owner, cap).expect("memory object: missing mapped cap");
    assert!(mapped.mapped);
    let first_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(owner)
        .expect("memory object: owner AS missing")
        .translate_address(owner_base)
        .expect("memory object: owner translation failed");
    unsafe {
        let ptr = first_frame.into_hhdm_mut::<u64>();
        assert_eq!(ptr.read_volatile(), 0);
        ptr.write_volatile(0x4d45_4d4f_424a_4543);
    }
    assert_eq!(object::move_to(owner, cap, target), Err(MemoryObjectError::AlreadyMapped));

    object::unmap(owner, cap).expect("memory object: owner unmap failed");
    let target_cap = object::move_to(owner, cap, target).expect("memory object: move failed");
    assert_eq!(object::info(owner, cap), Err(MemoryObjectError::UnknownCapability));
    let target_info = object::info(target, target_cap).expect("memory object: target cap missing");
    assert_eq!(target_info.owner, target);
    assert!(!target_info.mapped);

    let target_base = VAddr::from(0x44000usize);
    object::map(target, target_cap, target_base, true).expect("memory object: target map failed");
    let target_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(target)
        .expect("memory object: target AS missing")
        .translate_address(target_base)
        .expect("memory object: target translation failed");
    unsafe {
        let ptr = target_frame.into_hhdm_mut::<u64>();
        assert_eq!(ptr.read_volatile(), 0x4d45_4d4f_424a_4543);
    }
    object::unmap(target, target_cap).expect("memory object: target unmap failed");
    object::close_cap(target, target_cap).expect("memory object: close failed");

    let lend_cap = object::allocate(owner, 1).expect("memory object: lend allocation failed");
    let lend_base = VAddr::from(0x55000usize);
    object::map(owner, lend_cap, lend_base, false).expect("memory object: owner read map failed");
    let reader_cap =
        object::lend_read(owner, lend_cap, reader).expect("memory object: read lend failed");
    assert_eq!(
        object::map(reader, reader_cap, VAddr::from(0x66000usize), true),
        Err(MemoryObjectError::MissingRight)
    );
    object::map(reader, reader_cap, VAddr::from(0x66000usize), false)
        .expect("memory object: reader read map failed");
    assert_eq!(object::lend_write(owner, lend_cap, writer), Err(MemoryObjectError::LendingActive));
    assert_eq!(object::close_cap(reader, reader_cap), Err(MemoryObjectError::LendingActive));
    object::revoke_lend(owner, lend_cap, reader, reader_cap)
        .expect("memory object: read revoke failed");
    assert_eq!(object::info(reader, reader_cap), Err(MemoryObjectError::UnknownCapability));
    object::unmap(owner, lend_cap).expect("memory object: owner read unmap failed");

    let writer_cap =
        object::lend_write(owner, lend_cap, writer).expect("memory object: write lend failed");
    assert_eq!(
        object::map(owner, lend_cap, VAddr::from(0x77000usize), false),
        Err(MemoryObjectError::LendingActive)
    );
    object::map(writer, writer_cap, VAddr::from(0x88000usize), true)
        .expect("memory object: writer map failed");
    let writer_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(writer)
        .expect("memory object: writer AS missing")
        .translate_address(VAddr::from(0x88000usize))
        .expect("memory object: writer translation failed");
    unsafe {
        writer_frame.into_hhdm_mut::<u64>().write_volatile(0x5749_5445_4c45_4e44);
    }
    object::revoke_lend(owner, lend_cap, writer, writer_cap)
        .expect("memory object: write revoke failed");
    object::map(owner, lend_cap, lend_base, true)
        .expect("memory object: owner remap after revoke failed");
    let owner_frame = ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(owner)
        .expect("memory object: owner AS missing")
        .translate_address(lend_base)
        .expect("memory object: owner translation after revoke failed");
    unsafe {
        assert_eq!(owner_frame.into_hhdm_mut::<u64>().read_volatile(), 0x5749_5445_4c45_4e44);
    }
    object::unmap(owner, lend_cap).expect("memory object: owner final unmap failed");
    object::close_cap(owner, lend_cap).expect("memory object: lend close failed");

    let borrower_cleanup_cap =
        object::allocate(owner, 1).expect("memory object: borrower cleanup allocation failed");
    let borrower_cleanup_lend = object::lend_write(owner, borrower_cleanup_cap, writer)
        .expect("memory object: borrower cleanup lend failed");
    object::map(writer, borrower_cleanup_lend, VAddr::from(0x99000usize), true)
        .expect("memory object: borrower cleanup map failed");
    object::close_address_space(writer);
    assert_eq!(
        object::info(writer, borrower_cleanup_lend),
        Err(MemoryObjectError::UnknownCapability)
    );
    let cleanup_info = object::info(owner, borrower_cleanup_cap)
        .expect("memory object: borrower cleanup owner cap missing");
    assert!(!cleanup_info.lent);
    object::map(owner, borrower_cleanup_cap, VAddr::from(0xaa000usize), true)
        .expect("memory object: owner remap after borrower close failed");
    object::unmap(owner, borrower_cleanup_cap).expect("memory object: owner cleanup unmap failed");
    object::close_cap(owner, borrower_cleanup_cap)
        .expect("memory object: borrower cleanup close failed");

    let owner_cleanup_cap =
        object::allocate(owner, 1).expect("memory object: owner cleanup allocation failed");
    let owner_cleanup_lend = object::lend_read(owner, owner_cleanup_cap, reader)
        .expect("memory object: owner cleanup lend failed");
    object::map(reader, owner_cleanup_lend, VAddr::from(0xbb000usize), false)
        .expect("memory object: owner cleanup reader map failed");
    object::close_address_space(owner);
    assert_eq!(object::info(reader, owner_cleanup_lend), Err(MemoryObjectError::UnknownCapability));
    object::close_address_space(reader);
    object::close_address_space(target);

    close_user_address_space(writer).expect("memory object: failed to close writer AS");
    close_user_address_space(reader).expect("memory object: failed to close reader AS");
    close_user_address_space(target).expect("memory object: failed to close target AS");
    close_user_address_space(owner).expect("memory object: failed to close owner AS");

    logln!("First-class memory object tests passed.");
}
