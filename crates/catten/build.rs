use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=CATTEN_TRUST_MODE");
    match env::var("CATTEN_TRUST_MODE") {
        Err(env::VarError::NotPresent) => {}
        Ok(mode) if mode == "development" => {}
        Ok(mode) if mode == "production" => panic!(
            "production images are disabled: protected trust and recipient-key provisioning is \
             not implemented; development fixtures must not protect real credentials"
        ),
        _ => panic!("CATTEN_TRUST_MODE must be development or production"),
    }
    println!("cargo:rustc-env=CATTEN_TRUST_MODE=development");
    println!("cargo:warning=DEVELOPMENT image: public fixture trust; do not deploy real secrets");
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    if env::var_os("CARGO_FEATURE_BOOT_TRUST_TEST").is_some() {
        assert_eq!(arch, "x86_64", "boot_trust_test supports only the x86 QEMU fixture");
        println!(
            "cargo:warning=boot_trust_test uses public test enrollment/state/recipient fixtures"
        );
    }

    match arch.as_str() {
        "x86_64" => {
            // Tell cargo to pass the linker script to the linker...
            println!("cargo:rustc-link-arg=-T./crates/catten/linker/x86_64.ld");
            // ...and to re-run if it changes. Cargo resolves rerun-if-changed
            // paths relative to the manifest directory, not the workspace
            // root, so this must not repeat the crate prefix.
            println!("cargo:rerun-if-changed=linker/x86_64.ld");
        }
        "aarch64" => {
            // Tell cargo to pass the linker script to the linker...
            println!("cargo:rustc-link-arg=-T./crates/catten/linker/aarch64.ld");
            // ...and to re-run if it changes. Cargo resolves rerun-if-changed
            // paths relative to the manifest directory, not the workspace
            // root, so this must not repeat the crate prefix.
            println!("cargo:rerun-if-changed=linker/aarch64.ld");
        }
        "riscv64" => {
            // Tell cargo to pass the linker script to the linker...
            println!("cargo:rustc-link-arg=-T./crates/catten/linker/riscv64.ld");
            // ...and to re-run if it changes. Cargo resolves rerun-if-changed
            // paths relative to the manifest directory, not the workspace
            // root, so this must not repeat the crate prefix.
            println!("cargo:rerun-if-changed=linker/riscv64.ld");
        }
        _ => panic!("Invalid ISA"),
    }
}
