use alloc::boxed::Box;
use core::{
    ffi::{
        c_char,
        c_void,
    },
    fmt::Write,
    ptr::null_mut,
};

use flanterm_bindings::*;
use spin::LazyLock;

use crate::{
    cpu::multiprocessor::spin::mutex::Mutex,
    environment::boot_protocol::limine::FRAMEBUFFER_REQUEST,
    log::chars::{
        FONT_HEIGHT,
        FONT_WIDTH,
    },
};

/// The framebuffer terminal console.
///
/// A usable linear framebuffer is not guaranteed to be present: the bootloader
/// may not have been given one (e.g. a headless configuration, or a platform
/// where firmware exposes no Graphics Output Protocol). In that case this holds
/// `None` and callers fall back to the serial console, rather than the kernel
/// faulting while trying to draw to a nonexistent framebuffer.
pub struct FlantermConsole {
    ctx: Option<Box<flanterm_context>>,
}

impl FlantermConsole {
    /// Whether a framebuffer terminal is actually available.
    pub fn is_available(&self) -> bool {
        self.ctx.is_some()
    }
}

impl Write for FlantermConsole {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        if let Some(ctx) = self.ctx.as_mut() {
            unsafe {
                flanterm_write(&mut **ctx, s.as_ptr() as *const c_char, s.len());
            }
        }
        Ok(())
    }
}

pub static FT_CTX: LazyLock<Mutex<FlantermConsole>> = LazyLock::new(|| Mutex::new(init_console()));

/// Attempt to initialise the framebuffer terminal from the Limine framebuffer
/// response. Returns a console with no backing context if there is no usable
/// framebuffer (missing response, no framebuffers, zero dimensions, or a null
/// address), or if `flanterm` itself fails to initialise.
fn init_console() -> FlantermConsole {
    let Some(fb_res) = FRAMEBUFFER_REQUEST.response() else {
        crate::early_logln!("flanterm: no Limine framebuffer response");
        return FlantermConsole {
            ctx: None,
        };
    };
    let Some(fb) = fb_res.framebuffers().first() else {
        crate::early_logln!("flanterm: framebuffer response has no framebuffers");
        return FlantermConsole {
            ctx: None,
        };
    };
    if fb.address().is_null() || fb.width == 0 || fb.height == 0 || fb.pitch == 0 {
        crate::early_logln!("flanterm: no usable framebuffer (null or zero dimensions)");
        return FlantermConsole {
            ctx: None,
        };
    }
    crate::early_logln!("flanterm: framebuffer terminal initialised successfully");
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
    if ctx_mut.is_null() {
        crate::early_logln!("flanterm: flanterm_fb_init returned null");
        return FlantermConsole {
            ctx: None,
        };
    }
    crate::early_logln!("flanterm: framebuffer terminal initialised successfully");
    FlantermConsole {
        ctx: Some(unsafe { Box::from_raw(ctx_mut) }),
    }
}

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
