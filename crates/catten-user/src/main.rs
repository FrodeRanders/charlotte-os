//! CharlotteOS sitas smoke image — runs `basic_kv` and the mailbox-index demo
//! behind the crt0 contract.
//!
//! Results are written to the status page: `basic_kv` writes its verified key
//! count at offset 0, the mailbox-index demo writes its verified record count
//! at offset 1 (a `u32` each). The kernel self-test verifier checks both.
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
    let output = config::output_ptr::<u32>();

    unsafe {
        sitas_core::basic_kv::basic_kv_test(&reactor, output);
        sitas_core::mailbox_index::mailbox_index_test(&reactor, output.add(1));
    }

    unsafe {
        thread_exit();
    }
}

catten_rt::entry!(main);
