//! basic_kv on CharlotteOS — exercises sitas ShardedKv via ShardedExecutor.
#![no_std]
#![no_main]
#![deny(unsafe_op_in_unsafe_fn)]

use core::panic::PanicInfo;
use sitas_charlotte::CharlotteReactor;
use sitas_core::basic_kv;
use talc::{source::Claim, TalcLock};

const HEAP_BASE: usize = 0x0000_0000_0001_3000;
const HEAP_SIZE: usize = 0x0000_0000_0000_d000;
const RESULT_PAGE: *mut u32 = 0x0000_0000_0001_2000usize as *mut u32;

#[global_allocator]
static ALLOCATOR: TalcLock<spin::Mutex<()>, Claim> =
    TalcLock::new(unsafe { Claim::new(HEAP_BASE as *mut u8, HEAP_SIZE) });

#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    let reactor = CharlotteReactor::new(1, 0);
    unsafe {
        basic_kv::basic_kv_test(&reactor, RESULT_PAGE);
    }

    loop { core::hint::spin_loop(); }
}

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop { core::hint::spin_loop(); }
}
