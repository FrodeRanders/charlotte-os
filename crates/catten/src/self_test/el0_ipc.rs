//! EL0 endpoint IPC smoke test.
//!
//! This exercises the scalar endpoint ABI from real userspace SVC paths:
//! endpoint creation, same-address-space connection minting, cross-AS
//! connection delegation seeded by the kernel, send, call, receive, reply, and
//! reply polling. The userspace stubs never receive or pass ASIDs; authority is
//! represented only by caps in their own protection domains.

#[cfg(target_arch = "aarch64")]
use crate::cpu::isa::interface::memory::AddressSpaceInterface;
#[cfg(target_arch = "aarch64")]
use crate::cpu::isa::memory::paging::AddressSpace;
#[cfg(target_arch = "aarch64")]
use crate::cpu::scheduler::spawn_thread;
use crate::logln;
#[cfg(target_arch = "aarch64")]
use crate::memory::PHYSICAL_FRAME_ALLOCATOR;
#[cfg(target_arch = "aarch64")]
use crate::memory::{
    ADDRESS_SPACE_TABLE,
    KERNEL_AS,
    linear::{
        MemoryMapping,
        PageType,
        VAddr,
    },
};

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("el0_ipc.asm"));
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("el0_ipc_block.asm"));
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("el0_ipc_cross_as.asm"));
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("el0_ipc_memory.asm"));
#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("el0_ipc_memory_cancel.asm"));

#[cfg(target_arch = "aarch64")]
const IPC_CODE_VADDR: usize = 0x0000_0000_0001_0000;
#[cfg(target_arch = "aarch64")]
const IPC_RESULT_VADDR: usize = 0x0000_0000_0001_1000;
#[cfg(target_arch = "aarch64")]
const IPC_SENTINEL: u32 = 0x0000_1c50;
#[cfg(target_arch = "aarch64")]
const IPC_BLOCK_SERVER_VADDR: usize = 0x0000_0000_0001_2000;
#[cfg(target_arch = "aarch64")]
const IPC_BLOCK_CLIENT_VADDR: usize = 0x0000_0000_0001_3000;
#[cfg(target_arch = "aarch64")]
const IPC_BLOCK_RESULT_VADDR: usize = 0x0000_0000_0001_4000;
#[cfg(target_arch = "aarch64")]
const IPC_BLOCK_READY_SENTINEL: u32 = 0x0000_5150;
#[cfg(target_arch = "aarch64")]
const IPC_BLOCK_SERVER_SENTINEL: u32 = 0x0000_1c51;
#[cfg(target_arch = "aarch64")]
const IPC_BLOCK_CLIENT_SENTINEL: u32 = 0x0000_c117;
#[cfg(target_arch = "aarch64")]
const IPC_CROSS_CODE_VADDR: usize = 0x0000_0000_0001_0000;
#[cfg(target_arch = "aarch64")]
const IPC_CROSS_RESULT_VADDR: usize = 0x0000_0000_0001_1000;
#[cfg(target_arch = "aarch64")]
const IPC_CROSS_READY_SENTINEL: u32 = 0x0000_5e5e;
#[cfg(target_arch = "aarch64")]
const IPC_CROSS_SERVER_SENTINEL: u32 = 0x0000_5e51;
#[cfg(target_arch = "aarch64")]
const IPC_CROSS_CLIENT_SENTINEL: u32 = 0x0000_c1e1;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_CODE_VADDR: usize = 0x0000_0000_0001_0000;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_RESULT_VADDR: usize = 0x0000_0000_0001_1000;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_OBJECT_VADDR: usize = 0x0000_0000_0001_2000;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_READY_SENTINEL: u32 = 0x0000_6d5e;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_SERVER_SENTINEL: u32 = 0x0000_6d51;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_CLIENT_SENTINEL: u32 = 0x0000_c6d1;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_INITIAL_VALUE: u32 = 0x4d45_4d31;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_RETURNED_VALUE: u32 = 0x4d45_4d32;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_READ_BORROW_VALUE: u32 = 0x4252_5244;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_WRITE_BORROW_INITIAL_VALUE: u32 = 0x4257_5752;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_WRITE_BORROW_RETURNED_VALUE: u32 = 0x4252_5752;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_STATUS_MISSING_RIGHT: u32 = 12;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_CANCEL_READY_SENTINEL: u32 = 0x0000_ca5e;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_CANCEL_SERVER_SENTINEL: u32 = 0x0000_ca51;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_CANCEL_CLIENT_SENTINEL: u32 = 0x0000_cad1;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_CANCEL_BORROW_VALUE: u32 = 0x0000_b001;
#[cfg(target_arch = "aarch64")]
const IPC_STATUS_NO_MESSAGE: u32 = 2;
#[cfg(target_arch = "aarch64")]
const IPC_MEMORY_STATUS_UNKNOWN_CAPABILITY: u32 = 1;

#[cfg(target_arch = "aarch64")]
static mut IPC_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;
#[cfg(target_arch = "aarch64")]
static mut IPC_BLOCK_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;
#[cfg(target_arch = "aarch64")]
static mut IPC_CROSS_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;
#[cfg(target_arch = "aarch64")]
static mut IPC_CROSS_CLIENT_ASID: crate::memory::AddressSpaceId = 0;
#[cfg(target_arch = "aarch64")]
static mut IPC_MEMORY_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;
#[cfg(target_arch = "aarch64")]
static mut IPC_MEMORY_CANCEL_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    static __catten_el0_ipc_start: u8;
    static __catten_el0_ipc_end: u8;
    static __catten_el0_ipc_block_server_start: u8;
    static __catten_el0_ipc_block_server_end: u8;
    static __catten_el0_ipc_block_client_start: u8;
    static __catten_el0_ipc_block_client_end: u8;
    static __catten_el0_ipc_cross_server_start: u8;
    static __catten_el0_ipc_cross_server_end: u8;
    static __catten_el0_ipc_cross_client_start: u8;
    static __catten_el0_ipc_cross_client_end: u8;
    static __catten_el0_ipc_memory_server_start: u8;
    static __catten_el0_ipc_memory_server_end: u8;
    static __catten_el0_ipc_memory_client_start: u8;
    static __catten_el0_ipc_memory_client_end: u8;
    static __catten_el0_ipc_memory_cancel_server_start: u8;
    static __catten_el0_ipc_memory_cancel_server_end: u8;
    static __catten_el0_ipc_memory_cancel_client_start: u8;
    static __catten_el0_ipc_memory_cancel_client_end: u8;
}

