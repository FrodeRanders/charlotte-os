//! Self-test: single-node network appliance (DHCP lease + HTTP keyhole).
//!
//! Spawns the `tcpip` service in DHCP mode (it acquires its address from the
//! network's DHCP server) and polls the service's `socket::OP_STATUS` snapshot
//! until the reported IPv4 address is non-zero. Once the lease lands it also
//! spawns the `httpd` service, which listens on TCP port 80 through `tcpip`
//! and serves a hardcoded state keyhole — reachable from the host on QEMU
//! SLIRP or VMware NAT. Unlike the cross-node `tcpip_net_test`, this runs in a
//! single guest and only exercises the lease + listen path.
//!
//! Reaching the tcpip serving stage is *not* sufficient: the service enters
//! its serving loop before the DHCP exchange completes, so the verifier must
//! observe the interface address itself rather than the stage word.
//!
//! Requires the net driver and the frouter, both spawned by the net self-test
//! under `virtio_net_test` (which `dhcp_test` implies).
mod inner {
    use alloc::vec::Vec;

    use crate::{
        ipc::{self, ConnectionRights},
        logln,
        service::{
            bootstrap::{
                ManifestEntry,
                ManifestValue,
            },
            supervisor::{
                self,
                NameServiceHandle,
                ServiceDomain,
            },
        },
    };

    const DHCP_KEY: u64 = charlotte_launch::manifest_key(b"dhcp");

    // "tcpip" packed LE (catten_services::socket::NAME); the kernel crate does
    // not link catten_services, so mirror the packed short name here.
    const TCPIP_NAME: u64 = 0x0070_6970_6374;

    // Name-service opcodes (catten_services::ns).
    const NS_OP_LOOKUP: u32 = 2;

    // socket protocol (catten_services::socket) constants mirrored here.
    const SOCKET_OP_STATUS: u32 = 10;
    const STATUS_OFFSET_IP: usize = 0;
    const STATUS_OFFSET_MAGIC: usize = 4;
    const STATUS_MAGIC: u32 = 0x5443_5053;
    const STATUS_WORDS: usize = 5;

    static mut DHCP_NS: Option<NameServiceHandle> = None;

