build-catten-x86_64-debug:
    cargo build --package catten-x86_64-generic --target x86_64-unknown-none
build-catten-aarch64-debug:
    cargo build --package catten-aarch64-debug --target aarch64-unknown-none

image_dir := "./os-images"
temp_mnt_dir := "~/temp-mnt"
create-image-x86_64-debug: build-catten-x86_64-debug
    #!/usr/bin/env bash
    if [ ! -d {{image_dir}} ]; then mkdir {{image_dir}}; fi
    touch {{image_dir}}/charlotte-x86_64-debug.img
    dd if=/dev/zero of={{image_dir}}/charlotte-x86_64-debug.img bs=4K count=1048576
    parted -s {{image_dir}}/charlotte-x86_64-debug.img mklabel gpt
    parted -s {{image_dir}}/charlotte-x86_64-debug.img mkpart ESP fat32 1MiB 100%
    parted -s {{image_dir}}/charlotte-x86_64-debug.img set 1 esp on
    lodev=$(sudo losetup -fP --show {{image_dir}}/charlotte-x86_64-debug.img)
    sudo mkfs.fat -F32 ${lodev}p1
    if [ ! -d {{temp_mnt_dir}} ]; then mkdir {{temp_mnt_dir}}; fi
    sudo mount ${lodev}p1 {{temp_mnt_dir}}
    sudo mkdir -p {{temp_mnt_dir}}/EFI/BOOT
    sudo cp ./Limine/BOOTX64.EFI {{temp_mnt_dir}}/EFI/BOOT/BOOTX64.EFI
    sudo cp ./target/x86_64-unknown-none/debug/catten-x86_64-generic ./limine.conf {{temp_mnt_dir}}
    sudo umount {{temp_mnt_dir}}
    sudo losetup -d $lodev
    rm -r {{temp_mnt_dir}}