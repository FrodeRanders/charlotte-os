//! CharlotteOS adder — adds kernel-placed numbers, subtracts boilerplate.
#![no_std]
#![no_main]

extern crate alloc;
use catten_rt::{
    config,
    Args,
    Input,
};
use catten_syscall::thread_exit;

fn cmain(args: Args, input: Input<32>) -> ! {
    let a = args.get(0).unwrap_or(0);
    let b = args.get(1).unwrap_or(0);
    let kernel_val = input.read_u32(0).unwrap_or(0);

    let sum = a.wrapping_add(b).wrapping_add(kernel_val);
    config::write(0, 0xc0deu32);
    config::write(4, b);
    config::write(8, sum);

    unsafe {
        thread_exit();
    }
}

catten_rt::entry!(cmain);