#[cfg(target_arch = "aarch64")]
fn stub_bytes(start: *const u8, end: *const u8) -> &'static [u8] {
    let start = start as usize;
    let end = end as usize;
    assert!(end >= start, "EL0 IPC: invalid assembled stub bounds");
    unsafe { core::slice::from_raw_parts(start as *const u8, end - start) }
}

#[cfg(target_arch = "aarch64")]
fn ipc_stub_code() -> &'static [u8] {
    stub_bytes(
        core::ptr::addr_of!(__catten_el0_ipc_start),
        core::ptr::addr_of!(__catten_el0_ipc_end),
    )
}

#[cfg(target_arch = "aarch64")]
fn ipc_block_server_code() -> &'static [u8] {
    stub_bytes(
        core::ptr::addr_of!(__catten_el0_ipc_block_server_start),
        core::ptr::addr_of!(__catten_el0_ipc_block_server_end),
    )
}

#[cfg(target_arch = "aarch64")]
fn ipc_block_client_code() -> &'static [u8] {
    stub_bytes(
        core::ptr::addr_of!(__catten_el0_ipc_block_client_start),
        core::ptr::addr_of!(__catten_el0_ipc_block_client_end),
    )
}

#[cfg(target_arch = "aarch64")]
fn ipc_cross_server_code() -> &'static [u8] {
    stub_bytes(
        core::ptr::addr_of!(__catten_el0_ipc_cross_server_start),
        core::ptr::addr_of!(__catten_el0_ipc_cross_server_end),
    )
}

#[cfg(target_arch = "aarch64")]
fn ipc_cross_client_code() -> &'static [u8] {
    stub_bytes(
        core::ptr::addr_of!(__catten_el0_ipc_cross_client_start),
        core::ptr::addr_of!(__catten_el0_ipc_cross_client_end),
    )
}

#[cfg(target_arch = "aarch64")]
fn ipc_memory_server_code() -> &'static [u8] {
    stub_bytes(
        core::ptr::addr_of!(__catten_el0_ipc_memory_server_start),
        core::ptr::addr_of!(__catten_el0_ipc_memory_server_end),
    )
}

#[cfg(target_arch = "aarch64")]
fn ipc_memory_client_code() -> &'static [u8] {
    stub_bytes(
        core::ptr::addr_of!(__catten_el0_ipc_memory_client_start),
        core::ptr::addr_of!(__catten_el0_ipc_memory_client_end),
    )
}

#[cfg(target_arch = "aarch64")]
fn ipc_memory_cancel_server_code() -> &'static [u8] {
    stub_bytes(
        core::ptr::addr_of!(__catten_el0_ipc_memory_cancel_server_start),
        core::ptr::addr_of!(__catten_el0_ipc_memory_cancel_server_end),
    )
}

#[cfg(target_arch = "aarch64")]
fn ipc_memory_cancel_client_code() -> &'static [u8] {
    stub_bytes(
        core::ptr::addr_of!(__catten_el0_ipc_memory_cancel_client_start),
        core::ptr::addr_of!(__catten_el0_ipc_memory_cancel_client_end),
    )
}

#[cfg(target_arch = "aarch64")]
fn map_code_page(asid: usize, vaddr: VAddr, code: &[u8]) {
    let frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("EL0 IPC: failed to allocate code frame");
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(asid)
        .expect("EL0 IPC: address space not found")
        .map_page(MemoryMapping {
            vaddr,
            paddr: frame,
            page_type: PageType::UserCode,
        })
        .expect("EL0 IPC: failed to map code page");

    let hhdm: *mut u8 = frame.into();
    unsafe {
        core::ptr::write_bytes(hhdm, 0, 4096);
        core::ptr::copy_nonoverlapping(code.as_ptr(), hhdm, code.len());
    }
}

#[cfg(target_arch = "aarch64")]
fn map_result_page(asid: usize, vaddr: VAddr) -> crate::memory::physical::PAddr {
    let frame = PHYSICAL_FRAME_ALLOCATOR
        .lock()
        .allocate_frame()
        .expect("EL0 IPC: failed to allocate result frame");
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(asid)
        .expect("EL0 IPC: address space not found")
        .map_page(MemoryMapping {
            vaddr,
            paddr: frame,
            page_type: PageType::UserData,
        })
        .expect("EL0 IPC: failed to map result page");

    let hhdm: *mut u8 = frame.into();
    unsafe {
        core::ptr::write_bytes(hhdm, 0, 4096);
    }
    frame
}

#[cfg(target_arch = "aarch64")]
fn map_existing_data_page(asid: usize, vaddr: VAddr, frame: crate::memory::physical::PAddr) {
    ADDRESS_SPACE_TABLE
        .lock()
        .get_mut(asid)
        .expect("EL0 IPC: address space not found")
        .map_page(MemoryMapping {
            vaddr,
            paddr: frame,
            page_type: PageType::UserData,
        })
        .expect("EL0 IPC: failed to map existing data page");
}

#[cfg(target_arch = "aarch64")]
fn create_user_address_space(label: &str) -> usize {
    let user_as = {
        let _kas = KERNEL_AS.lock();
        let mut as_ = AddressSpace::get_current();
        as_.set_ttbr0(0);
        as_
    };
    let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
    logln!("[EL0 IPC] {} AS asid={}", label, asid);
    asid
}

pub fn test_el0_endpoint_ipc() {
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 endpoint IPC...");

        let user_as = {
            let _kas = KERNEL_AS.lock();
            let mut as_ = AddressSpace::get_current();
            as_.set_ttbr0(0);
            as_
        };
        let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
        logln!("[EL0 IPC] user AS asid={}", asid);

        map_code_page(asid, VAddr::from(IPC_CODE_VADDR), ipc_stub_code());
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "ic ialluis",
                "dsb ish",
                "isb",
                options(nomem, nostack, preserves_flags),
            );
        }

        let result_frame = map_result_page(asid, VAddr::from(IPC_RESULT_VADDR));
        unsafe {
            IPC_RESULT_FRAME = Some(result_frame);
        }

        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(IPC_CODE_VADDR) };
        let tid = spawn_thread(asid as crate::memory::AddressSpaceId, entry);
        logln!("[EL0 IPC] user thread spawned tid={} asid={}", tid, asid);

        let vtid = spawn_thread(crate::memory::KERNEL_ASID, verify_el0_endpoint_ipc);
        logln!("[EL0 IPC] verifier tid={}; assertion deferred.", vtid);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 endpoint IPC test (AArch64 only).");
    }
}

