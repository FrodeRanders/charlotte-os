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
    static user_stack_growth_start: u8;
    static user_stack_growth_end: u8;
    static user_register_probe_start: u8;
    static user_register_probe_end: u8;
    #[cfg(target_arch = "x86_64")]
    static user_state_probe_start: u8;
    #[cfg(target_arch = "x86_64")]
    static user_state_probe_end: u8;
}

/// Native fixture: force kernel entry paths to overwrite every caller-saved
/// vector register and the x87 state, so preservation cannot pass accidentally.
#[cfg(target_arch = "x86_64")]
#[unsafe(naked)]
pub(crate) extern "C" fn clobber_fp_state() {
    core::arch::naked_asm!(
        "pxor xmm0, xmm0",
        "pxor xmm1, xmm1",
        "pxor xmm2, xmm2",
        "pxor xmm3, xmm3",
        "pxor xmm4, xmm4",
        "pxor xmm5, xmm5",
        "pxor xmm6, xmm6",
        "pxor xmm7, xmm7",
        "pxor xmm8, xmm8",
        "pxor xmm9, xmm9",
        "pxor xmm10, xmm10",
        "pxor xmm11, xmm11",
        "pxor xmm12, xmm12",
        "pxor xmm13, xmm13",
        "pxor xmm14, xmm14",
        "pxor xmm15, xmm15",
        "fninit",
        "ret",
    );
}

/// The raw BSP/AP entries must recover even from the disabled-FP boot fixture
/// before generated Rust instructions or ordinary function calls can use SSE2.
#[cfg(target_arch = "x86_64")]
pub(crate) fn verify_boot_fp() {
    let (cr0, cr4): (u64, u64);
    let mut x87_control = 0u16;
    let mut mxcsr = 0u32;
    unsafe {
        core::arch::asm!(
            "mov {cr0}, cr0",
            "mov {cr4}, cr4",
            "fnstcw [{x87}]",
            "stmxcsr [{mxcsr}]",
            cr0 = out(reg) cr0,
            cr4 = out(reg) cr4,
            x87 = in(reg) &mut x87_control,
            mxcsr = in(reg) &mut mxcsr,
            options(nostack, preserves_flags),
        );
    }
    assert_eq!(cr0 & 0x2e, 0x22, "boot CR0 MP/NE with EM/TS clear");
    assert_eq!(cr4 & (0x600 | (1 << 18)), 0x600, "boot SSE enabled; OSXSAVE disabled");
    assert_eq!(x87_control, 0x037f);
    assert_eq!(mxcsr, 0x1f80);
    let value = core::hint::black_box(1.25f64);
    let doubled = core::hint::black_box(value + value);
    assert_eq!(doubled.to_bits(), 2.5f64.to_bits());
    crate::early_logln!("[x86 boot FP] normalized inherited state and SSE2 arithmetic passed");
}

#[cfg(target_arch = "x86_64")]
static TIMER_FP_CLOBBERS: [core::sync::atomic::AtomicU64;
    crate::cpu::scheduler::system_scheduler::MAX_TRACKED_LPS] =
    [const { core::sync::atomic::AtomicU64::new(0) };
        crate::cpu::scheduler::system_scheduler::MAX_TRACKED_LPS];

