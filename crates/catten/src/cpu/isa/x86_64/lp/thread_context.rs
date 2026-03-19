use core::mem::offset_of;

const INIT_KERNEL_STACK_PAGES: usize = 16;

use crate::common::collections::id_table;
use crate::cpu::isa::init::gdt::{
    KERNEL_CODE_SELECTOR,
    KERNEL_DATA_SELECTOR,
    USER_CODE_SELECTOR,
    USER_DATA_SELECTOR,
};
use crate::cpu::isa::interface::memory::address::VirtualAddress;
use crate::cpu::isa::memory::paging::PAGE_SIZE;
use crate::logln;
use crate::memory::allocators::stack_allocator::allocate_stack;
use crate::memory::{ADDRESS_SPACE_TABLE, AddressSpaceId, KERNEL_AS, KERNEL_ASID, VAddr};

/// # Interrupt stack frame structure for x86_64 architecture
/// Note: must be 16 byte aligned as per `AMD APM 8.9.3`
#[repr(C, align(16))]
struct InterruptStackFrame {
    gprs: [u64; 15],
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

impl InterruptStackFrame {
    fn new(is_user: bool, entry_point: VAddr, return_rsp: VAddr, flags: u64) -> Self {
        InterruptStackFrame {
            gprs: [0u64; 15],
            rip: <VAddr as Into<u64>>::into(entry_point),
            cs: if is_user {
                USER_CODE_SELECTOR
            } else {
                KERNEL_CODE_SELECTOR
            } as u64,
            rflags: flags,
            rsp: <VAddr as Into<u64>>::into(return_rsp),
            ss: if is_user {
                USER_DATA_SELECTOR
            } else {
                KERNEL_DATA_SELECTOR
            } as u64,
        }
    }

    fn push_to_stack(rsp: VAddr, isf: InterruptStackFrame) -> VAddr {
        let new_rsp = rsp - core::mem::size_of::<InterruptStackFrame>();
        unsafe {
            let isf_ptr = new_rsp.into_mut::<InterruptStackFrame>();
            isf_ptr.write(isf);
        }
        new_rsp
    }
}

#[derive(Debug, Clone, Default)]
pub struct ThreadContext {
    pub cr3: u64,
    pub rsp_cpl0: u64,
    kernel_stack_buf: VAddr,
    user_stack_buf: Option<VAddr>,
}
#[derive(Debug)]
pub enum Error {
    AddressSpaceNotFound,
    StackAllocError(crate::memory::allocators::stack_allocator::Error),
    IdTableError(id_table::Error),
}

impl From<crate::memory::allocators::stack_allocator::Error> for Error {
    fn from(err: crate::memory::allocators::stack_allocator::Error) -> Self {
        Error::StackAllocError(err)
    }
}

impl From<id_table::Error> for Error {
    fn from(err: id_table::Error) -> Self {
        Error::IdTableError(err)
    }
}

impl ThreadContext {
    pub fn new(asid: AddressSpaceId, entry_point: VAddr) -> Result<Self, Error> {
        logln!("Creating thread context.");
        let flags: u64 = 0x202;
        let mut tctx = ThreadContext {
            rsp_cpl0: 0,
            cr3: if asid == KERNEL_ASID {
                KERNEL_AS.lock().get_cr3()
            } else {
                ADDRESS_SPACE_TABLE.get(asid)?.get_cr3()
            },
            kernel_stack_buf: allocate_stack(INIT_KERNEL_STACK_PAGES)?,
            user_stack_buf: if asid != KERNEL_ASID {
                Some(allocate_stack(INIT_KERNEL_STACK_PAGES)?)
            } else {
                None
            },
        };
        let kernel_stack_top = tctx.kernel_stack_buf + INIT_KERNEL_STACK_PAGES * PAGE_SIZE;
        let user_stack_top =
            tctx.user_stack_buf.map(|buf| buf + INIT_KERNEL_STACK_PAGES * PAGE_SIZE);
        let isf = InterruptStackFrame::new(
            asid != KERNEL_ASID,
            entry_point,
            if asid != KERNEL_ASID {
                user_stack_top.unwrap()
            } else {
                kernel_stack_top
            },
            flags,
        );
        tctx.rsp_cpl0 =
            <VAddr as Into<u64>>::into(InterruptStackFrame::push_to_stack(kernel_stack_top, isf));
        logln!("Thread Context created.");
        Ok(tctx)
    }
}

#[unsafe(no_mangle)]
pub static TC_RSP_CPL0_OFFSET: usize = offset_of!(ThreadContext, rsp_cpl0);
#[unsafe(no_mangle)]
pub static TC_CR3_OFFSET: usize = offset_of!(ThreadContext, cr3);