pub fn test_el0_endpoint_ipc_blocking_receive() {
    #[cfg(target_arch = "aarch64")]
    {
        logln!("Testing EL0 blocking endpoint receive...");

        let user_as = {
            let _kas = KERNEL_AS.lock();
            let mut as_ = AddressSpace::get_current();
            as_.set_ttbr0(0);
            as_
        };
        let asid = ADDRESS_SPACE_TABLE.lock().add_element(user_as);
        logln!("[EL0 IPC block] user AS asid={}", asid);

        map_code_page(asid, VAddr::from(IPC_BLOCK_SERVER_VADDR), ipc_block_server_code());
        map_code_page(asid, VAddr::from(IPC_BLOCK_CLIENT_VADDR), ipc_block_client_code());
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "ic ialluis",
                "dsb ish",
                "isb",
                options(nomem, nostack, preserves_flags),
            );
        }

        let result_frame = map_result_page(asid, VAddr::from(IPC_BLOCK_RESULT_VADDR));
        unsafe {
            IPC_BLOCK_RESULT_FRAME = Some(result_frame);
        }

        let server_entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(IPC_BLOCK_SERVER_VADDR) };
        let client_entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(IPC_BLOCK_CLIENT_VADDR) };
        let server_tid = spawn_thread(asid as crate::memory::AddressSpaceId, server_entry);
        let client_tid = spawn_thread(asid as crate::memory::AddressSpaceId, client_entry);
        logln!("[EL0 IPC block] server tid={} client tid={} asid={}", server_tid, client_tid, asid);

        let vtid = spawn_thread(crate::memory::KERNEL_ASID, verify_el0_endpoint_ipc_blocking);
        logln!("[EL0 IPC block] verifier tid={}; assertion deferred.", vtid);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 blocking endpoint receive test (AArch64 only).");
    }
}

pub fn test_el0_endpoint_ipc_cross_address_space() {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::ipc::ConnectionRights;

        logln!("Testing EL0 cross-address-space endpoint IPC...");

        let server_asid = create_user_address_space("cross-AS server");
        let client_asid = create_user_address_space("cross-AS client");

        let endpoint = crate::ipc::endpoint_create(server_asid, 0x4352_4f53, 1, 4)
            .expect("EL0 IPC cross-AS: endpoint_create should succeed");
        assert_eq!(endpoint, 1, "EL0 IPC cross-AS stub expects server endpoint cap 1");
        let connection = crate::ipc::connection_delegate(
            server_asid,
            endpoint,
            client_asid,
            ConnectionRights::CALL,
        )
        .expect("EL0 IPC cross-AS: connection_delegate should succeed");
        assert_eq!(connection, 1, "EL0 IPC cross-AS stub expects client connection cap 1");

        map_code_page(server_asid, VAddr::from(IPC_CROSS_CODE_VADDR), ipc_cross_server_code());
        map_code_page(client_asid, VAddr::from(IPC_CROSS_CODE_VADDR), ipc_cross_client_code());
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "ic ialluis",
                "dsb ish",
                "isb",
                options(nomem, nostack, preserves_flags),
            );
        }

        let result_frame = map_result_page(server_asid, VAddr::from(IPC_CROSS_RESULT_VADDR));
        map_existing_data_page(client_asid, VAddr::from(IPC_CROSS_RESULT_VADDR), result_frame);
        unsafe {
            IPC_CROSS_RESULT_FRAME = Some(result_frame);
            IPC_CROSS_CLIENT_ASID = client_asid;
        }

        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(IPC_CROSS_CODE_VADDR) };
        let server_tid = spawn_thread(server_asid as crate::memory::AddressSpaceId, entry);
        let client_tid = spawn_thread(client_asid as crate::memory::AddressSpaceId, entry);
        logln!(
            "[EL0 IPC cross-AS] server tid={} asid={} client tid={} asid={}",
            server_tid,
            server_asid,
            client_tid,
            client_asid
        );

        let vtid = spawn_thread(crate::memory::KERNEL_ASID, verify_el0_endpoint_ipc_cross_as);
        logln!("[EL0 IPC cross-AS] verifier tid={}; assertion deferred.", vtid);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 cross-address-space endpoint IPC test (AArch64 only).");
    }
}

pub fn test_el0_endpoint_ipc_memory_move() {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::ipc::ConnectionRights;

        logln!("Testing EL0 cross-address-space memory IPC...");

        let server_asid = create_user_address_space("memory server");
        let client_asid = create_user_address_space("memory client");

        let endpoint = crate::ipc::endpoint_create(server_asid, 0x4d45_4d49, 1, 4)
            .expect("EL0 IPC memory: endpoint_create should succeed");
        assert_eq!(endpoint, 1, "EL0 IPC memory stub expects server endpoint cap 1");
        let connection = crate::ipc::connection_delegate(
            server_asid,
            endpoint,
            client_asid,
            ConnectionRights::CALL,
        )
        .expect("EL0 IPC memory: connection_delegate should succeed");
        assert_eq!(connection, 1, "EL0 IPC memory stub expects client connection cap 1");

        map_code_page(server_asid, VAddr::from(IPC_MEMORY_CODE_VADDR), ipc_memory_server_code());
        map_code_page(client_asid, VAddr::from(IPC_MEMORY_CODE_VADDR), ipc_memory_client_code());
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "ic ialluis",
                "dsb ish",
                "isb",
                options(nomem, nostack, preserves_flags),
            );
        }

        let result_frame = map_result_page(server_asid, VAddr::from(IPC_MEMORY_RESULT_VADDR));
        map_existing_data_page(client_asid, VAddr::from(IPC_MEMORY_RESULT_VADDR), result_frame);
        unsafe {
            IPC_MEMORY_RESULT_FRAME = Some(result_frame);
        }

        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(IPC_MEMORY_CODE_VADDR) };
        let server_tid = spawn_thread(server_asid as crate::memory::AddressSpaceId, entry);
        let client_tid = spawn_thread(client_asid as crate::memory::AddressSpaceId, entry);
        logln!(
            "[EL0 IPC memory] server tid={} asid={} client tid={} asid={} object_vaddr={:#x}",
            server_tid,
            server_asid,
            client_tid,
            client_asid,
            IPC_MEMORY_OBJECT_VADDR
        );

        let vtid = spawn_thread(crate::memory::KERNEL_ASID, verify_el0_endpoint_ipc_memory_move);
        logln!("[EL0 IPC memory] verifier tid={}; assertion deferred.", vtid);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 memory endpoint IPC test (AArch64 only).");
    }
}

