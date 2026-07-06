build-catten arch="x86_64" profile="debug" features="":
    cargo build --package catten --target {{ if arch == "x86_64" { "target_specs/x86_64-unknown-none-catten.json" } else if arch == "aarch64" { "target_specs/aarch64-unknown-none-catten.json" } else if arch == "riscv64" { "target_specs/riscv64gc-unknown-none-catten.json" } else { arch + "-unknown-none" } }} {{ if profile == "release" { "--release" } else { "" } }} {{ if features !=
    "" {"--features " + features} else {""} }}

build-catten-docs arch="x86_64" profile="debug" features="":
    cargo doc --package catten --target {{ if arch == "x86_64" { "target_specs/x86_64-unknown-none-catten.json" } else if arch == "aarch64" { "target_specs/aarch64-unknown-none-catten.json" } else if arch == "riscv64" { "target_specs/riscv64gc-unknown-none-catten.json" } else { arch + "-unknown-none" } }} {{ if profile == "release" { "--release" } else { "" } }} {{ if features !=
    "" {"--features " + features} else {""} }} --no-deps --open

image_dir := "./os-images"
temp_mnt_dir := "~/temp-mnt"
create-image arch="x86_64" profile="debug" features="": (build-catten arch profile features)
    #!/usr/bin/env bash
    if [ ! -d {{image_dir}} ]; then mkdir {{image_dir}}; fi
    touch {{image_dir}}/charlotte-{{arch}}-{{profile}}.img
    dd if=/dev/zero of={{image_dir}}/charlotte-{{arch}}-{{profile}}.img bs=4K count=1048576
    parted -s {{image_dir}}/charlotte-{{arch}}-{{profile}}.img mklabel gpt
    parted -s {{image_dir}}/charlotte-{{arch}}-{{profile}}.img mkpart ESP fat32 1MiB 100%
    parted -s {{image_dir}}/charlotte-{{arch}}-{{profile}}.img set 1 esp on
    lodev=$(sudo losetup -fP --show {{image_dir}}/charlotte-{{arch}}-{{profile}}.img)
    sudo mkfs.fat -F32 ${lodev}p1
    if [ ! -d {{temp_mnt_dir}} ]; then mkdir {{temp_mnt_dir}}; fi
    sudo mount ${lodev}p1 {{temp_mnt_dir}}
    sudo mkdir -p {{temp_mnt_dir}}/EFI/BOOT
    sudo cp ./limine-binary/BOOTX64.EFI {{temp_mnt_dir}}/EFI/BOOT/BOOTX64.EFI
    sudo cp ./target/{{ if arch == "x86_64" { "x86_64-unknown-none-catten" } else if arch == "aarch64" { "aarch64-unknown-none-catten" } else if arch == "riscv64" { "riscv64gc-unknown-none-catten" } else { arch + "-unknown-none" } }}/{{profile}}/catten ./limine.conf {{temp_mnt_dir}}
    sudo umount {{temp_mnt_dir}}
    sudo losetup -d $lodev
    rm -r {{temp_mnt_dir}}

vm_memory := "512M"
vm_num_lps := "8"
usb_image_path := "./test_data/disk_images/test-usb.img"

qemu-run-x86_64 profile="debug" features="qemu" gdb="false": (create-image "x86_64" profile features)
    qemu-system-x86_64 \
        -enable-kvm \
        -M q35,kernel-irqchip=split \
        -cpu host,+invtsc \
        -smp {{vm_num_lps}} \
        -m {{vm_memory}} \
        -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/ovmf/OVMF_CODE.fd \
        -boot d \
        -vga none \
        -device virtio-vga,xres=3840,yres=2160 \
        -drive file={{image_dir}}/charlotte-x86_64-{{profile}}.img,format=raw,if=none,id=nvme0 \
        -device nvme,drive=nvme0,serial=catten00 \
        -nic none \
        -device qemu-xhci,id=xhci \
        -device usb-kbd,bus=xhci.0 \
        -device usb-mouse,bus=xhci.0 \
        -netdev user,id=usbnet0 \
        -device usb-net,netdev=usbnet0,bus=xhci.0 \
        -device usb-storage,bus=xhci.0,drive=usbdrive0 \
        -drive if=none,id=usbdrive0,format=raw,file={{usb_image_path}} \
        -device amd-iommu \
        {{ if gdb == "true" {"-s -S"} else {""} }}

qemu-run-aarch64 profile="debug" gdb="false": (create-image "aarch64" profile)
    qemu-system-aarch64 \
        -M virt \
        -cpu cortex-a710 \
        -smp {{vm_num_lps}} \
        -m {{vm_memory}} \
        -bios /usr/share/edk2/aarch64/QEMU_EFI.fd \
        -boot d \
        -device ramfb \
        -device qemu-xhci,id=xhci \
        -device usb-kbd,bus=xhci.0 \
        -device usb-mouse,bus=xhci.0 \
        -device usb-net,netdev=usbnet0,bus=xhci.0 \
        -device arm-smmu \
        -drive file={{image_dir}}/charlotte-aarch64-{{profile}}.img,format=raw \
        -device usb-storage,bus=xhci.0,drive=usbdrive0 \
        -drive if=none,id=usbdrive0,format=raw,file={{usb_image_path}} \
        {{ if gdb == "true" {"-s -S"} else {""} }}

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