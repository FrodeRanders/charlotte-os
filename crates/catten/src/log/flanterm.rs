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
use limine::framebuffer::{
    FRAMEBUFFER_RGB,
    Framebuffer,
};
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

// One boot-lifetime console, not a runtime application allocation pool. Admit
// the complete fixed C prefix before its first allocation. This permits common
// 8K modes without permitting firmware geometry to exhaust the kernel heap.
const MAX_CONSOLE_HEAP_BYTES: usize = 12 * 1024 * 1024;
const MAX_FRAMEBUFFER_BYTES: usize = 256 * 1024 * 1024;
const FONT_GLYPHS: usize = 256;

struct FramebufferLayout {
    address: NonNull<u32>,
    width: usize,
    height: usize,
    pitch: usize,
    shifts: [u8; 3],
    font_scale: usize,
    allocations: [usize; 6],
    requested_bytes: usize,
}

impl FramebufferLayout {
    /// The bootloader supplies the actual mapped framebuffer backing. This
    /// checks metadata, arithmetic and C allocation bounds, not page ownership.
    fn validate(fb: &Framebuffer) -> Option<Self> {
        if fb.memory_model != FRAMEBUFFER_RGB || fb.bpp != 32 {
            return None;
        }
        let address = NonNull::new(fb.address().cast::<u32>())?;
        if !(address.as_ptr() as usize).is_multiple_of(core::mem::align_of::<u32>()) {
            return None;
        }
        let width = usize::try_from(fb.width).ok()?;
        let height = usize::try_from(fb.height).ok()?;
        let pitch = usize::try_from(fb.pitch).ok()?;
        if !pitch.is_multiple_of(size_of::<u32>()) || pitch < width.checked_mul(size_of::<u32>())? {
            return None;
        }
        let span = height.checked_mul(pitch)?;
        if span == 0 || span > MAX_FRAMEBUFFER_BYTES {
            return None;
        }
        (address.as_ptr() as usize).checked_add(span)?;

        let shifts = [fb.red_mask_shift, fb.green_mask_shift, fb.blue_mask_shift];
        if [fb.red_mask_size, fb.green_mask_size, fb.blue_mask_size] != [8; 3] {
            return None;
        }
        let mut channels = 0u32;
        for shift in shifts {
            if shift > 24 {
                return None;
            }
            let mask = 0xffu32 << shift;
            if channels & mask != 0 {
                return None;
            }
            channels |= mask;
        }

        // Match the pinned backend's automatic scaling, then pass it explicitly
        // so the checked rows/columns are exactly the ones C will allocate.
        let font_scale = if width >= 5120 && height >= 2880 {
            4
        } else if width >= 2560 && height >= 1440 {
            2
        } else {
            1
        };
        let columns = width / FONT_WIDTH.checked_mul(font_scale)?;
        let rows = height / FONT_HEIGHT.checked_mul(font_scale)?;
        if columns == 0 || rows == 0 {
            return None;
        }
        let cells = rows.checked_mul(columns)?;
        // The backend's VGA glyph decoding requires an 8-pixel-wide font.
        const {
            assert!(FONT_WIDTH == 8 && FONT_HEIGHT == 16);
            assert!(super::chars::FONT.len() == FONT_GLYPHS * FONT_HEIGHT);
        };
        let font_pixels = FONT_WIDTH.checked_mul(FONT_HEIGHT)?.checked_mul(FONT_GLYPHS)?;
        let allocations = [
            size_of::<flanterm_fb_context>(),
            font_pixels / 8,
            font_pixels.checked_mul(size_of::<bool>())?,
            cells.checked_mul(size_of::<flanterm_fb_char>())?,
            cells.checked_mul(size_of::<flanterm_fb_queue_item>())?,
            cells.checked_mul(size_of::<*mut flanterm_fb_queue_item>())?,
        ];
        let mut requested_bytes = 0usize;
        for size in allocations {
            if size == 0 {
                return None;
            }
            core::alloc::Layout::from_size_align(size, core::mem::align_of::<max_align_t>())
                .ok()?;
            requested_bytes = requested_bytes.checked_add(size)?;
        }
        if requested_bytes > MAX_CONSOLE_HEAP_BYTES {
            return None;
        }
        Some(Self {
            address,
            width,
            height,
            pitch,
            shifts,
            font_scale,
            allocations,
            requested_bytes,
        })
    }
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
                flanterm_write(ctx.0.as_ptr(), s.as_ptr() as *const c_char, s.len());
            }
        }
        Ok(())
    }
}

pub static FT_CTX: LazyLock<Mutex<FlantermConsole>> = LazyLock::new(|| Mutex::new(init_console()));

/// Attempt to initialise the framebuffer terminal from the Limine framebuffer
/// response. Returns a console with no backing context if there is no usable
/// framebuffer, its metadata/bounds are unsupported, or C initialization fails.
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
    init_framebuffer(fb)
}

fn init_framebuffer(fb: &Framebuffer) -> FlantermConsole {
    let Some(layout) = FramebufferLayout::validate(fb) else {
        crate::early_logln!("flanterm: unsupported framebuffer metadata or allocation bounds");
        return FlantermConsole {
            ctx: None,
        };
    };
    debug_assert_eq!(layout.allocations.iter().sum::<usize>(), layout.requested_bytes);
    let ctx_mut = unsafe {
        flanterm_fb_init(
            Some(malloc),
            Some(free),
            layout.address.as_ptr(),
            layout.width,
            layout.height,
            layout.pitch,
            8,
            layout.shifts[0],
            8,
            layout.shifts[1],
            8,
            layout.shifts[2],
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
            layout.font_scale,
            layout.font_scale,
            0,
        )
    };
    let Some(ctx) = NonNull::new(ctx_mut) else {
        crate::early_logln!("flanterm: flanterm_fb_init returned null");
        return FlantermConsole {
            ctx: None,
        };
    };
    crate::early_logln!(
        "flanterm: framebuffer terminal initialised successfully ({}x{}, scale={}, heap={})",
        layout.width,
        layout.height,
        layout.font_scale,
        layout.requested_bytes
    );
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
