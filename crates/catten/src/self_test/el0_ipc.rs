//! EL0 endpoint IPC smoke test.
//!
//! This exercises the scalar endpoint ABI from a real userspace SVC path:
//! endpoint creation, same-address-space connection minting, send, call,
//! receive, reply, and reply polling. The test is intentionally single-AS; it
//! proves the user/kernel contract without adding a global name registry or a
//! userspace-provided target ASID.

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
static mut IPC_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;
#[cfg(target_arch = "aarch64")]
static mut IPC_BLOCK_RESULT_FRAME: Option<crate::memory::physical::PAddr> = None;

#[cfg(target_arch = "aarch64")]
unsafe extern "C" {
    static __catten_el0_ipc_start: u8;
    static __catten_el0_ipc_end: u8;
    static __catten_el0_ipc_block_server_start: u8;
    static __catten_el0_ipc_block_server_end: u8;
    static __catten_el0_ipc_block_client_start: u8;
    static __catten_el0_ipc_block_client_end: u8;
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
