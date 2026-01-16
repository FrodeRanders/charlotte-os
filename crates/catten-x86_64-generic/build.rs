use std::env;

fn main() {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    match arch.as_str() {
        "x86_64" => {
            // Tell cargo to pass the linker script to the linker...
            println!("cargo:rustc-link-arg=-T./crates/catten-x86_64-generic/linker/x86_64.ld");
            // ...and to re-run if it changes.
            println!("cargo:rerun-if-changed=./crates/catten-x86_64-generic/linker/x86_64.ld");
        }
        "aarch64" => {
            // Tell cargo to pass the linker script to the linker...
            println!("cargo:rustc-link-arg=-T./crates/catten-aarch64-generic/linker/aarch64.ld");
            // ...and to re-run if it changes.
            println!("cargo:rerun-if-changed=./crates/catten-aarch64-generic/linker/aarch64.ld");
        }
        "riscv64" => {
            // Tell cargo to pass the linker script to the linker...
            println!("cargo:rustc-link-arg=-T./crates/catten-riscv64-generic/linker/riscv64.ld");
            // ...and to re-run if it changes.
            println!("cargo:rerun-if-changed=./crates/catten-riscv64-generic/linker/riscv64.ld");
        }
        _ => panic!("Invalid ISA"),
    }

    println!("cargo:rerun-if-changed=asm");
}
