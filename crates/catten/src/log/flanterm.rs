use alloc::boxed::Box;
use alloc::fmt::Write;
use core::ffi::{c_char, c_void};
use core::ptr::null_mut;

use flanterm_bindings::*;
use spin::LazyLock;

use crate::cpu::multiprocessor::spin::mutex::Mutex;
use crate::environment::boot_protocol::limine::FRAMEBUFFER_REQUEST;
use crate::log::chars::{FONT_HEIGHT, FONT_WIDTH};

pub struct FlantermContext {
    ctx: Box<flanterm_context>,
}

impl FlantermContext {
    pub fn new(ctx: Box<flanterm_context>) -> Self {
        FlantermContext {
            ctx,
        }
    }
}

impl Write for FlantermContext {
    fn write_str(&mut self, str: &str) -> core::fmt::Result {
        let ctx_mut = &mut *self.ctx;
        unsafe {
            flanterm_write(ctx_mut, str.as_ptr() as *const c_char, str.len());
        }
        Ok(())
    }
}

pub static FT_CTX: LazyLock<Mutex<FlantermContext>> = LazyLock::new(|| {
    let fb_res = FRAMEBUFFER_REQUEST.response().expect("Failed to get Limine framebuffer response");
    let fb = fb_res.framebuffers().first().expect("No framebuffer found in Limine response");
    let ctx_mut = unsafe {
        flanterm_fb_init(
            Some(malloc),
            Some(free),
            fb.address() as *mut u32,
            fb.width as usize,
            fb.height as usize,
            fb.pitch as usize,
            fb.red_mask_size,
            fb.red_mask_shift,
            fb.green_mask_size,
            fb.green_mask_shift,
            fb.blue_mask_size,
            fb.blue_mask_shift,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &raw const super::chars::FONT as *mut c_void,
            FONT_WIDTH,
            FONT_HEIGHT,
            0,
            0,
            0,
            0,
        )
    };
    Mutex::new(FlantermContext::new(unsafe { Box::from_raw(ctx_mut) }))
});

pub extern "C" fn malloc(size: usize) -> *mut c_void {
    let layout =
        core::alloc::Layout::from_size_align(size, core::mem::align_of::<max_align_t>()).unwrap();
    unsafe { alloc::alloc::alloc(layout) as *mut c_void }
}

pub extern "C" fn free(ptr: *mut c_void, size: usize) {
    let layout =
        core::alloc::Layout::from_size_align(size, core::mem::align_of::<max_align_t>()).unwrap();
    unsafe { alloc::alloc::dealloc(ptr as *mut u8, layout) };
}
