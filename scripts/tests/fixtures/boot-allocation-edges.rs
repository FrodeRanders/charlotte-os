#![allow(dead_code, non_camel_case_types, unused_imports)]
extern crate alloc;
extern crate self as flanterm_bindings;
extern crate self as spin;
extern crate self as limine;
use core::cell::Cell;
use core::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::alloc::{GlobalAlloc, Layout, System};
pub use std::sync::LazyLock;
pub struct RwLock<T>(std::sync::RwLock<T>);
impl<T> RwLock<T> {
    pub fn new(value: T) -> Self { Self(std::sync::RwLock::new(value)) }
    pub fn read(&self) -> std::sync::RwLockReadGuard<'_, T> { self.0.read().unwrap() }
}
#[macro_export] macro_rules! early_logln { ($($tokens:tt)*) => {}; }
#[macro_export] macro_rules! logln { ($($tokens:tt)*) => {}; }
pub mod mp { pub const MP_FLAG_X2APIC: u32 = 1; }
pub fn ap_main() {}
mod cpu {
    pub mod scheduler { pub mod system_scheduler { pub const MAX_TRACKED_LPS: usize = 256; } }
    pub mod isa { pub mod lp {
        pub type LpId = u32;
        pub mod ops {
            pub fn store_lp_id(_: u32) {} pub fn mpidr() -> u64 { 0 }
            pub fn log_mpidr() {} pub fn get_lic_id() -> u32 { 0 }
            pub fn get_lp_id() -> u32 { 0 }
        }
    }}
    pub mod multiprocessor {
        pub mod spin { pub mod mutex {
            pub struct Mutex<T>(std::sync::Mutex<T>);
            impl<T> Mutex<T> { pub fn new(value:T)->Self { Self(std::sync::Mutex::new(value)) } }
        }}
        #[path = "@ROOT@/crates/catten/src/cpu/multiprocessor/startup.rs"] pub mod startup;
        pub fn count() -> u32 { *startup::LP_COUNT.read() }
    }
}
mod environment { pub mod boot_protocol { pub mod limine {
    pub struct Request;
    pub static MP_REQUEST: Request = Request;
    pub struct Response { pub flags: u32 }
    pub struct Cpu { pub processor_id: u32 }
    impl Cpu { pub fn bootstrap(&self, _: fn(), _: u64) {} }
    static CPUS: [Cpu; 257] = [const { Cpu { processor_id: 0 } }; 257];
    impl Request { pub fn response(&self)->Option<Response> { Some(Response { flags: 1 }) } }
    impl Response { pub fn cpus(&self)->&[Cpu] {
        &CPUS[..std::env::var("Q3_LP_COUNT").unwrap().parse::<usize>().unwrap()]
    }}
    pub struct FramebufferRequest;
    pub static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest;
    pub struct FramebufferResponse;
    pub struct Framebuffer {
        pub width:u64, pub height:u64, pub pitch:u64,
        pub red_mask_size:u8, pub red_mask_shift:u8,
        pub green_mask_size:u8, pub green_mask_shift:u8,
        pub blue_mask_size:u8, pub blue_mask_shift:u8,
    }
    pub static PIXELS: core::sync::atomic::AtomicPtr<u32> = core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());
    impl FramebufferRequest { pub fn response(&self)->Option<FramebufferResponse> { Some(FramebufferResponse) } }
    impl FramebufferResponse { pub fn framebuffers(&self)->&[Framebuffer] { &FRAMEBUFFERS } }
    static FRAMEBUFFERS: [Framebuffer;1] = [Framebuffer {
        width:128,height:64,pitch:512,red_mask_size:8,red_mask_shift:16,
        green_mask_size:8,green_mask_shift:8,blue_mask_size:8,blue_mask_shift:0,
    }];
    impl Framebuffer { pub fn address(&self)->*mut u8 { PIXELS.load(core::sync::atomic::Ordering::SeqCst).cast() } }
}}}
// The host only passes this opaque pointer back to C. Layout information in
// the unsafe control comes from the compiled backend, not this declaration.
#[repr(C)] pub struct flanterm_context { _opaque: [u8; 1] }
#[repr(C, align(@MALLOC_ALIGNMENT@))] pub struct max_align_t([u8; 32]);
type Malloc = Option<extern "C" fn(usize)->*mut core::ffi::c_void>;
type Free = Option<unsafe extern "C" fn(*mut core::ffi::c_void, usize)>;
unsafe extern "C" {
    pub fn flanterm_write(ctx:*mut flanterm_context, text:*const core::ffi::c_char, len:usize);
    pub fn flanterm_fb_init(malloc:Malloc, free:Free, framebuffer:*mut u32,
        width:usize,height:usize,pitch:usize,rsize:u8,rshift:u8,gsize:u8,gshift:u8,bsize:u8,bshift:u8,
        canvas:*mut u32,ansi:*mut u32,bright:*mut u32,bg:*mut u32,fg:*mut u32,
        bg_bright:*mut u32,fg_bright:*mut u32,font:*mut core::ffi::c_void,
        fw:usize,fh:usize,spacing:usize,sx:usize,sy:usize,margin:usize)->*mut flanterm_context;
    fn q3_base_size()->usize;
    fn q3_malloc_alignment()->usize;
    fn q3_deinit(ctx:*mut flanterm_context, free:Free);
}
thread_local! { static TRACK: Cell<bool> = const { Cell::new(false) }; }
static CALLS: AtomicUsize = AtomicUsize::new(0);
static FAIL_AT: AtomicUsize = AtomicUsize::new(0);
static WRONG_LAYOUT: AtomicUsize = AtomicUsize::new(0);
static POINTERS: [AtomicUsize; 32] = [const { AtomicUsize::new(0) }; 32];
static SIZES: [AtomicUsize; 32] = [const { AtomicUsize::new(0) }; 32];
static ALIGNS: [AtomicUsize; 32] = [const { AtomicUsize::new(0) }; 32];
struct Tracer;
#[global_allocator] static ALLOCATOR: Tracer = Tracer;
unsafe impl GlobalAlloc for Tracer {
    unsafe fn alloc(&self, layout:Layout)->*mut u8 {
        let tracing = TRACK.try_with(Cell::get).unwrap_or(false);
        let index = if tracing { CALLS.fetch_add(1,SeqCst) } else { usize::MAX };
        if tracing && FAIL_AT.load(SeqCst) == index + 1 { return core::ptr::null_mut(); }
        let pointer = unsafe { System.alloc(layout) };
        if index < 32 && !pointer.is_null() {
            SIZES[index].store(layout.size(),SeqCst); ALIGNS[index].store(layout.align(),SeqCst);
            POINTERS[index].store(pointer as usize,SeqCst);
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer:*mut u8, mut layout:Layout) {
        for i in 0..32 {
            if POINTERS[i].load(SeqCst) == pointer as usize {
                let original = Layout::from_size_align(SIZES[i].load(SeqCst), ALIGNS[i].load(SeqCst)).unwrap();
                if original != layout { WRONG_LAYOUT.fetch_add(1,SeqCst); }
                // Keep the deliberately unsafe control from corrupting the host.
                layout = original;
                POINTERS[i].store(0,SeqCst);
                break;
            }
        }
        unsafe { System.dealloc(pointer,layout) };
    }
}
fn live() -> usize { POINTERS.iter().filter(|p| p.load(SeqCst)!=0).count() }
fn start(fail_at:usize) {
    assert_eq!(live(),0);
    CALLS.store(0,SeqCst); FAIL_AT.store(fail_at,SeqCst); WRONG_LAYOUT.store(0,SeqCst);
    TRACK.with(|t|t.set(true));
}
fn stop() { TRACK.with(|t|t.set(false)); }
mod log {
    #[path = "@ROOT@/crates/catten/src/log/chars.rs"] pub mod chars;
    pub mod flanterm {
        include!("@CONSOLE@");
        #[test]
        fn terminal() {
            use crate::*;
            use core::fmt::Write;
            assert_eq!(unsafe { q3_malloc_alignment() }, core::mem::align_of::<max_align_t>());
            let mut pixels = vec![0u32; 128*64];
            environment::boot_protocol::limine::PIXELS.store(pixels.as_mut_ptr(),SeqCst);
            // The backend allocates context, font bits, font bools, grid,
            // queue and map in that order. Every failed prefix must unwind.
            for failure in 1..=6 {
                start(failure);
                let console = init_console();
                stop();
                assert!(!console.is_available());
                assert_eq!(CALLS.load(SeqCst),failure);
                assert_eq!(live(),0,"C initialization retained a failed prefix");
                assert_eq!(WRONG_LAYOUT.load(SeqCst),0,"C cleanup used a different allocation layout");
            }
            start(0);
            let mut console = init_console();
            let pointer = console.ctx.as_ref().unwrap().0.as_ptr();
            let allocations = CALLS.load(SeqCst);
            // Exercise scrolling and escape parser paths after initialization.
            for _ in 0..100 { console.write_str("hello\n\x1b[31mred\x1b[0m\x1b[2J\x1b[H").unwrap(); }
            drop(console);
            stop();
            assert_eq!(allocations,6);
            assert_eq!(CALLS.load(SeqCst),allocations,"terminal write allocated");
            assert_eq!(live(),6,"console abandonment released backend storage");
            assert_eq!(WRONG_LAYOUT.load(SeqCst),0);
            // Production retains the static context. The host alone explicitly
            // disposes it after its last use to verify the complete C release.
            unsafe { q3_deinit(pointer,Some(free)); }
            assert_eq!(live(),0);
            assert_eq!(WRONG_LAYOUT.load(SeqCst),0);
            assert!(malloc(0).is_null());
            assert!(malloc(usize::MAX).is_null());
        }
    }
}
#[test] fn lp_count() {
    assert_eq!(cpu::multiprocessor::count() as usize,
        std::env::var("Q3_LP_COUNT").unwrap().parse::<usize>().unwrap());
}
