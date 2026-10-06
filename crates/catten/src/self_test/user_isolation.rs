//! Execute first-entry snapshots and real user faults under the running scheduler.
use crate::{
    cpu::{
        isa::interface::memory::AddressSpaceInterface,
        scheduler,
    },
    memory::{
        self,
        ADDRESS_SPACE_TABLE,
        AddressSpaceHandle,
        PAddr,
        VAddr,
        backing_budget::Kind,
        linear::PageType,
        preparation::PreparingUserBacking,
    },
};

#[cfg(target_arch = "aarch64")]
core::arch::global_asm!(include_str!("user_entry_aarch64.asm"));
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(include_str!("user_entry_x86_64.asm"));
unsafe extern "C" {
    static user_register_probe_start: u8;
    static user_register_probe_end: u8;
    #[cfg(target_arch = "x86_64")]
    static user_state_probe_start: u8;
    #[cfg(target_arch = "x86_64")]
    static user_state_probe_end: u8;
}

// Kernel fixture namespace owner. Failed/abandoned verification retains the
// published root; only explicit finish retires physically quiescent backing.
struct Fixture {
    handle: AddressSpaceHandle,
    result: PAddr,
    tid: usize,
    generation: u64,
}

impl Fixture {
    fn spawn(code: &[u8]) -> Self {
        assert!(!code.is_empty() && code.len() <= 4096);
        let handle =
            memory::register_user_address_space(memory::AddressSpace::try_new_user().unwrap())
                .unwrap();
        memory::set_domain_limits(
            handle,
            memory::DomainLimits {
                user_stack_pages: 1,
                max_threads: 1,
            },
        )
        .unwrap();
        crate::completion::open_address_space(handle.id(), 16);
        let result = {
            let mut table = ADDRESS_SPACE_TABLE.lock();
            let space = table.get_mut(handle.id()).unwrap();
            let mut backing = PreparingUserBacking::new(space, Kind::Image).unwrap();
            backing.fill(|bytes| bytes[..code.len()].copy_from_slice(code));
            backing
                .map_with(VAddr::from(0x20000usize), PageType::UserCode, |space, mapping| {
                    space.map_existing_page(mapping).is_ok()
                })
                .unwrap();
            let mut backing = PreparingUserBacking::new(space, Kind::Image).unwrap();
            backing.fill(|bytes| bytes.fill(0xa5));
            backing
                .map_with(VAddr::from(0x12000usize), PageType::UserData, |space, mapping| {
                    space.map_existing_page(mapping).is_ok()
                })
                .unwrap()
        };
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("dsb ishst", "ic ialluis", "dsb ish", "isb", options(nomem, nostack));
        }
        let mut generation = 0;
        // Documented test ABI: entry is a mapped user address, not a kernel function.
        let entry = unsafe { core::mem::transmute::<usize, extern "C" fn()>(0x20000) };
        crate::logln!(
            "[user isolation] launching {} bytes: first={:02x?} asid={}",
            code.len(),
            &code[..code.len().min(8)],
            handle.id()
        );
        let tid = scheduler::spawn_thread_after_publish(handle.id(), entry, |_, value| {
            generation = value
        });
        Self {
            handle,
            result,
            tid,
            generation,
        }
    }

    fn word(&self, offset: usize) -> u64 {
        let base: *const u8 = self.result.into();
        unsafe { core::ptr::read_volatile(base.add(offset).cast()) }
    }

    fn wait_for(&self, sentinel: u64) {
        self.wait_for_at(800, sentinel);
    }

    fn wait_for_at(&self, offset: usize, sentinel: u64) {
        let deadline = super::results::Deadline::after_millis(10_000);
        while self.word(offset) != sentinel {
            deadline.assert_pending("user register-state probe");
            scheduler::yield_lp();
        }
    }

    fn finish(self) {
        let deadline = super::results::Deadline::after_millis(10_000);
        loop {
            let alive = scheduler::threads::MASTER_THREAD_TABLE
                .read()
                .get(self.tid)
                .is_ok_and(|thread| thread.generation == self.generation);
            if !alive {
                break;
            }
            deadline.assert_pending("faulted user thread retirement");
            scheduler::yield_lp();
        }
        loop {
            match memory::close_user_address_space_handle(self.handle) {
                Ok(()) => break,
                Err(memory::AddressSpaceCloseError::OperationsInFlight) => {
                    deadline.assert_pending("user stack retirement lease");
                    scheduler::yield_lp();
                }
                result => result.expect("user isolation fixture teardown"),
            }
        }
    }
}

