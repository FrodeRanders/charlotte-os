build-catten arch="x86_64" profile="debug" features="":
    cargo build --package catten \
        --target {{ if arch == "x86_64" { "target_specs/x86_64-unknown-none-catten.json" } else if arch == "aarch64" { "target_specs/aarch64-unknown-none-catten.json" } else if arch == "riscv64" { "target_specs/riscv64gc-unknown-none-catten.json" } else { arch + "-unknown-none" } }} \
        --no-default-features --features "acpi{{ if features != "" { "," + features } else { "" } }}" \
        {{ if profile == "release" { "--release" } else { "" } }}

build-catten-docs arch="x86_64" profile="debug" features="":
    cargo doc --package catten --target {{ if arch == "x86_64" { "target_specs/x86_64-unknown-none-catten.json" } else if arch == "aarch64" { "target_specs/aarch64-unknown-none-catten.json" } else if arch == "riscv64" { "target_specs/riscv64gc-unknown-none-catten.json" } else { arch + "-unknown-none" } }} {{ if profile == "release" { "--release" } else { "" } }} {{ if features !=
    "" {"--features " + features} else {""} }} --no-deps --open

# Rebuild the reference EL0 service ELFs (name service, echo, client) and
# refresh the copies embedded in the kernel self-tests.
build-el0-services:
    cd crates/catten-services && cargo build -p catten-services --release --target aarch64-unknown-none.json -Z build-std=core,alloc
    cp crates/catten-services/target/aarch64-unknown-none/release/ns crates/catten/src/self_test/ns.elf
    cp crates/catten-services/target/aarch64-unknown-none/release/echo crates/catten/src/self_test/echo.elf
    cp crates/catten-services/target/aarch64-unknown-none/release/client crates/catten/src/self_test/client.elf
    cp crates/catten-services/target/aarch64-unknown-none/release/uart crates/catten/src/self_test/uart.elf
    cp crates/catten-services/target/aarch64-unknown-none/release/cclient crates/catten/src/self_test/cclient.elf
    cp crates/catten-services/target/aarch64-unknown-none/release/servicemgr crates/catten/src/self_test/servicemgr.elf

image_dir := "./os-images"
create-image arch="x86_64" profile="debug" features="": (build-catten arch profile features)
    ./scripts/create-boot-image.sh --arch "{{arch}}" --profile "{{profile}}"

verify-limine:
    ./scripts/verify-limine.sh

vm_memory := "512M"
vm_num_lps := "8"
usb_image_path := "./test_data/disk_images/test-usb.img"

qemu-run-x86_64 profile="debug" gdb="false":
    ./scripts/run-x86_64.sh "{{profile}}" --kvm {{ if gdb == "true" { "--gdb" } else { "" } }}

qemu-run-aarch64 profile="debug" gdb="false":
    ./scripts/run-aarch64.sh "{{profile}}" {{ if gdb == "true" { "--gdb" } else { "" } }}

qemu-run-riscv64 profile="debug" gdb="false": (create-image "riscv64" profile)
    qemu-system-riscv64 \
        -M virt \
        -cpu tt-ascalon \
        -smp {{vm_num_lps}} \
        -m {{vm_memory}} \
        -bios /usr/share/edk2/riscv64/QEMU_EFI.fd \
        -boot d \
        -device ramfb \
        -device qemu-xhci,id=xhci \
        -device usb-kbd,bus=xhci.0 \
        -device usb-mouse,bus=xhci.0 \
        -device usb-net,netdev=usbnet0,bus=xhci.0 \
        -device riscv-iommu-pci \
        -drive file={{image_dir}}/charlotte-riscv64-{{profile}}.img,format=raw \
        -device usb-storage,bus=xhci.0,drive=usbdrive0 \
        -drive if=none,id=usbdrive0,format=raw,file={{usb_image_path}} \
        {{ if gdb == "true" {"-s -S"} else {""} }}

update-loc:
    tokei \
        --exclude target \
        --exclude .git \
        --exclude .vscode \
        --exclude .github \
        --exclude limine-binary \
        --exclude os-images \
    > loc.txt

clean:
    cargo clean
    rm -rf {{image_dir}}

distclean: clean
    if [ -f Cargo.lock ]; then rm Cargo.lock; fi

# Run the same strict check the CI uses (-D warnings) to catch issues locally
# before pushing. Equivalent to `RUSTFLAGS="-D warnings" cargo check ...`
check arch="aarch64":
    RUSTFLAGS="-D warnings" cargo check --package catten \
        --target {{ if arch == "x86_64" { "target_specs/x86_64-unknown-none-catten.json" } else { "target_specs/aarch64-unknown-none-catten.json" } }} \
        --no-default-features --features acpi

# Build and check raft service binary
check-raft:
    RUSTFLAGS="-D warnings" cargo check --manifest-path crates/catten-services/Cargo.toml --target crates/catten-services/aarch64-unknown-none.json
