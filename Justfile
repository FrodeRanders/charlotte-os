init-submodules:
    git submodule update --init --recursive

build-catten arch="x86_64" profile="debug":
    cargo build --package catten --target {{ if arch == "x86_64" { "x86_64-unknown-none-catten.json" } else if arch == "aarch64" { "aarch64-unknown-none-catten.json" } else if arch == "riscv64" { "riscv64gc-unknown-none-catten.json" } else { arch + "-unknown-none" } }} {{ if profile == "release" { "--release" } else { "" } }}

image_dir := "./os-images"
temp_mnt_dir := "~/temp-mnt"
create-image arch="x86_64" profile="debug": (build-catten arch profile) init-submodules
    #!/usr/bin/env bash
    if [ ! -d {{image_dir}} ]; then mkdir {{image_dir}}; fi
    touch {{image_dir}}/charlotte-{{arch}}-{{profile}}.hdd
    dd if=/dev/zero of={{image_dir}}/charlotte-{{arch}}-{{profile}}.hdd bs=4K count=1048576
    parted -s {{image_dir}}/charlotte-{{arch}}-{{profile}}.hdd mklabel gpt
    parted -s {{image_dir}}/charlotte-{{arch}}-{{profile}}.hdd mkpart ESP fat32 1MiB 100%
    parted -s {{image_dir}}/charlotte-{{arch}}-{{profile}}.hdd set 1 esp on
    lodev=$(sudo losetup -fP --show {{image_dir}}/charlotte-{{arch}}-{{profile}}.hdd)
    sudo mkfs.fat -F32 ${lodev}p1
    if [ ! -d {{temp_mnt_dir}} ]; then mkdir {{temp_mnt_dir}}; fi
    sudo mount ${lodev}p1 {{temp_mnt_dir}}
    sudo mkdir -p {{temp_mnt_dir}}/EFI/BOOT
    sudo cp ./Limine/BOOTX64.EFI {{temp_mnt_dir}}/EFI/BOOT/BOOTX64.EFI
    sudo cp ./target/{{ if arch == "x86_64" { "x86_64-unknown-none-catten" } else if arch == "aarch64" { "aarch64-unknown-none-catten" } else if arch == "riscv64" { "riscv64gc-unknown-none-catten" } else { arch + "-unknown-none" } }}/{{profile}}/catten ./limine.conf {{temp_mnt_dir}}
    sudo umount {{temp_mnt_dir}}
    sudo losetup -d $lodev
    rm -r {{temp_mnt_dir}}

vm_memory := "4G"
vm_num_lps := "2"

qemu-run-x86_64 profile="debug" serial= "" gdb="false": (create-image "x86_64" profile)
    qemu-system-x86_64 -enable-kvm -M q35 -cpu host,+invtsc -smp {{vm_num_lps}} -m {{vm_memory}} -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/ovmf/OVMF_CODE.fd -boot d -serial {{if serial != "" {"file:"+serial} else {"stdio"}}} \
    -drive file={{image_dir}}/charlotte-x86_64-{{profile}}.hdd,format=raw {{if gdb == "true" {"-s -S"} else {""}}}

qemu-run-aarch64 profile="debug": (create-image "aarch64" profile)
    qemu-system-aarch64 -M virt -cpu cortex-a76 -smp {{vm_num_lps}} -m {{vm_memory}} -device ramfb -device qemu-xhci -device usb-kbd -m {{vm_memory}} -bios /usr/share/edk2/aarch64/QEMU_EFI.fd -boot d \
    -drive file={{image_dir}}/charlotte-aarch64-{{profile}}.hdd,format=raw

clean:
    cargo clean
    rm -rf {{image_dir}}