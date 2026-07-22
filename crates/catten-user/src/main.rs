//! CharlotteOS sitas smoke image — runs `basic_kv` behind the crt0 contract.
#![no_std]
#![no_main]

extern crate alloc;
use catten_rt::{
    Context,
    config,
};
use catten_syscall::thread_exit;
use sitas_charlotte::CharlotteReactor;

fn main(_ctx: Context) -> ! {
    let reactor = CharlotteReactor::new(0);

    unsafe {
        sitas_core::basic_kv::basic_kv_test(&reactor, config::output_ptr());
    }

    unsafe {
        thread_exit();
    }
}

catten_rt::entry!(main);
