init-submodules:
    git submodule update --init --recursive

build-catten arch="x86_64" profile="debug":
    cargo build --package catten --target {{arch}}-unknown-none {{ if profile == "release" { "--release" } else { "" } }}

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
    sudo cp ./target/{{arch}}-unknown-none/{{profile}}/catten ./limine.conf {{temp_mnt_dir}}
    sudo umount {{temp_mnt_dir}}
    sudo losetup -d $lodev
    rm -r {{temp_mnt_dir}}

clean:
    cargo clean
    rm -rf {{image_dir}}