pub fn test_el0_endpoint_ipc_memory_cancel() {
    #[cfg(target_arch = "aarch64")]
    {
        use crate::ipc::ConnectionRights;

        logln!("Testing EL0 queued memory IPC cancellation...");

        let server_asid = create_user_address_space("memory cancel server");
        let client_asid = create_user_address_space("memory cancel client");

        let endpoint = crate::ipc::endpoint_create(server_asid, 0x4d45_4d43, 1, 4)
            .expect("EL0 IPC memory cancel: endpoint_create should succeed");
        assert_eq!(endpoint, 1, "EL0 IPC memory cancel stub expects server endpoint cap 1",);
        let connection = crate::ipc::connection_delegate(
            server_asid,
            endpoint,
            client_asid,
            ConnectionRights::CALL,
        )
        .expect("EL0 IPC memory cancel: connection_delegate should succeed");
        assert_eq!(connection, 1, "EL0 IPC memory cancel stub expects client connection cap 1",);

        map_code_page(
            server_asid,
            VAddr::from(IPC_MEMORY_CODE_VADDR),
            ipc_memory_cancel_server_code(),
        );
        map_code_page(
            client_asid,
            VAddr::from(IPC_MEMORY_CODE_VADDR),
            ipc_memory_cancel_client_code(),
        );
        unsafe {
            core::arch::asm!(
                "dsb ishst",
                "ic ialluis",
                "dsb ish",
                "isb",
                options(nomem, nostack, preserves_flags),
            );
        }

        let result_frame = map_result_page(server_asid, VAddr::from(IPC_MEMORY_RESULT_VADDR));
        map_existing_data_page(client_asid, VAddr::from(IPC_MEMORY_RESULT_VADDR), result_frame);
        unsafe {
            IPC_MEMORY_CANCEL_RESULT_FRAME = Some(result_frame);
        }

        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(IPC_MEMORY_CODE_VADDR) };
        let server_tid = spawn_thread(server_asid as crate::memory::AddressSpaceId, entry);
        let client_tid = spawn_thread(client_asid as crate::memory::AddressSpaceId, entry);
        logln!(
            "[EL0 IPC memory cancel] server tid={} asid={} client tid={} asid={}",
            server_tid,
            server_asid,
            client_tid,
            client_asid
        );

        let vtid = spawn_thread(crate::memory::KERNEL_ASID, verify_el0_endpoint_ipc_memory_cancel);
        logln!("[EL0 IPC memory cancel] verifier tid={}; assertion deferred.", vtid);
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        logln!("Skipping EL0 memory IPC cancellation test (AArch64 only).");
    }
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_endpoint_ipc() {
    use crate::cpu::scheduler::yield_lp;

    let frame = unsafe { IPC_RESULT_FRAME }.expect("EL0 IPC: result frame not initialized");
    let base: *mut u8 = frame.into();
    let result = base as *const u32;
    let mut spins: u64 = 0;
    loop {
        let sentinel = unsafe { core::ptr::read_volatile(result) };
        if sentinel == IPC_SENTINEL {
            let endpoint = unsafe { core::ptr::read_volatile(result.add(1)) };
            let connection = unsafe { core::ptr::read_volatile(result.add(2)) };
            let send_status = unsafe { core::ptr::read_volatile(result.add(3)) };
            let recv_send_status = unsafe { core::ptr::read_volatile(result.add(4)) };
            let recv_send_opcode = unsafe { core::ptr::read_volatile(result.add(5)) };
            let recv_send_arg0 = unsafe { core::ptr::read_volatile(result.add(6)) };
            let call_cap = unsafe { core::ptr::read_volatile(result.add(7)) };
            let recv_call_status = unsafe { core::ptr::read_volatile(result.add(8)) };
            let recv_call_opcode = unsafe { core::ptr::read_volatile(result.add(9)) };
            let recv_call_arg0 = unsafe { core::ptr::read_volatile(result.add(10)) };
            let reply_cap = unsafe { core::ptr::read_volatile(result.add(11)) };
            let reply_status = unsafe { core::ptr::read_volatile(result.add(12)) };
            let poll_status = unsafe { core::ptr::read_volatile(result.add(13)) };
            let poll_result = unsafe { core::ptr::read_volatile(result.add(14)) };
            let poll_cap = unsafe { core::ptr::read_volatile(result.add(15)) };

            assert_ne!(endpoint, 0, "EL0 IPC: endpoint_create returned no cap");
            assert_ne!(connection, 0, "EL0 IPC: connect returned no cap");
            assert_eq!(send_status, 0, "EL0 IPC: scalar_send failed");
            assert_eq!(recv_send_status, 0, "EL0 IPC: recv(send) failed");
            assert_eq!(recv_send_opcode, 7, "EL0 IPC: recv(send) opcode mismatch");
            assert_eq!(recv_send_arg0, 0x55, "EL0 IPC: recv(send) arg mismatch");
            assert_ne!(call_cap, 0, "EL0 IPC: scalar_call returned no cap");
            assert_eq!(recv_call_status, 0, "EL0 IPC: recv(call) failed");
            assert_eq!(recv_call_opcode, 8, "EL0 IPC: recv(call) opcode mismatch");
            assert_eq!(recv_call_arg0, 0x66, "EL0 IPC: recv(call) arg mismatch");
            assert_ne!(reply_cap, 0, "EL0 IPC: call message had no reply cap");
            assert_eq!(reply_status, 0, "EL0 IPC: reply failed");
            assert_eq!(poll_status, 0, "EL0 IPC: reply poll did not complete");
            assert_eq!(poll_result, 0x1234, "EL0 IPC: reply result mismatch");
            assert_eq!(poll_cap, 0, "EL0 IPC: plain reply returned a cap");

            logln!(
                "[EL0 IPC] SUCCESS: endpoint cap {}, connection cap {}, call cap {}, reply result \
                 {:#x}.",
                endpoint,
                connection,
                call_cap,
                poll_result
            );
            loop {
                yield_lp();
            }
        }
        spins += 1;
        assert!(
            spins < 20_000_000,
            "[EL0 IPC] FAILED: user thread did not write the result-page sentinel",
        );
        yield_lp();
    }
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_endpoint_ipc_blocking() {
    use crate::cpu::scheduler::yield_lp;

    let frame =
        unsafe { IPC_BLOCK_RESULT_FRAME }.expect("EL0 IPC block: result frame not initialized");
    let base: *mut u8 = frame.into();
    let result = base as *const u32;
    let mut spins: u64 = 0;
    loop {
        let ready = unsafe { core::ptr::read_volatile(result) };
        let server_done = unsafe { core::ptr::read_volatile(result.add(8)) };
        let client_done = unsafe { core::ptr::read_volatile(result.add(13)) };
        if ready == IPC_BLOCK_READY_SENTINEL
            && server_done == IPC_BLOCK_SERVER_SENTINEL
            && client_done == IPC_BLOCK_CLIENT_SENTINEL
        {
            let endpoint = unsafe { core::ptr::read_volatile(result.add(1)) };
            let connection = unsafe { core::ptr::read_volatile(result.add(2)) };
            let recv_status = unsafe { core::ptr::read_volatile(result.add(3)) };
            let recv_opcode = unsafe { core::ptr::read_volatile(result.add(4)) };
            let recv_arg0 = unsafe { core::ptr::read_volatile(result.add(5)) };
            let reply_cap = unsafe { core::ptr::read_volatile(result.add(6)) };
            let reply_status = unsafe { core::ptr::read_volatile(result.add(7)) };
            let call_cap = unsafe { core::ptr::read_volatile(result.add(9)) };
            let poll_status = unsafe { core::ptr::read_volatile(result.add(10)) };
            let poll_result = unsafe { core::ptr::read_volatile(result.add(11)) };
            let poll_cap = unsafe { core::ptr::read_volatile(result.add(12)) };

            assert_ne!(endpoint, 0, "EL0 IPC block: endpoint_create returned no cap");
            assert_ne!(connection, 0, "EL0 IPC block: connect returned no cap");
            assert_eq!(recv_status, 0, "EL0 IPC block: recv_block failed");
            assert_eq!(recv_opcode, 9, "EL0 IPC block: opcode mismatch");
            assert_eq!(recv_arg0, 0x77, "EL0 IPC block: arg mismatch");
            assert_ne!(reply_cap, 0, "EL0 IPC block: missing reply cap");
            assert_eq!(reply_status, 0, "EL0 IPC block: reply failed");
            assert_ne!(call_cap, 0, "EL0 IPC block: scalar_call returned no cap");
            assert_eq!(poll_status, 0, "EL0 IPC block: reply poll did not complete");
            assert_eq!(poll_result, 0x4567, "EL0 IPC block: reply result mismatch");
            assert_eq!(poll_cap, 0, "EL0 IPC block: plain reply returned a cap");

            logln!(
                "[EL0 IPC block] SUCCESS: server blocked on endpoint {}, client call cap {}, \
                 reply result {:#x}.",
                endpoint,
                call_cap,
                poll_result
            );
            loop {
                yield_lp();
            }
        }
        spins += 1;
        assert!(
            spins < 40_000_000,
            "[EL0 IPC block] FAILED: blocking receive flow did not complete (ready={:#x}, \
             server={:#x}, client={:#x})",
            ready,
            server_done,
            client_done
        );
        yield_lp();
    }
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_endpoint_ipc_cross_as() {
    use crate::cpu::scheduler::yield_lp;

    let frame =
        unsafe { IPC_CROSS_RESULT_FRAME }.expect("EL0 IPC cross-AS: result frame not initialized");
    let client_asid = unsafe { IPC_CROSS_CLIENT_ASID };
    let base: *mut u8 = frame.into();
    let result = base as *const u32;
    let mut spins: u64 = 0;
    loop {
        let ready = unsafe { core::ptr::read_volatile(result) };
        let server_done = unsafe { core::ptr::read_volatile(result.add(1)) };
        let client_done = unsafe { core::ptr::read_volatile(result.add(14)) };
        if ready == IPC_CROSS_READY_SENTINEL
            && server_done == IPC_CROSS_SERVER_SENTINEL
            && client_done == IPC_CROSS_CLIENT_SENTINEL
        {
            let recv_status = unsafe { core::ptr::read_volatile(result.add(2)) };
            let recv_opcode = unsafe { core::ptr::read_volatile(result.add(3)) };
            let recv_arg0 = unsafe { core::ptr::read_volatile(result.add(4)) };
            let reply_cap = unsafe { core::ptr::read_volatile(result.add(5)) };
            let sender = unsafe { core::ptr::read_volatile(result.add(6)) };
            let interface = unsafe { core::ptr::read_volatile(result.add(7)) };
            let version = unsafe { core::ptr::read_volatile(result.add(8)) };
            let reply_status = unsafe { core::ptr::read_volatile(result.add(9)) };
            let call_cap = unsafe { core::ptr::read_volatile(result.add(10)) };
            let poll_status = unsafe { core::ptr::read_volatile(result.add(11)) };
            let poll_result = unsafe { core::ptr::read_volatile(result.add(12)) };
            let poll_cap = unsafe { core::ptr::read_volatile(result.add(13)) };

            assert_eq!(recv_status, 0, "EL0 IPC cross-AS: recv_block failed");
            assert_eq!(recv_opcode, 0x33, "EL0 IPC cross-AS: opcode mismatch");
            assert_eq!(recv_arg0, 0x99, "EL0 IPC cross-AS: arg mismatch");
            assert_ne!(reply_cap, 0, "EL0 IPC cross-AS: missing reply cap");
            assert_eq!(sender, client_asid as u32, "EL0 IPC cross-AS: sender ASID mismatch");
            assert_eq!(interface, 0x4352_4f53, "EL0 IPC cross-AS: interface mismatch");
            assert_eq!(version, 1, "EL0 IPC cross-AS: version mismatch");
            assert_eq!(reply_status, 0, "EL0 IPC cross-AS: reply failed");
            assert_ne!(call_cap, 0, "EL0 IPC cross-AS: scalar_call returned no cap");
            assert_eq!(poll_status, 0, "EL0 IPC cross-AS: reply poll did not complete");
            assert_eq!(poll_result, 0x6789, "EL0 IPC cross-AS: reply result mismatch");
            assert_eq!(poll_cap, 0, "EL0 IPC cross-AS: plain reply returned a cap");

            logln!(
                "[EL0 IPC cross-AS] SUCCESS: client AS {} called server endpoint; call cap {}, \
                 reply result {:#x}.",
                sender,
                call_cap,
                poll_result
            );
            loop {
                yield_lp();
            }
        }
        spins += 1;
        assert!(
            spins < 40_000_000,
            "[EL0 IPC cross-AS] FAILED: flow did not complete (ready={:#x}, server={:#x}, \
             client={:#x})",
            ready,
            server_done,
            client_done
        );
        yield_lp();
    }
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_endpoint_ipc_memory_move() {
    use crate::cpu::scheduler::yield_lp;

    let frame =
        unsafe { IPC_MEMORY_RESULT_FRAME }.expect("EL0 IPC memory: result frame not initialized");
    let base: *mut u8 = frame.into();
    let result = base as *const u32;
    let mut spins: u64 = 0;
    loop {
        let ready = unsafe { core::ptr::read_volatile(result) };
        let server_done = unsafe { core::ptr::read_volatile(result.add(1)) };
        let client_done = unsafe { core::ptr::read_volatile(result.add(2)) };
        if ready == IPC_MEMORY_READY_SENTINEL
            && server_done == IPC_MEMORY_SERVER_SENTINEL
            && client_done == IPC_MEMORY_CLIENT_SENTINEL
        {
            let client_alloc_cap = unsafe { core::ptr::read_volatile(result.add(3)) };
            let client_map = unsafe { core::ptr::read_volatile(result.add(4)) };
            let client_unmap = unsafe { core::ptr::read_volatile(result.add(5)) };
            let client_call_cap = unsafe { core::ptr::read_volatile(result.add(6)) };
            let client_moved_map = unsafe { core::ptr::read_volatile(result.add(7)) };
            let client_poll = unsafe { core::ptr::read_volatile(result.add(8)) };
            let client_poll_result = unsafe { core::ptr::read_volatile(result.add(9)) };
            let client_poll_cap = unsafe { core::ptr::read_volatile(result.add(10)) };
            let client_returned_memory = unsafe { core::ptr::read_volatile(result.add(11)) };
            let client_returned_map = unsafe { core::ptr::read_volatile(result.add(12)) };
            let client_returned_value = unsafe { core::ptr::read_volatile(result.add(13)) };
            let client_returned_unmap = unsafe { core::ptr::read_volatile(result.add(14)) };
            let client_close = unsafe { core::ptr::read_volatile(result.add(15)) };
            let server_recv = unsafe { core::ptr::read_volatile(result.add(16)) };
            let server_opcode = unsafe { core::ptr::read_volatile(result.add(17)) };
            let server_arg0 = unsafe { core::ptr::read_volatile(result.add(18)) };
            let server_reply = unsafe { core::ptr::read_volatile(result.add(19)) };
            let server_memory = unsafe { core::ptr::read_volatile(result.add(20)) };
            let server_map = unsafe { core::ptr::read_volatile(result.add(21)) };
            let server_read_value = unsafe { core::ptr::read_volatile(result.add(22)) };
            let server_unmap = unsafe { core::ptr::read_volatile(result.add(23)) };
            let server_reply_move = unsafe { core::ptr::read_volatile(result.add(24)) };
            let client_read_borrow_cap = unsafe { core::ptr::read_volatile(result.add(25)) };
            let client_read_borrow_map = unsafe { core::ptr::read_volatile(result.add(26)) };
            let client_read_borrow_unmap = unsafe { core::ptr::read_volatile(result.add(27)) };
            let client_read_borrow_call = unsafe { core::ptr::read_volatile(result.add(28)) };
            let client_read_borrow_poll = unsafe { core::ptr::read_volatile(result.add(29)) };
            let client_read_borrow_result = unsafe { core::ptr::read_volatile(result.add(30)) };
            let client_read_borrow_cap_reply = unsafe { core::ptr::read_volatile(result.add(31)) };
            let client_read_borrow_memory_reply =
                unsafe { core::ptr::read_volatile(result.add(32)) };
            let client_read_borrow_owner_map = unsafe { core::ptr::read_volatile(result.add(33)) };
            let client_read_borrow_owner_unmap =
                unsafe { core::ptr::read_volatile(result.add(34)) };
            let client_read_borrow_close = unsafe { core::ptr::read_volatile(result.add(35)) };
            let client_write_borrow_cap = unsafe { core::ptr::read_volatile(result.add(36)) };
            let client_write_borrow_map = unsafe { core::ptr::read_volatile(result.add(37)) };
            let client_write_borrow_unmap = unsafe { core::ptr::read_volatile(result.add(38)) };
            let client_write_borrow_call = unsafe { core::ptr::read_volatile(result.add(39)) };
            let client_write_borrow_poll = unsafe { core::ptr::read_volatile(result.add(40)) };
            let client_write_borrow_result = unsafe { core::ptr::read_volatile(result.add(41)) };
            let client_write_borrow_cap_reply = unsafe { core::ptr::read_volatile(result.add(42)) };
            let client_write_borrow_memory_reply =
                unsafe { core::ptr::read_volatile(result.add(43)) };
            let client_write_borrow_returned_map =
                unsafe { core::ptr::read_volatile(result.add(44)) };
            let client_write_borrow_returned_value =
                unsafe { core::ptr::read_volatile(result.add(45)) };
            let client_write_borrow_returned_unmap =
                unsafe { core::ptr::read_volatile(result.add(46)) };
            let client_write_borrow_close = unsafe { core::ptr::read_volatile(result.add(47)) };
            let server_read_borrow_recv = unsafe { core::ptr::read_volatile(result.add(50)) };
            let server_read_borrow_opcode = unsafe { core::ptr::read_volatile(result.add(51)) };
            let server_read_borrow_arg0 = unsafe { core::ptr::read_volatile(result.add(52)) };
            let server_read_borrow_reply = unsafe { core::ptr::read_volatile(result.add(53)) };
            let server_read_borrow_memory = unsafe { core::ptr::read_volatile(result.add(54)) };
            let server_read_borrow_write_map = unsafe { core::ptr::read_volatile(result.add(55)) };
            let server_read_borrow_map = unsafe { core::ptr::read_volatile(result.add(56)) };
            let server_read_borrow_value = unsafe { core::ptr::read_volatile(result.add(57)) };
            let server_read_borrow_unmap = unsafe { core::ptr::read_volatile(result.add(58)) };
            let server_read_borrow_reply_status =
                unsafe { core::ptr::read_volatile(result.add(59)) };
            let server_write_borrow_recv = unsafe { core::ptr::read_volatile(result.add(60)) };
            let server_write_borrow_opcode = unsafe { core::ptr::read_volatile(result.add(61)) };
            let server_write_borrow_arg0 = unsafe { core::ptr::read_volatile(result.add(62)) };
            let server_write_borrow_reply = unsafe { core::ptr::read_volatile(result.add(63)) };
            let server_write_borrow_memory = unsafe { core::ptr::read_volatile(result.add(64)) };
            let server_write_borrow_map = unsafe { core::ptr::read_volatile(result.add(65)) };
            let server_write_borrow_value = unsafe { core::ptr::read_volatile(result.add(66)) };
            let server_write_borrow_unmap = unsafe { core::ptr::read_volatile(result.add(67)) };
            let server_write_borrow_reply_status =
                unsafe { core::ptr::read_volatile(result.add(68)) };

            assert_ne!(client_alloc_cap, 0, "EL0 IPC memory: allocation returned no cap");
            assert_eq!(client_map, 0, "EL0 IPC memory: client initial map failed");
            assert_eq!(client_unmap, 0, "EL0 IPC memory: client initial unmap failed");
            assert_ne!(client_call_cap, 0, "EL0 IPC memory: call_move returned no cap");
            assert_eq!(
                client_moved_map, 1,
                "EL0 IPC memory: moved-from cap should be unknown to caller",
            );
            assert_eq!(server_recv, 0, "EL0 IPC memory: server recv_block failed");
            assert_eq!(server_opcode, 0x44, "EL0 IPC memory: opcode mismatch");
            assert_eq!(server_arg0, 0xab, "EL0 IPC memory: arg mismatch");
            assert_ne!(server_reply, 0, "EL0 IPC memory: missing reply token");
            assert_ne!(server_memory, 0, "EL0 IPC memory: missing moved memory cap");
            assert_eq!(server_map, 0, "EL0 IPC memory: server map failed");
            assert_eq!(
                server_read_value, IPC_MEMORY_INITIAL_VALUE,
                "EL0 IPC memory: server saw wrong payload",
            );
            assert_eq!(server_unmap, 0, "EL0 IPC memory: server unmap failed");
            assert_eq!(server_reply_move, 0, "EL0 IPC memory: reply_move failed");
            assert_eq!(client_poll, 0, "EL0 IPC memory: reply poll did not complete");
            assert_eq!(client_poll_result, 0x2468, "EL0 IPC memory: reply result mismatch");
            assert_eq!(client_poll_cap, 0, "EL0 IPC memory: reply returned unexpected IPC cap");
            assert_ne!(
                client_returned_memory, 0,
                "EL0 IPC memory: reply did not return memory cap",
            );
            assert_eq!(client_returned_map, 0, "EL0 IPC memory: returned map failed");
            assert_eq!(
                client_returned_value, IPC_MEMORY_RETURNED_VALUE,
                "EL0 IPC memory: returned payload mismatch",
            );
            assert_eq!(client_returned_unmap, 0, "EL0 IPC memory: returned unmap failed",);
            assert_eq!(client_close, 0, "EL0 IPC memory: returned close failed");

            assert_ne!(
                client_read_borrow_cap, 0,
                "EL0 IPC memory: read-borrow allocation returned no cap",
            );
            assert_eq!(client_read_borrow_map, 0, "EL0 IPC memory: read-borrow seed map failed");
            assert_eq!(
                client_read_borrow_unmap, 0,
                "EL0 IPC memory: read-borrow seed unmap failed",
            );
            assert_ne!(
                client_read_borrow_call, 0,
                "EL0 IPC memory: borrow_read returned no pending-call cap",
            );
            assert_eq!(
                server_read_borrow_recv, 0,
                "EL0 IPC memory: server read-borrow recv failed",
            );
            assert_eq!(server_read_borrow_opcode, 0x45, "EL0 IPC memory: read-borrow opcode");
            assert_eq!(server_read_borrow_arg0, 0xbc, "EL0 IPC memory: read-borrow arg");
            assert_ne!(server_read_borrow_reply, 0, "EL0 IPC memory: read-borrow reply");
            assert_ne!(server_read_borrow_memory, 0, "EL0 IPC memory: read-borrow memory");
            assert_eq!(
                server_read_borrow_write_map, IPC_MEMORY_STATUS_MISSING_RIGHT,
                "EL0 IPC memory: read-borrow must not map writable",
            );
            assert_eq!(server_read_borrow_map, 0, "EL0 IPC memory: read-borrow map failed");
            assert_eq!(
                server_read_borrow_value, IPC_MEMORY_READ_BORROW_VALUE,
                "EL0 IPC memory: read-borrow payload mismatch",
            );
            assert_eq!(server_read_borrow_unmap, 0, "EL0 IPC memory: read-borrow unmap failed",);
            assert_eq!(
                server_read_borrow_reply_status, 0,
                "EL0 IPC memory: read-borrow reply failed",
            );
            assert_eq!(client_read_borrow_poll, 0, "EL0 IPC memory: read-borrow poll failed",);
            assert_eq!(
                client_read_borrow_result, 0x1357,
                "EL0 IPC memory: read-borrow reply result mismatch",
            );
            assert_eq!(
                client_read_borrow_cap_reply, 0,
                "EL0 IPC memory: read-borrow returned unexpected IPC cap",
            );
            assert_eq!(
                client_read_borrow_memory_reply, 0,
                "EL0 IPC memory: read-borrow returned unexpected memory cap",
            );
            assert_eq!(
                client_read_borrow_owner_map, 0,
                "EL0 IPC memory: read-borrow owner remap after reply failed",
            );
            assert_eq!(
                client_read_borrow_owner_unmap, 0,
                "EL0 IPC memory: read-borrow owner unmap failed",
            );
            assert_eq!(
                client_read_borrow_close, 0,
                "EL0 IPC memory: read-borrow owner close failed",
            );

            assert_ne!(
                client_write_borrow_cap, 0,
                "EL0 IPC memory: write-borrow allocation returned no cap",
            );
            assert_eq!(client_write_borrow_map, 0, "EL0 IPC memory: write-borrow seed map failed");
            assert_eq!(
                client_write_borrow_unmap, 0,
                "EL0 IPC memory: write-borrow seed unmap failed",
            );
            assert_ne!(
                client_write_borrow_call, 0,
                "EL0 IPC memory: borrow_write returned no pending-call cap",
            );
            assert_eq!(
                server_write_borrow_recv, 0,
                "EL0 IPC memory: server write-borrow recv failed",
            );
            assert_eq!(server_write_borrow_opcode, 0x46, "EL0 IPC memory: write-borrow opcode",);
            assert_eq!(server_write_borrow_arg0, 0xcd, "EL0 IPC memory: write-borrow arg");
            assert_ne!(server_write_borrow_reply, 0, "EL0 IPC memory: write-borrow reply");
            assert_ne!(server_write_borrow_memory, 0, "EL0 IPC memory: write-borrow memory");
            assert_eq!(server_write_borrow_map, 0, "EL0 IPC memory: write-borrow map failed");
            assert_eq!(
                server_write_borrow_value, IPC_MEMORY_WRITE_BORROW_INITIAL_VALUE,
                "EL0 IPC memory: write-borrow payload mismatch",
            );
            assert_eq!(server_write_borrow_unmap, 0, "EL0 IPC memory: write-borrow unmap failed",);
            assert_eq!(
                server_write_borrow_reply_status, 0,
                "EL0 IPC memory: write-borrow reply failed",
            );
            assert_eq!(client_write_borrow_poll, 0, "EL0 IPC memory: write-borrow poll failed",);
            assert_eq!(
                client_write_borrow_result, 0x2469,
                "EL0 IPC memory: write-borrow reply result mismatch",
            );
            assert_eq!(
                client_write_borrow_cap_reply, 0,
                "EL0 IPC memory: write-borrow returned unexpected IPC cap",
            );
            assert_eq!(
                client_write_borrow_memory_reply, 0,
                "EL0 IPC memory: write-borrow returned unexpected memory cap",
            );
            assert_eq!(
                client_write_borrow_returned_map, 0,
                "EL0 IPC memory: write-borrow owner remap failed",
            );
            assert_eq!(
                client_write_borrow_returned_value, IPC_MEMORY_WRITE_BORROW_RETURNED_VALUE,
                "EL0 IPC memory: write-borrow returned payload mismatch",
            );
            assert_eq!(
                client_write_borrow_returned_unmap, 0,
                "EL0 IPC memory: write-borrow owner unmap failed",
            );
            assert_eq!(
                client_write_borrow_close, 0,
                "EL0 IPC memory: write-borrow owner close failed",
            );

            logln!(
                "[EL0 IPC memory] SUCCESS: moved cap {} to server cap {}, returned cap {}, \
                 borrow-read {:#x}, borrow-write {:#x}.",
                client_alloc_cap,
                server_memory,
                client_returned_memory,
                server_read_borrow_value,
                client_write_borrow_returned_value
            );
            loop {
                yield_lp();
            }
        }
        spins += 1;
        assert!(
            spins < 60_000_000,
            "[EL0 IPC memory] FAILED: flow did not complete (ready={:#x}, server={:#x}, \
             client={:#x})",
            ready,
            server_done,
            client_done
        );
        yield_lp();
    }
}

#[cfg(target_arch = "aarch64")]
extern "C" fn verify_el0_endpoint_ipc_memory_cancel() {
    use crate::cpu::scheduler::yield_lp;

    let frame = unsafe { IPC_MEMORY_CANCEL_RESULT_FRAME }
        .expect("EL0 IPC memory cancel: result frame not initialized");
    let base: *mut u8 = frame.into();
    let result = base as *const u32;
    let mut spins: u64 = 0;
    loop {
        let ready = unsafe { core::ptr::read_volatile(result) };
        let server_done = unsafe { core::ptr::read_volatile(result.add(1)) };
        let client_done = unsafe { core::ptr::read_volatile(result.add(2)) };
        if ready == IPC_MEMORY_CANCEL_READY_SENTINEL
            && server_done == IPC_MEMORY_CANCEL_SERVER_SENTINEL
            && client_done == IPC_MEMORY_CANCEL_CLIENT_SENTINEL
        {
            let move_alloc_cap = unsafe { core::ptr::read_volatile(result.add(3)) };
            let move_map = unsafe { core::ptr::read_volatile(result.add(4)) };
            let move_unmap = unsafe { core::ptr::read_volatile(result.add(5)) };
            let move_call_cap = unsafe { core::ptr::read_volatile(result.add(6)) };
            let move_pending_close = unsafe { core::ptr::read_volatile(result.add(7)) };
            let moved_from_map = unsafe { core::ptr::read_volatile(result.add(8)) };
            let borrow_alloc_cap = unsafe { core::ptr::read_volatile(result.add(9)) };
            let borrow_map = unsafe { core::ptr::read_volatile(result.add(10)) };
            let borrow_unmap = unsafe { core::ptr::read_volatile(result.add(11)) };
            let borrow_call_cap = unsafe { core::ptr::read_volatile(result.add(12)) };
            let borrow_pending_close = unsafe { core::ptr::read_volatile(result.add(13)) };
            let borrow_owner_remap = unsafe { core::ptr::read_volatile(result.add(14)) };
            let borrow_owner_value = unsafe { core::ptr::read_volatile(result.add(15)) };
            let borrow_owner_unmap = unsafe { core::ptr::read_volatile(result.add(16)) };
            let borrow_owner_close = unsafe { core::ptr::read_volatile(result.add(17)) };
            let server_first_recv = unsafe { core::ptr::read_volatile(result.add(18)) };
            let server_second_recv = unsafe { core::ptr::read_volatile(result.add(19)) };

            assert_ne!(move_alloc_cap, 0, "EL0 IPC memory cancel: move allocation returned no cap",);
            assert_eq!(move_map, 0, "EL0 IPC memory cancel: move seed map failed");
            assert_eq!(move_unmap, 0, "EL0 IPC memory cancel: move seed unmap failed");
            assert_ne!(
                move_call_cap, 0,
                "EL0 IPC memory cancel: call_move returned no pending-call cap",
            );
            assert_eq!(
                move_pending_close, 0,
                "EL0 IPC memory cancel: closing queued moved-memory call failed",
            );
            assert_eq!(
                moved_from_map, IPC_MEMORY_STATUS_UNKNOWN_CAPABILITY,
                "EL0 IPC memory cancel: moved-from cap should remain consumed",
            );

            assert_ne!(
                borrow_alloc_cap, 0,
                "EL0 IPC memory cancel: borrow allocation returned no cap",
            );
            assert_eq!(borrow_map, 0, "EL0 IPC memory cancel: borrow seed map failed");
            assert_eq!(borrow_unmap, 0, "EL0 IPC memory cancel: borrow seed unmap failed");
            assert_ne!(
                borrow_call_cap, 0,
                "EL0 IPC memory cancel: borrow_write returned no pending-call cap",
            );
            assert_eq!(
                borrow_pending_close, 0,
                "EL0 IPC memory cancel: closing queued borrow call failed",
            );
            assert_eq!(
                borrow_owner_remap, 0,
                "EL0 IPC memory cancel: cancelled borrow was not revoked to owner",
            );
            assert_eq!(
                borrow_owner_value, IPC_MEMORY_CANCEL_BORROW_VALUE,
                "EL0 IPC memory cancel: borrow owner payload changed unexpectedly",
            );
            assert_eq!(borrow_owner_unmap, 0, "EL0 IPC memory cancel: borrow owner unmap failed",);
            assert_eq!(borrow_owner_close, 0, "EL0 IPC memory cancel: borrow owner close failed",);

            assert_eq!(
                server_first_recv, IPC_STATUS_NO_MESSAGE,
                "EL0 IPC memory cancel: server received cancelled moved-memory call",
            );
            assert_eq!(
                server_second_recv, IPC_STATUS_NO_MESSAGE,
                "EL0 IPC memory cancel: server received cancelled borrow call",
            );

            logln!(
                "[EL0 IPC memory cancel] SUCCESS: cancelled move call cap {}, borrow call cap {}, \
                 server recv statuses {}/{}.",
                move_call_cap,
                borrow_call_cap,
                server_first_recv,
                server_second_recv
            );
            loop {
                yield_lp();
            }
        }
        spins += 1;
        assert!(
            spins < 40_000_000,
            "[EL0 IPC memory cancel] FAILED: flow did not complete (ready={:#x}, server={:#x}, \
             client={:#x})",
            ready,
            server_done,
            client_done
        );
        yield_lp();
    }
}
