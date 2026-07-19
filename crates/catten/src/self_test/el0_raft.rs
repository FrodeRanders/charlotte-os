#[cfg(target_arch = "aarch64")]
mod inner {
    use crate::{
        ipc::ConnectionRights,
        service::supervisor::{
            self,
            NameServiceHandle,
            ServiceDomain,
        },
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
    const MAX_SPINS: u64 = 80_000_000;

    static mut RAFT_NS: Option<NameServiceHandle> = None;

    fn spawn_raft_node(args: &[u32], ns_handle: &NameServiceHandle) -> ServiceDomain {
        let addr = crate::service::loader::load_domain(RAFT_ELF);
        let conn = crate::ipc::connection_delegate(
            ns_handle.domain.asid,
            ns_handle.endpoint_cap,
            addr.asid,
            ConnectionRights::CALL,
        )
        .expect("raft conn delegate");
        crate::service::bootstrap::write_bootstrap_cap(addr.config_frame, conn);
        let base: *mut u8 = addr.config_frame.into();
        unsafe {
            core::ptr::write_volatile(base.add(ARGC_OFFSET) as *mut usize, args.len());
            for (i, &a) in args.iter().enumerate() {
                core::ptr::write_volatile(base.add(ARGS_OFFSET + i * 4) as *mut u32, a);
            }
        }
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(addr.entry_vaddr) };
        let tid = crate::cpu::scheduler::spawn_thread(addr.asid, entry);
        let generation = crate::cpu::scheduler::threads::MASTER_THREAD_TABLE
            .read()
            .get(tid)
            .expect("raft thread missing after spawn")
            .generation;
        ServiceDomain {
            asid: addr.asid,
            tid,
            generation,
            config_frame: addr.config_frame,
        }
    }

    pub(super) fn test_el0_raft() {
        crate::logln!("[raft] two-node boot test");

        let ns = supervisor::spawn_name_service(NS_ELF, NS_INTERFACE, 1, 8);
        crate::logln!("[raft] ns ok asid={} tid={}", ns.domain.asid, ns.domain.tid);

        unsafe {
            RAFT_NS = Some(ns);
        }
        let _verifier =
            crate::cpu::scheduler::spawn_thread(crate::memory::KERNEL_ASID, verify_raft_cluster);
        crate::logln!("[raft] verifier deferred");
    }

    extern "C" fn verify_raft_cluster() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { RAFT_NS }.expect("[raft] verifier name service missing");
        let ns_stage: *const u32 = {
            let base: *mut u8 = ns.domain.config_frame.into();
            base as *const u32
        };
        while unsafe { core::ptr::read_volatile(ns_stage) } < 2 {
            yield_lp();
        }

        // Do not start clients that synchronously register until the name
        // service has entered its receive loop. During early boot the
        // scheduler is cooperative, so starting all three together can let a
        // polling client starve the server it is waiting for.
        let r1_domain = spawn_raft_node(&[b'r' as u32, b'1' as u32, b'r' as u32, b'2' as u32], &ns);
        let r1_stage: *const u32 = {
            let base: *mut u8 = r1_domain.config_frame.into();
            base as *const u32
        };
        while unsafe { core::ptr::read_volatile(r1_stage) } < 6 {
            yield_lp();
        }
        let r2_domain = spawn_raft_node(&[b'r' as u32, b'2' as u32, b'r' as u32, b'1' as u32], &ns);
        crate::logln!("[raft] nodes spawned in registration order after name service became ready");

        let r1_config = r1_domain.config_frame;
        let r2_config = r2_domain.config_frame;
        let r1: *const u32 = {
            let base: *mut u8 = r1_config.into();
            base as *const u32
        };
        let r2: *const u32 = {
            let base: *mut u8 = r2_config.into();
            base as *const u32
        };

        crate::logln!(
            "[raft] verifier running: stages {}/{}",
            unsafe { core::ptr::read_volatile(r1) },
            unsafe { core::ptr::read_volatile(r2) }
        );

        let mut spins = 0u64;
        loop {
            let s1 = unsafe { core::ptr::read_volatile(r1.add(2)) };
            let s2 = unsafe { core::ptr::read_volatile(r2.add(2)) };
            if (s1 == 3 && s2 == 1) || (s1 == 1 && s2 == 3) {
                // Require the elected leader to have processed at least one
                // asynchronous RPC completion, proving replies flowed through
                // CharlotteTransport into RaftNode.
                let c1 = unsafe { core::ptr::read_volatile(r1.add(4)) };
                let c2 = unsafe { core::ptr::read_volatile(r2.add(4)) };
                if c1 + c2 > 0 {
                    crate::logln!(
                        "[raft] SUCCESS: two-node cluster elected one leader (states {}/{}, \
                         completions {}/{}).",
                        s1,
                        s2,
                        c1,
                        c2
                    );
                    break;
                }
            }
            spins += 1;
            if spins % 10_000 == 0 {
                let stage1 = unsafe { core::ptr::read_volatile(r1) };
                let stage2 = unsafe { core::ptr::read_volatile(r2) };
                crate::logln!("[raft] waiting: stages {}/{}, states {}/{}", stage1, stage2, s1, s2);
            }
            assert!(spins < MAX_SPINS, "[raft] FAILED to elect one leader");
            yield_lp();
        }

        loop {
            yield_lp();
        }
    }
}

#[cfg(target_arch = "aarch64")]
pub fn test_el0_raft() {
    inner::test_el0_raft();
}

#[cfg(not(target_arch = "aarch64"))]
pub fn test_el0_raft() {}