#[cfg(target_arch = "x86_64")]
pub(crate) fn note_timer_fp_clobber() {
    let lp = crate::cpu::isa::lp::ops::get_lp_id() as usize;
    TIMER_FP_CLOBBERS[lp].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

#[cfg(target_arch = "x86_64")]
fn verify_kernel_fp() {
    let flags: u64;
    unsafe { core::arch::asm!("pushfq", "pop {}", out(reg) flags, options(preserves_flags)) };
    assert_ne!(flags & (1 << 9), 0, "kernel FP fixture requires the caller's enabled IRQs");
    let lp = crate::cpu::isa::lp::ops::get_lp_id() as usize;
    let interval = *crate::cpu::isa::timers::tsc::TSC_FREQUENCY_HZ / 10;
    assert!(interval > 0);
    let mut observed = [0u64; 4];
    unsafe {
        // Bounded native register fixture: retain XMM15 for 100 ms without
        // calling Rust. The aligned atomic
        // counter loads inside that window prove this LP ran the timer's
        // deliberate clobber hook while the sentinel was live.
        core::arch::asm!(
            "pcmpeqd xmm15, xmm15",
            "lfence",
            "mov r10, [{counter}]",
            "rdtsc",
            "shl rdx, 32",
            "or rax, rdx",
            "mov r11, rax",
            "add r11, {interval}",
            "2:",
            "rdtsc",
            "shl rdx, 32",
            "or rax, rdx",
            "cmp rax, r11",
            "jb 2b",
            "lfence",
            "movdqu [{observed}], xmm15",
            "mov [{observed} + 16], r10",
            "mov rax, [{counter}]",
            "mov [{observed} + 24], rax",
            counter = in(reg) TIMER_FP_CLOBBERS[lp].as_ptr(),
            interval = in(reg) interval,
            observed = in(reg) observed.as_mut_ptr(),
            out("rax") _, out("rdx") _, out("r10") _, out("r11") _, out("xmm15") _,
            options(nostack),
        );
    }
    assert_eq!(&observed[..2], &[u64::MAX; 2], "kernel SIMD state across IRQ");
    assert!(observed[3].wrapping_sub(observed[2]) > 0, "no local timer clobber observed");
    assert_eq!(crate::cpu::isa::lp::ops::get_lp_id() as usize, lp);
    crate::logln!("[kernel FP] live XMM value survived measured local timer clobbers");
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
        Self::spawn_with(code, |_| {})
    }

    fn spawn_with(code: &[u8], before_publish: impl FnOnce(AddressSpaceHandle)) -> Self {
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
        before_publish(handle);
        let mut generation = 0;
        // The test ABI requires entry to name a mapped userspace address.
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
        let stack_progress = memory::thread_stack::retirement_progress();
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
                    if deadline.is_expired() {
                        // Exact retained-node outcome is copied under staging,
                        // then printed after its guard leaves. It grants no retry.
                        if let Some(snapshot) =
                            scheduler::threads::staged_retirement_snapshot(self.generation)
                        {
                            crate::logln!(
                                "[user isolation] staged thread generation={} root={:?} lp={:?} \
                                 started={} error={:?}",
                                self.generation,
                                snapshot.root,
                                snapshot.lp,
                                snapshot.started,
                                snapshot.error
                            );
                        }
                        #[cfg(target_arch = "x86_64")]
                        crate::logln!(
                            "[user isolation] global shootdown observations \
                             [success,busy,masked,exhausted,delivery,timeout]={:?}",
                            crate::cpu::isa::interrupts::fixed::ipis::shootdown_progress()
                        );
                        crate::logln!(
                            "[user isolation] retained stack lease asid={} generation={} \
                             user-retirement counters \
                             [started,released,identity,detach,invalidation,physical]: \
                             before={:?} after={:?}",
                            self.handle.id(),
                            self.handle.generation(),
                            stack_progress,
                            memory::thread_stack::retirement_progress()
                        );
                    }
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
    #[cfg(target_arch = "x86_64")]
    verify_kernel_fp();
    scheduler::system_scheduler::verify_executor_rejection();
    verify_abort_handoff();
    verify_concurrent_abort();
    verify_stack_growth();
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
        for offset in (160..416).step_by(8) {
            assert_eq!(state.word(offset), u64::MAX, "preserved XMM word {offset}");
        }
        assert_eq!(state.word(0) & 0xffff, 0x077f, "preserved x87 control");
        assert_eq!(state.word(24) as u32, 0x3f80, "preserved MXCSR");
        assert_eq!(state.word(832), u64::MAX);
        assert_eq!(state.word(840), u64::MAX);
        assert_eq!(state.word(848), 1f64.to_bits());
        assert_eq!(state.word(856), 0x123000);
        assert_eq!(state.word(864), 0x456000);
        state.finish();
        crate::logln!(concat!(
            "[user isolation] x86 x87/XMM0-15/MXCSR preserved across syscalls, timer IRQs ",
            "and blocking; fresh domains cleared"
        ));
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

fn verify_abort_handoff() {
    // First entry faults; the second entry spins in the same admitted root.
    // The sweep must request that peer and survive explicit scheduling before
    // requesting its own exit and completing its root operation.
    let mut instructions = [0u8; 132];
    #[cfg(target_arch = "x86_64")]
    {
        instructions[..4].copy_from_slice(&[0x31, 0xc0, 0x8b, 0]); // null read
        instructions[128..130].copy_from_slice(&[0xeb, 0xfe]); // jmp .
    }
    #[cfg(target_arch = "aarch64")]
    {
        instructions[..8].copy_from_slice(&[
            0x00, 0x00, 0x80, 0xd2, // mov x0, #0
            0x00, 0x00, 0x40, 0xf9, // ldr x0, [x0]
        ]);
        instructions[128..].copy_from_slice(&[0x00, 0x00, 0x00, 0x14]); // b .
    }
    let fixture = Fixture::spawn_with(&instructions, |handle| {
        memory::set_domain_limits(
            handle,
            memory::DomainLimits {
                user_stack_pages: 1,
                max_threads: 2,
            },
        )
        .unwrap();
        let mut generation = 0;
        // Documented fixture ABI: this is the second mapped EL0 entry address.
        let entry = unsafe { core::mem::transmute::<usize, extern "C" fn()>(0x20080) };
        let peer = scheduler::spawn_thread_after_publish(handle.id(), entry, |_, value| {
            generation = value;
        });
        scheduler::system_scheduler::arm_self_handoff(handle, (peer, generation));
    });
    let handle = fixture.handle;
    fixture.finish();
    scheduler::system_scheduler::finish_self_handoff(handle);
}

fn verify_concurrent_abort() {
    // Both entries wait on a mapped fixture word until both are published.
    // Otherwise the first fault can fence admission before its peer is ready.
    // Then both fault and rendezvous inside their admitted sweeps.
    #[cfg(target_arch = "x86_64")]
    let code = [
        0xb9, 0x00, 0x20, 0x01, 0x00, // mov ecx, 0x12000
        0x83, 0x39, 0x00, // cmp dword [rcx], 0
        0x75, 0xfb, // jne to cmp
        0x31, 0xc0, 0x8b, 0x00, // null read
    ];
    #[cfg(target_arch = "aarch64")]
    let code = [
        0x00, 0x00, 0x84, 0xd2, // mov x0, #0x2000
        0x20, 0x00, 0xa0, 0xf2, // movk x0, #1, lsl #16
        0x01, 0x00, 0x40, 0xb9, // ldr w1, [x0]
        0xe1, 0xff, 0xff, 0x35, // cbnz w1, to ldr
        0x00, 0x00, 0x80, 0xd2, // mov x0, #0
        0x00, 0x00, 0x40, 0xf9, // null read
    ];
    let fixture = Fixture::spawn_with(&code, |handle| {
        memory::set_domain_limits(
            handle,
            memory::DomainLimits {
                user_stack_pages: 1,
                max_threads: 2,
            },
        )
        .unwrap();
        scheduler::system_scheduler::arm_concurrent_handoff(handle);
        let entry = unsafe { core::mem::transmute::<usize, extern "C" fn()>(0x20000) };
        scheduler::spawn_thread(handle.id(), entry);
    });
    let handle = fixture.handle;
    // Fixture boundary: its exact root remains owned and maps this result frame
    // at 0x12000. A volatile release plus the architecture barrier opens entry.
    let gate: *mut u32 = fixture.result.into();
    unsafe { core::ptr::write_volatile(gate, 0) };
    core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
    fixture.finish();
    scheduler::system_scheduler::finish_concurrent_handoff(handle);
}

fn check_snapshot(snapshot: &Fixture) {
    #[cfg(target_arch = "aarch64")]
    for offset in (0..784).step_by(8) {
        assert_eq!(snapshot.word(offset), 0, "initial ARM register word {offset}");
    }
    #[cfg(target_arch = "x86_64")]
    {
        assert_eq!(snapshot.word(896), 8, "SysV initial stack alignment");
        assert_eq!(snapshot.word(0), 0x037f, "x87 default control and empty tags");
        assert_eq!(snapshot.word(8), 0, "x87 instruction pointer");
        assert_eq!(snapshot.word(16), 0, "x87 data pointer");
        assert_eq!(snapshot.word(24) as u32, 0x1f80, "MXCSR default");
        // The hardware MXCSR_MASK at 28 is intentionally implementation-defined.
        for offset in (32..512).chain(512..632).step_by(8) {
            assert_eq!(snapshot.word(offset), 0, "initial x86 register word {offset}");
        }
    }
    #[cfg(target_arch = "x86_64")]
    {
        assert_eq!(snapshot.word(856), 0, "initial FS base");
        assert_eq!(snapshot.word(864), 0, "initial GS base");
    }
}

fn verify_stack_growth() {
    for mode in 0..3 {
        let fixture = Fixture::spawn_with(
            unsafe { code(&raw const user_stack_growth_start, &raw const user_stack_growth_end) },
            |root| {
                memory::set_domain_limits(
                    root,
                    memory::DomainLimits {
                        user_stack_pages: 4,
                        max_threads: 1,
                    },
                )
                .unwrap();
                if mode == 2 {
                    let mut table = ADDRESS_SPACE_TABLE.lock();
                    let space = table.get_mut(root.id()).unwrap();
                    let mut backing = PreparingUserBacking::new(space, Kind::Image).unwrap();
                    backing.fill(|bytes| bytes.fill(0x5a));
                    backing
                        .map_with(
                            VAddr::from(charlotte_launch::user_address::STACK_BASE + 2 * 4096),
                            PageType::UserData,
                            |space, mapping| space.map_existing_page(mapping).is_ok(),
                        )
                        .unwrap();
                }
                scheduler::threads::arm_stack_growth(root, mode);
            },
        );
        let root = fixture.handle;
        if mode == 0 {
            fixture.wait_for(0xface);
            {
                let table = scheduler::threads::MASTER_THREAD_TABLE.read();
                let thread = table.get(fixture.tid).unwrap();
                assert_eq!(thread.generation, fixture.generation);
                assert_eq!(thread.address_space, Some(root));
                assert_eq!(thread.context.user_stack_committed_pages(), 3);
                assert_eq!(
                    thread.operation_executor_lp.load(core::sync::atomic::Ordering::Acquire),
                    usize::MAX
                );
            }
            // Existing bounded EL0 fixture gate; no production wait polls it.
            let bytes: *mut u8 = fixture.result.into();
            unsafe {
                core::ptr::write_volatile(bytes.add(808).cast::<u32>(), 0);
            }
            core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);
        }
        fixture.finish();
        scheduler::threads::verify_stack_growth(root);
    }
    crate::logln!(
        "[user isolation] real EL0 stack growth, deferred-abort handback and foreign-leaf \
         rollback passed"
    );
}
