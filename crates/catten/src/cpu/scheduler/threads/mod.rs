pub mod waker;

use alloc::{
    boxed::Box,
    collections::BTreeMap,
    sync::{
        Arc,
        Weak,
    },
    vec::Vec,
};
use core::{
    mem::offset_of,
    sync::atomic::{
        AtomicU64,
        Ordering,
    },
};

use spin::LazyLock;

use crate::{
    cpu::{
        isa::lp::{
            LpId,
            thread_context::ThreadContext,
        },
        multiprocessor::spin::{
            mutex::Mutex,
            rwlock::RwLock,
        },
        scheduler::threads::waker::Waker,
    },
    klib::{
        collections::id_table::IdTable,
        observer::{
            Observable,
            Observer,
        },
    },
    memory::{
        AddressSpaceId,
        KERNEL_ASID,
    },
};

pub static MASTER_THREAD_TABLE: LazyLock<RwLock<ThreadTable>> =
    LazyLock::new(|| RwLock::new(ThreadTable::new()));
pub type ThreadTable = IdTable<Thread>;
pub type ThreadId = usize;
pub type ThreadGeneration = u64;

static NEXT_THREAD_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Threads that have exited but are awaiting reaping, keyed by the logical
/// processor on which they last executed. A thread cannot free its own kernel
/// stack (in `ThreadContext::drop`) while it is still executing on it, so
/// `abort` stages the dying thread here instead of dropping it.
///
/// The list is **per-LP** on purpose: a thread is reaped only by the LP it died
/// on, and only from [`reap_dead_threads`], which runs in `cond_yield_lp`
/// *after* the `switch_ctx` that leaves the dying thread's stack. This
/// guarantees the dying thread is no longer executing anywhere before its stack
/// is freed. A shared, cross-LP list would let one LP free a stack that a thread
/// on another LP has not yet switched off — a use-after-free that manifests as a
/// translation fault on the next timer-IRQ return.
pub static DEAD_THREADS: LazyLock<RwLock<BTreeMap<LpId, Vec<Thread>>>> =
    LazyLock::new(|| RwLock::new(BTreeMap::new()));

/// Stage a thread that has stopped being scheduled on `lp` for reaping by that
/// same LP. The thread's stack is not freed until [`reap_dead_threads`] runs on
/// `lp` after a context switch away from it.
pub fn stage_dead_thread(lp: LpId, thread: Thread) {
    DEAD_THREADS.write().entry(lp).or_default().push(thread);
}

/// Drops any threads awaiting reaping on the *current* LP, freeing their stacks.
/// MUST be called from a thread other than the one being reaped (e.g. from
/// `cond_yield_lp` after the context switch away from the dying thread). Safe to
/// call when there is nothing to reap.
pub fn reap_dead_threads() {
    let lp = crate::cpu::isa::lp::ops::get_lp_id();
    // Move this LP's dead threads out under the lock, then drop them after
    // releasing it so their `Drop` (which frees stacks via the frame allocator)
    // does not run while holding the DEAD_THREADS lock.
    let dead: Vec<Thread> = {
        let mut guard = DEAD_THREADS.write();
        match guard.get_mut(&lp) {
            Some(threads) if !threads.is_empty() => core::mem::take(threads),
            _ => return,
        }
    };

    // `switch_ctx` is coroutine-like: it returns through the incoming
    // context's older `cond_yield_lp` invocation.  Consequently an LP-local
    // reap list can contain that very incoming context (for example after a
    // remote abort/re-admission race).  LP identity alone is therefore not a
    // sufficient proof that a stack is no longer live.  Never unmap the stack
    // containing the instruction stream's current SP; leave it for the next
    // switch on this LP.
    let current_sp = current_stack_pointer();
    let (deferred, reclaimable): (Vec<_>, Vec<_>) =
        dead.into_iter().partition(|thread| thread.context.kernel_stack_contains(current_sp));
    if !deferred.is_empty() {
        DEAD_THREADS.write().entry(lp).or_default().extend(deferred);
    }
    drop(reclaimable);
}

