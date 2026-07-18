#![allow(unused_assignments)]
#[cfg(target_arch = "aarch64")]
use crate::{
    ipc::{self, CapabilityId, ConnectionRights},
    service::supervisor::{self, NameServiceHandle, ServiceDomain},
};
use crate::logln;

#[cfg(target_arch = "aarch64")]
const NS_ELF: &[u8] = include_bytes!("ns.elf");
#[cfg(target_arch = "aarch64")]
const RAFT_ELF: &[u8] = include_bytes!("raft.elf");

#[cfg(target_arch = "aarch64")]
const fn packed_name(bytes: &[u8]) -> u64 {
    let mut packed = [0u8; 8];
    let mut i = 0;
    while i < bytes.len() && i < 8 {
        packed[i] = bytes[i];
        i += 1;
    }
    u64::from_le_bytes(packed)
}
#[cfg(target_arch = "aarch64")]
const NS_INTERFACE: u64 = packed_name(b"NAME");

#[cfg(target_arch = "aarch64")]
const _RAFT_INTERFACE: u64 = packed_name(b"RAFT");

const ARGC_OFFSET: usize = 24;
const ARGS_OFFSET: usize = 32;

const OP_LOOKUP: u32 = 2;
const OP_STATUS: u32 = 8;
const NAME_R1: u64 = packed_name(b"raft-r1");

const VERIFIER_ASID: usize = 0x9000;

#[cfg(target_arch = "aarch64")]
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

fn yield_lp() { crate::yield_lp(); }

fn wait_reply(call: CapabilityId, _label: &str) -> ipc::ReplyValue {
    let mut spins: u64 = 0;
    loop {
        match ipc::poll_reply(VERIFIER_ASID, call) {
            Ok(Some(val)) => return val,
            _ => {
                spins += 1;
                if spins >= 200 {
                    return ipc::ReplyValue { result: -1, cap: None, memory: None };
                }
                yield_lp();
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
pub fn test_el0_raft() {
    logln!("[raft] two-node leader election test");

    let ns = supervisor::spawn_name_service(NS_ELF, NS_INTERFACE, 1, 8);
    logln!("[raft] ns ok asid={} tid={}", ns.domain.asid, ns.domain.tid);
    let ns_endpoint = ns.endpoint_cap;

    let _r1 = spawn_raft_node(
        &[b'r' as u32, b'1' as u32, b'r' as u32, b'2' as u32],
        &ns,
    );
    logln!("[raft] r1 spawned");

    let _r2 = spawn_raft_node(
        &[b'r' as u32, b'2' as u32, b'r' as u32, b'1' as u32],
        &ns,
    );
    logln!("[raft] r2 spawned");

    let vconn = ipc::connection_delegate(
        ns.domain.asid, ns_endpoint, VERIFIER_ASID,
        ConnectionRights::CALL,
    ).expect("verifier connection");

    for _ in 0..5 {
        let l1 = ipc::scalar_call(VERIFIER_ASID, vconn, OP_LOOKUP, NAME_R1)
            .expect("lookup r1");
        let r1_reply = wait_reply(l1, "r1 lookup");
        if let Some(r1_conn) = r1_reply.cap {
            let s1 = ipc::scalar_call(VERIFIER_ASID, r1_conn, OP_STATUS, 0)
                .expect("status r1");
            let s1_reply = wait_reply(s1, "r1 status");
            let state1 = (s1_reply.result & 0xFF) as u32;
            logln!("[raft] r1 state");
            if state1 == 3 {
                logln!("[raft] PASS leader");
                return;
            }
        }
    }

    logln!("[raft] no leader (race in peer discovery?)");
}
