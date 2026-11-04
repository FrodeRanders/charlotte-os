use core::{
    ffi::{
        c_char,
        c_void,
    },
    fmt::Write,
    ptr::{
        NonNull,
        null_mut,
    },
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
/// The context is `None` when the bootloader provides no usable framebuffer
/// or terminal initialization fails. Writes then return without drawing;
/// the logging layer decides whether to use another console.
pub struct FlantermConsole {
    ctx: Option<BootContext>,
}

/// The C backend owns a larger `flanterm_fb_context` and its child buffers.
/// Retain that complete allocation for the boot lifetime; a Rust Box of its
/// base `flanterm_context` would have the wrong allocation layout. Dropping this
/// handle is deliberately inert. There is no console replacement/retirement
/// path; adding one requires explicit backend deinit outside console guards.
struct BootContext(NonNull<flanterm_context>);

// The context has no thread affinity. FT_CTX serializes every C operation,
// and no other alias is retained after successful initialization.
unsafe impl Send for BootContext {}

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
                flanterm_write(ctx.0.as_ptr(), s.as_ptr() as *const c_char, s.len());
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
    let Some(ctx) = NonNull::new(ctx_mut) else {
        crate::early_logln!("flanterm: flanterm_fb_init returned null");
        return FlantermConsole {
            ctx: None,
        };
    };
    crate::early_logln!("flanterm: framebuffer terminal initialised successfully");
    FlantermConsole {
        ctx: Some(BootContext(ctx)),
    }
}

extern "C" fn malloc(size: usize) -> *mut c_void {
    let Ok(layout) =
        core::alloc::Layout::from_size_align(size, core::mem::align_of::<max_align_t>())
    else {
        return null_mut();
    };
    if size == 0 {
        return null_mut();
    }
    unsafe { alloc::alloc::alloc(layout) as *mut c_void }
}

/// Called only by the backend for an exact nonzero allocation from `malloc`.
unsafe extern "C" fn free(ptr: *mut c_void, size: usize) {
    let layout =
        core::alloc::Layout::from_size_align(size, core::mem::align_of::<max_align_t>()).unwrap();
    unsafe { alloc::alloc::dealloc(ptr as *mut u8, layout) };
}
