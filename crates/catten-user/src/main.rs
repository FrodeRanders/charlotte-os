//! basic_kv on CharlotteOS — exercises sitas ShardedKv via ShardedExecutor.
#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

extern crate alloc;

use core::panic::PanicInfo;
use sitas_charlotte::CharlotteReactor;
use sitas_core::basic_kv;

const RESULT_PAGE: *mut u32 = 0x0000_0000_0001_2000usize as *mut u32;

#[global_allocator]
static ALLOCATOR: talc::TalcLock<spin::Mutex<()>, talc::source::Claim> =
    talc::TalcLock::new(unsafe {
        talc::source::Claim::new(core::ptr::null_mut(), 0)
    });

static mut ARENA: [u8; 16384] = [0u8; 16384];

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    // Register the arena with the allocator (can't do it in a const context
    // because ARENA is a mutable static).
    unsafe {
        ALLOCATOR
            .lock()
            .claim(core::ptr::addr_of_mut!(ARENA).cast(), 16384)
            .expect("talc: failed to claim userspace arena");
    }

    let reactor = CharlotteReactor::new(1, 0);
    unsafe { basic_kv::basic_kv_test(&reactor, RESULT_PAGE); }

    loop { core::hint::spin_loop(); }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}
