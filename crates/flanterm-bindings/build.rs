extern crate bindgen;
extern crate cc;
fn main() {
    let target = std::env::var("TARGET").expect("Target triple is not set");
    let mut e = cc::Build::new();
    e.file("flanterm.c")
        .file("backends/fb.c")
        .flag("-fno-stack-protector")
        .flag("-fno-stack-check")
        .flag("-ffreestanding")
        .flag("-mno-red-zone")
        .flag("-fno-lto");

    if target.as_str() == "x86_64-unknown-none" {
        e.flag("-march=x86-64")
            .flag("-mno-80387")
            .flag("-mno-mmx")
            .flag("-mno-sse")
            .flag("-mno-sse2");
    } else {
        e.flag("-mgeneral-regs-only");
    }

    e.compile("flanterm");
    let bindings = bindgen::builder()
        .clang_arg("-ffreestanding")
        .use_core()
        .header("wrapper.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");
    let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings.write_to_file(out_path.join("bindings.rs")).expect("Couldn't write bindings!");
}
