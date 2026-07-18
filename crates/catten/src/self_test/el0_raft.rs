#[cfg(target_arch = "aarch64")]
mod inner {
    use crate::{
        ipc::ConnectionRights,
        service::supervisor::{self, NameServiceHandle, ServiceDomain},
    };

    const NS_ELF: &[u8] = include_bytes!("ns.elf");
    const RAFT_ELF: &[u8] = include_bytes!("raft.elf");

    const fn packed_name(bytes: &[u8]) -> u64 {
        let mut packed = [0u8; 8];
        let mut i = 0;
        while i < bytes.len() && i < 8 {
            packed[i] = bytes[i];
            i += 1;
        }
        u64::from_le_bytes(packed)
    }

    const NS_INTERFACE: u64 = packed_name(b"NAME");
    const _RAFT_INTERFACE: u64 = packed_name(b"RAFT");

    const ARGC_OFFSET: usize = 24;
    const ARGS_OFFSET: usize = 32;

    fn spawn_raft_node(args: &[u32], ns_handle: &NameServiceHandle) -> ServiceDomain {
        let addr = crate::service::loader::load_domain(RAFT_ELF);
        let conn = crate::ipc::connection_delegate(
            ns_handle.domain.asid, ns_handle.endpoint_cap, addr.asid,
            ConnectionRights::SEND | ConnectionRights::CALL,
        ).expect("raft conn delegate");
        crate::service::bootstrap::write_bootstrap_cap(addr.config_frame, conn);
        let base: *mut u8 = addr.config_frame.into();
        unsafe {
            core::ptr::write_volatile(base.add(ARGC_OFFSET) as *mut usize, args.len());
            for (i, &a) in args.iter().enumerate() {
                core::ptr::write_volatile(base.add(ARGS_OFFSET + i * 4) as *mut u32, a);
            }
        }
        let entry: extern "C" fn() = unsafe {
            core::mem::transmute::<usize, extern "C" fn()>(addr.entry_vaddr)
        };
        let tid = crate::cpu::scheduler::spawn_thread(addr.asid, entry);
        ServiceDomain { asid: addr.asid, tid, config_frame: addr.config_frame }
    }

    pub(super) fn test_el0_raft() {
        crate::logln!("[raft] two-node boot test");

        let ns = supervisor::spawn_name_service(NS_ELF, NS_INTERFACE, 1, 8);
        crate::logln!("[raft] ns ok asid={} tid={}", ns.domain.asid, ns.domain.tid);

        let _r1 = spawn_raft_node(
            &[b'r' as u32, b'1' as u32, b'r' as u32, b'2' as u32],
            &ns,
        );
        crate::logln!("[raft] r1 spawned");

        let _r2 = spawn_raft_node(
            &[b'r' as u32, b'2' as u32, b'r' as u32, b'1' as u32],
            &ns,
        );
        crate::logln!("[raft] r2 spawned");

        crate::logln!("[raft] PASS boot");
    }
}

#[cfg(target_arch = "aarch64")]
pub fn test_el0_raft() {
    inner::test_el0_raft();
}

#[cfg(not(target_arch = "aarch64"))]
pub fn test_el0_raft() {}
