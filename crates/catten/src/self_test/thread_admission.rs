//! User thread preparation and observation authorization regressions.
use crate::{
    completion,
    cpu::{
        isa::interface::memory::{
            AddressSpaceInterface,
            MemoryMapping,
        },
        scheduler::{
            system_scheduler::publish_thread,
            threads::{
                MASTER_THREAD_TABLE,
                Thread,
            },
        },
    },
    memory::{
        self,
        ADDRESS_SPACE_TABLE,
        AddressSpaceHandle,
        PHYSICAL_FRAME_ALLOCATOR,
        VAddr,
        linear::PageType,
    },
    syscall::{
        self,
        TrapFrame,
    },
};

extern "C" fn unused_entry() {}

fn domain() -> AddressSpaceHandle {
    let handle =
        memory::register_user_address_space(memory::AddressSpace::try_new_user().unwrap()).unwrap();
    memory::set_domain_limits(
        handle,
        memory::DomainLimits {
            user_stack_pages: 1,
            max_threads: 1,
        },
    )
    .unwrap();
    completion::open_address_space(handle.id(), 16);
    handle
}

fn frame(asid: usize, arg1: u64, arg2: u64, arg3: u64) -> TrapFrame {
    let mut regs = [0; 19];
    regs[1] = arg1;
    regs[2] = arg2;
    regs[3] = arg3;
    TrapFrame {
        regs,
        elr_el1: 0,
        spsr_el1: 0,
        sp_el0: 0,
        lp_id: 0,
        asid,
    }
}