unsafe fn code(start: *const u8, end: *const u8) -> &'static [u8] {
    // Linker/assembly boundary: these labels bound one static code payload.
    let len = end.addr().checked_sub(start.addr()).expect("ordered assembly labels");
    unsafe { core::slice::from_raw_parts(start, len) }
}

pub(crate) fn verify() {
    let snapshot = Fixture::spawn(unsafe {
        code(&raw const user_register_probe_start, &raw const user_register_probe_end)
    });
    snapshot.wait_for(0xdead);
    check_snapshot(&snapshot);
    snapshot.finish();
    crate::logln!("[user isolation] first-entry GPR and FP/SIMD snapshot passed");

    #[cfg(target_arch = "x86_64")]
    {
        crate::cpu::isa::interrupts::fixed::exceptions::test_fault_origin();
        let state = Fixture::spawn(unsafe {
            code(&raw const user_state_probe_start, &raw const user_state_probe_end)
        });
        state.wait_for_at(792, 0xbeef);
        let fresh = Fixture::spawn(unsafe {
            code(&raw const user_register_probe_start, &raw const user_register_probe_end)
        });
        fresh.wait_for(0xdead);
        check_snapshot(&fresh);
        fresh.finish();
        state.wait_for(0xdead);
        assert_eq!(state.word(832), u64::MAX);
        assert_eq!(state.word(840), u64::MAX);
        assert_eq!(state.word(848), 1f64.to_bits());
        assert_eq!(state.word(856), 0x123000);
        assert_eq!(state.word(864), 0x456000);
        state.finish();
        for (name, instructions) in [
            ("invalid opcode", &[0x0f, 0x0b][..]),
            ("divide by zero", &[0x31, 0xc9, 0xb8, 1, 0, 0, 0, 0x31, 0xd2, 0xf7, 0xf1][..]),
            ("general protection", &[0xfa][..]), // user CLI
            ("unmapped data", &[0x31, 0xc0, 0x8b, 0][..]),
            ("non-executable data", &[0xb8, 0, 0x20, 1, 0, 0xff, 0xe0][..]),
            ("stack growth rejection", &[0xb8, 0, 0xf0, 0xff, 0, 0x8b, 0][..]),
        ] {
            Fixture::spawn(instructions).finish();
            crate::logln!(
                "[user isolation] {name} contained; verifier and other domains remain live"
            );
        }
    }
}

fn check_snapshot(snapshot: &Fixture) {
    #[cfg(target_arch = "aarch64")]
    for offset in (0..784).step_by(8) {
        assert_eq!(snapshot.word(offset), 0, "initial ARM register word {offset}");
    }
    #[cfg(target_arch = "x86_64")]
    {
        assert_eq!(snapshot.word(0), 0x037f, "x87 default control and empty tags");
        assert_eq!(snapshot.word(8), 0, "x87 instruction pointer");
        assert_eq!(snapshot.word(16), 0, "x87 data pointer");
        assert_eq!(snapshot.word(24) as u32, 0x1f80, "MXCSR default");
        // The hardware MXCSR_MASK at 28 is intentionally implementation-defined.
        for offset in (32..512).chain(520..640).step_by(8) {
            assert_eq!(snapshot.word(offset), 0, "initial x86 register word {offset}");
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        assert_eq!(snapshot.word(856), 0, "initial FS base");
        assert_eq!(snapshot.word(864), 0, "initial GS base");
    }
}
