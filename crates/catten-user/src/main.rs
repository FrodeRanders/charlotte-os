//! CharlotteOS sitas smoke image — runs `basic_kv` behind the crt0 contract.
#![no_std]
#![no_main]

extern crate alloc;
use catten_rt::{
    Args,
    Input,
    config,
};
use catten_syscall::thread_exit;
use sitas_charlotte::CharlotteReactor;

fn cmain(args: Args, _input: Input<0>) -> ! {
    let asid = u64::from(args.get(0).unwrap_or(0));
    let lp_id = args.get(1).unwrap_or(0);
    let reactor = CharlotteReactor::new(asid, lp_id);

    unsafe {
        sitas_core::basic_kv::basic_kv_test(&reactor, config::CONFIG_VADDR as *mut u32);
    }

    unsafe {
        thread_exit();
    }
}

catten_rt::entry!(cmain);