pub(crate) fn run() {
    memory::thread_stack::test_admission();
    crate::cpu::scheduler::threads::retirement_tests::run();
    let own = domain();
    let foreign = domain();
    let own_thread = Thread::try_new(own.id(), unused_entry).unwrap();
    let generation = own_thread.generation;
    let tid = publish_thread(own_thread).unwrap();
    assert!(crate::cpu::scheduler::system_scheduler::domain_has_live_threads(own));
    assert!(!crate::cpu::scheduler::system_scheduler::domain_has_live_threads(foreign));
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    for _ in 0..64 {
        let mut spawn = frame(own.id(), 0x0002_0000, 0, 0);
        syscall::syscall_dispatch(&mut spawn, catten_syscall::SyscallNumber::SpawnThread as u16);
        assert_eq!((spawn.regs[0], spawn.regs[1]), (u64::MAX, 0));
    }
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(ADDRESS_SPACE_TABLE.lock().get(own.id()).unwrap().thread_stack_slots, 1);
    let foreign_thread = Thread::try_new(foreign.id(), unused_entry).unwrap();
    let foreign_generation = foreign_thread.generation;
    let foreign_tid = publish_thread(foreign_thread).unwrap();
    let kernel_tid = publish_thread(Thread::new(memory::KERNEL_ASID, unused_entry)).unwrap();
    for (target, generation) in [(foreign_tid, foreign_generation), (kernel_tid, 1)] {
        for expected in [0, generation, u64::MAX] {
            for _ in 0..128 {
                let mut watch = frame(own.id(), target as u64, expected, 0);
                syscall::syscall_dispatch(
                    &mut watch,
                    catten_syscall::SyscallNumber::ObserveThreadExit as u16,
                );
                assert_eq!(watch.regs[0], u64::MAX);
                assert_eq!(MASTER_THREAD_TABLE.read().get(target).unwrap().exit_watch_count(), 0);
            }
        }
    }
    for expected in [0, generation] {
        let mut watch = frame(own.id(), tid as u64, expected, 0);
        syscall::syscall_dispatch(
            &mut watch,
            catten_syscall::SyscallNumber::ObserveThreadExit as u16,
        );
        assert_ne!(watch.regs[0], u64::MAX);
        completion::cancel(own.id(), watch.regs[0]).unwrap();
        completion::close(own.id(), watch.regs[0]).unwrap();
    }
    // A trusted internal adapter can still subscribe across domains.
    let watch = completion::observe_thread_exit_with_generation(
        own.id(),
        foreign_tid,
        Some(foreign_generation),
    )
    .unwrap();
    completion::cancel(own.id(), watch).unwrap();
    completion::close(own.id(), watch).unwrap();
    // Ordinary EL0 callers cannot acquire TCP/IP's owner-inspection authority.
    let mut status = frame(own.id(), foreign.id() as u64, foreign.generation() as u64, 0);
    syscall::syscall_dispatch(&mut status, catten_syscall::SyscallNumber::SocketOwnerStatus as u16);
    assert_eq!(status.regs[0], u64::MAX);
    for target in [tid, foreign_tid, kernel_tid] {
        let thread = MASTER_THREAD_TABLE.write().take_element(target).unwrap();
        drop(thread);
    }
    assert!(!crate::cpu::scheduler::system_scheduler::domain_has_live_threads(own));
    // Repeated detached preparation must reuse the same bounded slot.
    for _ in 0..128 {
        let thread = Thread::try_new(own.id(), unused_entry).unwrap();
        assert_eq!(ADDRESS_SPACE_TABLE.lock().get(own.id()).unwrap().thread_stack_slots, 1);
        drop(thread);
        assert_eq!(ADDRESS_SPACE_TABLE.lock().get(own.id()).unwrap().thread_stack_slots, 0);
    }
    let replacement = Thread::try_new(foreign.id(), unused_entry).unwrap();
    let replacement_tid = publish_thread(replacement).unwrap();
    let mut stale = frame(own.id(), replacement_tid as u64, foreign_generation, 0);
    syscall::syscall_dispatch(&mut stale, catten_syscall::SyscallNumber::ObserveThreadExit as u16);
    assert_eq!(stale.regs[0], u64::MAX);
    let thread = MASTER_THREAD_TABLE.write().take_element(replacement_tid).unwrap();
    drop(thread);
    memory::close_user_address_space_handle(foreign).unwrap();

    // Allocation rejection follows the same provisional owner as production.
    assert!(memory::thread_stack::PreparingStackPage::with_frame(own, || None).is_err());
    assert_eq!(ADDRESS_SPACE_TABLE.lock().get(own.id()).unwrap().thread_stack_slots, 0);
    memory::set_domain_limits(
        own,
        memory::DomainLimits {
            user_stack_pages: 1,
            max_threads: 64,
        },
    )
    .unwrap();
    let mut slots: [Option<memory::thread_stack::StackSlot>; 64] = core::array::from_fn(|_| None);
    for (index, slot) in slots.iter_mut().enumerate() {
        let reserved = memory::thread_stack::StackSlot::reserve(own).unwrap();
        assert_eq!(
            reserved.base(),
            charlotte_launch::user_address::STACK_BASE
                + index * charlotte_launch::user_address::STACK_STRIDE
        );
        *slot = Some(reserved);
    }
    assert!(memory::thread_stack::StackSlot::reserve(own).is_err());
    drop(slots);
    assert_eq!(ADDRESS_SPACE_TABLE.lock().get(own.id()).unwrap().thread_stack_slots, 0);
    let base = charlotte_launch::user_address::STACK_BASE;
    let object = memory::object::allocate(own.id(), 1).unwrap();
    assert!(memory::object::map(own.id(), object, VAddr::from(base), true).is_err());
    memory::object::close_cap(own.id(), object).unwrap();
    // Force a kernel-side layout collision to exercise the actual constructor
    // error, despite the application/ELF gates now excluding this layout.
    let backing = PHYSICAL_FRAME_ALLOCATOR.lock().allocate_frame().unwrap();
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(own.id())
        .unwrap()
        .map_page(MemoryMapping {
            vaddr: VAddr::from(base),
            paddr: backing,
            page_type: PageType::UserData,
        })
        .unwrap();
    let bytes: *mut u8 = backing.into();
    unsafe {
        core::ptr::write_bytes(bytes, 0x5a, 16);
    }
    let free = PHYSICAL_FRAME_ALLOCATOR.lock().free_frames();
    assert!(Thread::try_new(own.id(), unused_entry).is_err());
    assert!(unsafe { core::slice::from_raw_parts(bytes, 16) }.iter().all(|&byte| byte == 0x5a));
    assert_eq!(PHYSICAL_FRAME_ALLOCATOR.lock().free_frames(), free);
    assert_eq!(ADDRESS_SPACE_TABLE.lock().get(own.id()).unwrap().thread_stack_slots, 0);
    // An unstarted launch keeps its rollback owner until thread preparation
    // succeeds. The colliding mapping is reclaimed by complete root teardown.
    let loaded = crate::service::loader::LoadedDomain {
        asid: own.id(),
        address_space: own,
        entry_vaddr: unused_entry as *const () as usize,
        config_frame: backing,
        status_frame: backing,
    };
    assert!(
        crate::service::supervisor::try_start_domain_with_limits(
            loaded,
            crate::service::supervisor::ServiceLimits {
                user_stack_size: 4096,
                max_threads: 1
            }
        )
        .is_err()
    );
    assert!(!memory::address_space_handle_is_current(own));
    let successor = domain();
    assert_eq!(successor.id(), own.id());
    assert_ne!(successor, own);
    assert!(memory::thread_stack::StackSlot::reserve(own).is_err());
    let slot = memory::thread_stack::StackSlot::reserve(successor).unwrap();
    drop(slot);
    memory::close_user_address_space_handle(successor).unwrap();
    crate::logln!(
        "[thread admission] collision/quota rejection, slot reuse, launch rollback, scoped \
         watches and privileged owner inspection passed"
    );
}