    fn spawn_binary(
        image: &[u8],
        ns: &NameServiceHandle,
        manifest: &[ManifestEntry<'_>],
    ) -> ServiceDomain {
        let addr = crate::service::loader::load_domain(image);
        let conn = crate::ipc::connection_delegate(
            ns.domain.asid,
            ns.endpoint_cap,
            addr.asid,
            ConnectionRights::CALL,
        )
        .expect("dhcp spawn conn delegate");
        crate::service::bootstrap::write_bootstrap_cap(addr.config_frame, conn);
        crate::service::bootstrap::write_manifest(addr.config_frame, manifest);
        let entry: extern "C" fn() =
            unsafe { core::mem::transmute::<usize, extern "C" fn()>(addr.entry_vaddr) };
        let tid = crate::cpu::scheduler::spawn_thread(addr.asid, entry);
        let generation = crate::cpu::scheduler::threads::MASTER_THREAD_TABLE
            .read()
            .get(tid)
            .expect("dhcp thread missing after spawn")
            .generation;
        ServiceDomain {
            asid: addr.asid,
            address_space: addr.address_space,
            tid,
            generation,
            config_frame: addr.config_frame,
            status_frame: addr.status_frame,
        }
    }

    fn kernel_ns_connection(ns: &NameServiceHandle) -> u64 {
        crate::ipc::connection_delegate(
            ns.domain.asid,
            ns.endpoint_cap,
            crate::memory::KERNEL_ASID,
            crate::ipc::ConnectionRights::CALL,
        )
        .expect("[dhcp] kernel name-service connection")
    }

    fn lookup_service(kernel_ns: u64, name: u64) -> Option<u64> {
        let call =
            ipc::scalar_call(crate::memory::KERNEL_ASID, kernel_ns, NS_OP_LOOKUP, name).ok()?;
        if ipc::wait_reply(crate::memory::KERNEL_ASID, call).is_err() {
            let _ = ipc::close_cap(crate::memory::KERNEL_ASID, call);
            return None;
        }
        let connection = ipc::poll_reply(crate::memory::KERNEL_ASID, call)
            .ok()
            .flatten()
            .and_then(|reply| reply.cap)
            .filter(|cap| *cap != 0);
        let _ = ipc::close_cap(crate::memory::KERNEL_ASID, call);
        connection
    }

    /// Query `socket::OP_STATUS` and decode the packed u32 word snapshot.
    fn status_snapshot(tcpip_conn: u64) -> Option<Vec<u32>> {
        let call =
            ipc::scalar_call(crate::memory::KERNEL_ASID, tcpip_conn, SOCKET_OP_STATUS, 0).ok()?;
        if ipc::wait_reply(crate::memory::KERNEL_ASID, call).is_err() {
            let _ = ipc::close_cap(crate::memory::KERNEL_ASID, call);
            return None;
        }
        let memory = ipc::poll_reply(crate::memory::KERNEL_ASID, call)
            .ok()
            .flatten()
            .and_then(|reply| reply.memory);
        let _ = ipc::close_cap(crate::memory::KERNEL_ASID, call);
        let memory = memory?;
        let bytes = crate::memory::object::snapshot_bytes(
            crate::memory::KERNEL_ASID,
            memory,
            STATUS_WORDS * 4,
        )
        .ok()?;
        let _ = crate::memory::object::close_cap(crate::memory::KERNEL_ASID, memory);
        if bytes.len() < STATUS_WORDS * 4 {
            return None;
        }
        let mut words = Vec::with_capacity(STATUS_WORDS);
        for index in 0..STATUS_WORDS {
            let word = u32::from_le_bytes([
                bytes[index * 4],
                bytes[index * 4 + 1],
                bytes[index * 4 + 2],
                bytes[index * 4 + 3],
            ]);
            words.push(word);
        }
        Some(words)
    }

    pub fn test_el0_dhcp() {
        logln!("Testing EL0 single-node network (DHCP lease + HTTP keyhole)...");

        let name_service = supervisor::node_name_service();
        let ns_asid = name_service.domain.asid;
        logln!("[dhcp] using node name service (asid={})", ns_asid);

        unsafe { DHCP_NS = Some(name_service) };

        let _vtid = crate::self_test::results::spawn_verifier(
            crate::self_test::results::TestId::Dhcp,
            verify_el0_dhcp,
        );
        logln!("[dhcp] verifier deferred (waits for a DHCP lease)");
    }

    extern "C" fn verify_el0_dhcp() {
        use crate::cpu::scheduler::yield_lp;

        let ns = unsafe { DHCP_NS.as_ref() }.expect("[dhcp] test state missing");

        let tcpip_manifest = [ManifestEntry {
            key: DHCP_KEY,
            flags: 0,
            value: ManifestValue::Bytes(b"1"),
        }];
        let tcpip = spawn_binary(
            crate::service::store::service_elf(b"tcpip").expect("[el0_dhcp] tcpip.elf"),
            ns,
            &tcpip_manifest,
        );
        logln!("[dhcp] tcpip spawned (asid={})", tcpip.asid);
        let tcpip_cfg: *const u32 = {
            let base: *mut u8 = tcpip.status_frame.into();
            base as *const u32
        };

        // The service enters its serving loop (stage 6) before the DHCP
        // exchange completes, so first wait for it to answer OP_STATUS, then
        // poll the reported address until the lease lands.
        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        while unsafe { core::ptr::read_volatile(tcpip_cfg) } < 6 {
            deadline.assert_pending("EL0 dhcp tcpip service startup");
            yield_lp();
        }
        logln!("[dhcp] tcpip reached the serving stage; polling for a lease.");

        let kernel_ns = kernel_ns_connection(ns);
        let tcpip_conn = lookup_service(kernel_ns, TCPIP_NAME).unwrap_or_else(|| {
            logln!("[dhcp] FAILURE: tcpip service not registered in the name service.");
            crate::self_test::results::fail(crate::self_test::results::TestId::Dhcp);
            0
        });
        if tcpip_conn == 0 {
            return;
        }

        let deadline = crate::self_test::results::Deadline::after_millis(60_000);
        let mut spins: u64 = 0;
        let (mut ip_word, mut magic) = (0u32, 0u32);
        loop {
            if let Some(words) = status_snapshot(tcpip_conn) {
                ip_word = words[STATUS_OFFSET_IP];
                magic = words[STATUS_OFFSET_MAGIC];
                if magic == STATUS_MAGIC && ip_word != 0 {
                    break;
                }
            }
            spins += 1;
            if spins.is_multiple_of(200_000) {
                logln!(
                    "[dhcp] waiting for lease: ip={}.{}.{}.{} magic={:#010x}",
                    (ip_word >> 24) & 0xff,
                    (ip_word >> 16) & 0xff,
                    (ip_word >> 8) & 0xff,
                    ip_word & 0xff,
                    magic
                );
            }
            deadline.assert_pending("EL0 dhcp lease acquisition");
            yield_lp();
        }

        let octets = [
            ((ip_word >> 24) & 0xff) as u8,
            ((ip_word >> 16) & 0xff) as u8,
            ((ip_word >> 8) & 0xff) as u8,
            (ip_word & 0xff) as u8,
        ];
        logln!(
            "[dhcp] SUCCESS: DHCP assigned {}.{}.{}.{}",
            octets[0],
            octets[1],
            octets[2],
            octets[3]
        );

        // Bring up the HTTP keyhole so the host can reach the appliance over
        // the freshly acquired address.
        let httpd = spawn_binary(
            crate::service::store::service_elf(b"httpd").expect("[el0_dhcp] httpd.elf"),
            ns,
            &[],
        );
        logln!("[dhcp] httpd spawned (asid={})", httpd.asid);
        let httpd_cfg: *const u32 = {
            let base: *mut u8 = httpd.status_frame.into();
            base as *const u32
        };
        let deadline = crate::self_test::results::Deadline::after_millis(30_000);
        let mut spins: u64 = 0;
        while unsafe { core::ptr::read_volatile(httpd_cfg) } < 6 {
            spins += 1;
            if spins.is_multiple_of(200_000) {
                let stage = unsafe { core::ptr::read_volatile(httpd_cfg) };
                let error = unsafe { core::ptr::read_volatile(httpd_cfg.add(2)) };
                logln!("[dhcp] waiting: httpd stage={} error={:#x}", stage, error);
            }
            deadline.assert_pending("EL0 dhcp httpd listening");
            yield_lp();
        }
        logln!(
            "[dhcp] httpd listening on port 80; try: curl http://{}.{}.{}.{}/",
            octets[0],
            octets[1],
            octets[2],
            octets[3]
        );

        crate::self_test::results::pass(crate::self_test::results::TestId::Dhcp);
    }
}

pub use inner::test_el0_dhcp;
