build-catten arch="x86_64" profile="debug" features="":
    cargo build --package catten --target {{ if arch == "x86_64" { "target_specs/x86_64-unknown-none-catten.json" } else if arch == "aarch64" { "target_specs/aarch64-unknown-none-catten.json" } else if arch == "riscv64" { "target_specs/riscv64gc-unknown-none-catten.json" } else { arch + "-unknown-none" } }} {{ if profile == "release" { "--release" } else { "" } }} {{ if features !=
    "" {"--features " + features} else {""} }}

image_dir := "./os-images"
temp_mnt_dir := "~/temp-mnt"
create-image arch="x86_64" profile="debug" features="": (build-catten arch profile features)
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
    sudo cp ./limine-binary/BOOTX64.EFI {{temp_mnt_dir}}/EFI/BOOT/BOOTX64.EFI
    sudo cp ./target/{{ if arch == "x86_64" { "x86_64-unknown-none-catten" } else if arch == "aarch64" { "aarch64-unknown-none-catten" } else if arch == "riscv64" { "riscv64gc-unknown-none-catten" } else { arch + "-unknown-none" } }}/{{profile}}/catten ./limine.conf {{temp_mnt_dir}}
    sudo umount {{temp_mnt_dir}}
    sudo losetup -d $lodev
    rm -r {{temp_mnt_dir}}

vm_memory := "512M"
vm_num_lps := "8"

qemu-run-x86_64 profile="debug" serial="" features="legacy_com_ports" gdb="false": (create-image "x86_64" profile features)
    #!/usr/bin/env bash
    if [ ! -f {{image_dir}}/scsi-test.hdd ]; then
        dd if=/dev/zero of={{image_dir}}/scsi-test.hdd bs=4K count=262144
    fi
    qemu-system-x86_64 \
        -enable-kvm \
        -M q35,kernel-irqchip=split \
        -cpu host,+invtsc \
        -smp {{vm_num_lps}} \
        -m {{vm_memory}} \
        -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/ovmf/OVMF_CODE.fd \
        -boot d \
        -serial {{if serial != "" {"file:"+serial} else {"stdio"}}} \
        -chardev file,path=logs/pci_serial.txt,id=pci_ser0 \
        -device pci-serial,chardev=pci_ser0 \
        -vga none \
        -device virtio-vga \
        -drive file={{image_dir}}/charlotte-x86_64-{{profile}}.hdd,format=raw,if=none,id=nvme0 \
        -device nvme,drive=nvme0,serial=catten00 \
        -drive file={{image_dir}}/scsi-test.hdd,format=raw,if=none,id=scsi_disk0 \
        -device virtio-scsi-pci,id=scsi0 \
        -device scsi-hd,drive=scsi_disk0,bus=scsi0.0 \
        -nic none \
        -device qemu-xhci,id=xhci \
        -device usb-kbd,bus=xhci.0 \
        -device usb-mouse,bus=xhci.0 \
        -netdev user,id=usbnet0 \
        -device usb-net,netdev=usbnet0,bus=xhci.0 \
        -device amd-iommu \
        {{ if gdb == "true" {"-s -S"} else {""} }}

qemu-run-aarch64 profile="debug" gdb="false": (create-image "aarch64" profile)
    qemu-system-aarch64 -M virt -cpu cortex-a76 -smp {{vm_num_lps}} -m {{vm_memory}} -device ramfb -device qemu-xhci -device usb-kbd -m {{vm_memory}} -bios /usr/share/edk2/aarch64/QEMU_EFI.fd -boot d \
    -drive file={{image_dir}}/charlotte-aarch64-{{profile}}.hdd,format=raw \
    {{ if gdb == "true" {"-s -S"} else {""} }}

clean:
    cargo clean
    rm -rf {{image_dir}}

distclean: clean
    if [ -f Cargo.lock ]; then rm Cargo.lock; fi
    if [ -d logs ]; then rm logs/*.txt; fi