#[cfg(target_arch = "aarch64")]
fn current_stack_pointer() -> usize {
    let sp: usize;
    unsafe {
        core::arch::asm!("mov {}, sp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    sp
}

#[cfg(target_arch = "x86_64")]
fn current_stack_pointer() -> usize {
    let sp: usize;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) sp, options(nomem, nostack, preserves_flags));
    }
    sp
}

pub type ThreadCount = usize;

#[derive(Debug)]
pub enum ThreadState {
    Running(LpId),
    Ready(LpId),
    NeedsLpAssignment,
    Blocked(Arc<Waker>),
}

#[derive(Debug)]
pub struct Thread {
    /// Boxed so the context (and therefore its `saved_sp`/`rsp_cpl0` field) has
    /// a **stable heap address**. `cond_yield_lp` captures a raw pointer to that
    /// field under lock and dereferences it lock-free inside `switch_ctx`; if the
    /// context lived inline in `MASTER_THREAD_TABLE`'s backing `Vec`, a
    /// concurrent `spawn_thread` on another LP that grows the `Vec` would move
    /// every `Thread` and leave that pointer dangling — corrupting the saved
    /// stack pointer of the thread being switched. The `Box` keeps the context
    /// pinned regardless of table reallocation.
    pub context: Box<ThreadContext>,
    pub asid: AddressSpaceId,
    /// Distinguishes successive occupants of a reusable [`ThreadId`] slot.
    pub generation: ThreadGeneration,
    pub state: ThreadState,
    /// The LP this thread prefers to run on, assigned at spawn time.
    /// Re-admission via `submit_woken_thread` and `submit_ready_thread`
    /// try this LP first rather than scanning for the least-loaded one,
    /// giving the thread cache affinity and keeping its timer events on
    /// the same LP's queue.
    pub affinity_lp: Option<LpId>,
    /// A hard placement constraint for work whose semantics are LP-local
    /// (notably shard workers). Unlike soft affinity, rebalancing must never
    /// change this value.
    pub pinned_lp: Option<LpId>,
    /// Explicit permission for Ready-state load migration. Hard pinning still
    /// takes precedence. Set false for work with unmodelled LP-local state.
    pub migration_safe: bool,
    exit_observers: Mutex<Vec<Weak<dyn Observer>>>,
}

pub const THREAD_CTX_OFFSET: usize = offset_of!(Thread, context);

impl Thread {
    pub fn new(asid: AddressSpaceId, entry_point: extern "C" fn()) -> Self {
        Thread {
            context: Box::new(
                if asid != KERNEL_ASID {
                    ThreadContext::create_user_thread_context(asid, entry_point)
                        .expect("Error creating user thread context")
                } else {
                    ThreadContext::create_kernel_thread_context(entry_point)
                        .expect("Error creating kernel thread context")
                },
            ),
            asid,
            generation: NEXT_THREAD_GENERATION.fetch_add(1, Ordering::Relaxed),
            state: ThreadState::NeedsLpAssignment,
            affinity_lp: None,
            pinned_lp: None,
            migration_safe: false,
            exit_observers: Mutex::new(Vec::new()),
        }
    }

    pub fn is_user_thread(&self) -> bool {
        self.asid != KERNEL_ASID
    }
}

/// The `Observable` trait is implemented for `Thread` to notify observers when a thread exits and
/// is dropped. This can be used to implement thread joining like functionality but also crucially
/// for monitoring when work started from a system call, nearly all of which are asynchronous in
/// Catten, finishes executing so userspace can be notified if requested. In any case the
/// completion capability returned from the system call would be registered as an observer of the
/// thread that is executing the work whose completion it represents so that userspace software can
/// monitor it in real time using the same mechanism the kernel would use.
impl Observable for Thread {
    fn register_observer(&self, observer: Weak<dyn Observer>) {
        self.exit_observers.lock().push(observer);
    }
}

impl Drop for Thread {
    fn drop(&mut self) {
        for observer in self.exit_observers.lock().iter() {
            if let Some(observer) = observer.upgrade() {
                observer.notify();
            }
        }
    }
}
