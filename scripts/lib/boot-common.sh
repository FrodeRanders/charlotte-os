#!/usr/bin/env bash
# Shared helpers for CharlotteOS boot and image tooling.
# Source this file; do not execute it directly.

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
    echo "error: scripts/lib/boot-common.sh must be sourced" >&2
    exit 2
fi

catten_boot_init() {
    local root_dir="$1"

    if [ ! -d "$root_dir/limine-binary" ] || [ ! -x "$root_dir/scripts/verify-limine.sh" ]; then
        echo "error: invalid CharlotteOS root: ${root_dir}" >&2
        return 1
    fi

    CATTEN_BOOT_ROOT_DIR="$(cd "$root_dir" && pwd)"
    CATTEN_BOOT_LIMINE_CONFIG="${CATTEN_LIMINE_CONFIG:-${CATTEN_BOOT_ROOT_DIR}/limine.conf}"
    if [[ "$CATTEN_BOOT_LIMINE_CONFIG" != /* ]]; then
        CATTEN_BOOT_LIMINE_CONFIG="${CATTEN_BOOT_ROOT_DIR}/${CATTEN_BOOT_LIMINE_CONFIG}"
    fi
    if [ ! -f "$CATTEN_BOOT_LIMINE_CONFIG" ]; then
        echo "error: Limine configuration not found: ${CATTEN_BOOT_LIMINE_CONFIG}" >&2
        return 1
    fi

    "${CATTEN_BOOT_ROOT_DIR}/scripts/verify-limine.sh"
}

catten_boot_require_commands() {
    local command_name
    local missing=0

    for command_name in "$@"; do
        if ! command -v "$command_name" >/dev/null 2>&1; then
            echo "error: required command not found: ${command_name}" >&2
            missing=1
        fi
    done
    if [ "$missing" = "1" ]; then
        echo "       install the missing boot tooling using your platform package manager" >&2
        return 1
    fi
}

catten_boot_validate_positive_integer() {
    local option_name="$1"
    local value="$2"

    if ! [[ "$value" =~ ^[0-9]+$ ]] || [ "$value" -lt 1 ]; then
        echo "error: ${option_name} must be a positive integer" >&2
        return 1
    fi
}

catten_boot_validate_port() {
    local option_name="$1"
    local value="$2"

    if ! [[ "$value" =~ ^[0-9]+$ ]] || [ "$value" -lt 1 ] || [ "$value" -gt 65535 ]; then
        echo "error: ${option_name} must be an integer from 1 through 65535" >&2
        return 1
    fi
}

catten_boot_validate_instance() {
    local value="$1"

    if [ -n "$value" ] && [[ ! "$value" =~ ^[A-Za-z0-9._-]+$ ]]; then
        echo "error: --instance may contain only letters, digits, '.', '_' and '-'" >&2
        return 1
    fi
}

catten_boot_sha256() {
    local path="$1"

    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$path" | awk '{print $1}'
    else
        echo "error: sha256sum or shasum is required" >&2
        return 1
    fi
}

catten_boot_sha256_stdin() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 | awk '{print $1}'
    else
        echo "error: sha256sum or shasum is required" >&2
        return 1
    fi
}

catten_boot_find_llvm_nm() {
    local rust_host
    local rust_llvm_nm

    if [ -n "${LLVM_NM:-}" ] && [ -x "$LLVM_NM" ]; then
        printf '%s\n' "$LLVM_NM"
        return 0
    fi
    if command -v llvm-nm >/dev/null 2>&1; then
        command -v llvm-nm
        return 0
    fi
    if command -v rustc >/dev/null 2>&1; then
        rust_host="$(rustc -vV | awk '/^host:/ {print $2}')"
        rust_llvm_nm="$(rustc --print sysroot)/lib/rustlib/${rust_host}/bin/llvm-nm"
        if [ -x "$rust_llvm_nm" ]; then
            printf '%s\n' "$rust_llvm_nm"
            return 0
        fi
    fi
    return 1
}

catten_boot_write_kernel_symbols() {
    local kernel="$1"
    local kernel_sha256="$2"
    local llvm_nm
    local symbol_map="${kernel}.symbols"
    local archive_dir="${CATTEN_BOOT_ROOT_DIR}/target/kernel-symbols"
    local archived_map="${archive_dir}/${kernel_sha256}.symbols"
    local temporary_map

    if ! llvm_nm="$(catten_boot_find_llvm_nm)"; then
        rm -f -- "$symbol_map"
        echo "warning: llvm-nm is unavailable; kernel symbol map was not generated" >&2
        echo "         install rustup's llvm-tools-preview component or set LLVM_NM" >&2
        return 0
    fi

    temporary_map="$(mktemp "${symbol_map}.tmp.XXXXXX")" || return 1
    if ! {
        printf '# CharlotteOS kernel symbol map v1\n'
        printf '# kernel-sha256: %s\n' "$kernel_sha256"
        printf '# columns: address size type demangled-symbol\n'
        "$llvm_nm" --defined-only --numeric-sort --print-size --demangle "$kernel"
    } >"$temporary_map"; then
        rm -f -- "$temporary_map"
        rm -f -- "$symbol_map"
        echo "warning: failed to generate kernel symbol map with ${llvm_nm}" >&2
        return 0
    fi

    mv -f -- "$temporary_map" "$symbol_map"
    mkdir -p "$archive_dir"
    cp -f -- "$symbol_map" "$archived_map"
    echo ">>> Kernel symbols: ${symbol_map}"
    echo ">>> Archived symbols: ${archived_map}"
}

catten_boot_report_kernel() {
    local kernel="$1"
    local kernel_sha256

    if [ ! -f "$kernel" ]; then
        echo "error: kernel payload does not exist: ${kernel}" >&2
        return 1
    fi
    kernel_sha256="$(catten_boot_sha256 "$kernel")" || return 1
    echo ">>> Kernel payload: ${kernel}"
    echo ">>> Kernel SHA-256: ${kernel_sha256}"
    catten_boot_write_kernel_symbols "$kernel" "$kernel_sha256"
}

catten_boot_record_log_kernel() {
    local log="$1"
    local kernel="$2"
    local symbol_map="${kernel}.symbols"
    local kernel_sha256

    kernel_sha256="$(catten_boot_sha256 "$kernel")" || return 1
    printf '%s\n' "$kernel_sha256" >"${log}.kernel.sha256"
    rm -f -- "${log}.symbols"
    if [ -f "$symbol_map" ]; then
        cp -f -- "$symbol_map" "${log}.symbols"
    fi
}

catten_boot_bundle_sha256() (
    local bundle_dir="$1"
    local service_elf
    local service_digest
    local digest_lines=""
    local service_files=()

    LC_ALL=C
    shopt -s nullglob
    service_files=("$bundle_dir"/*.elf)
    if [ "${#service_files[@]}" -eq 0 ]; then
        echo "error: service bundle contains no ELF files: ${bundle_dir}" >&2
        return 1
    fi

    for service_elf in "${service_files[@]}"; do
        service_digest="$(catten_boot_sha256 "$service_elf")" || return 1
        digest_lines="${digest_lines}$(basename "$service_elf"):${service_digest}
"
    done
    printf '%s' "$digest_lines" | catten_boot_sha256_stdin
)

catten_boot_create_uefi_image() {
    local image="$1"
    local size_mib="$2"
    local efi_boot_file="$3"
    local kernel="$4"
    local limine_config="$5"
    local volume_label="${6:-}"
    local image_dir
    local temporary_image
    local mformat_args=(-i)

    catten_boot_validate_positive_integer "boot image size" "$size_mib"
    catten_boot_require_commands dd mformat mmd mcopy

    for required_file in "$efi_boot_file" "$kernel" "$limine_config"; do
        if [ ! -f "$required_file" ]; then
            echo "error: boot-image input does not exist: ${required_file}" >&2
            return 1
        fi
    done

    image_dir="$(dirname "$image")"
    mkdir -p "$image_dir"
    temporary_image="$(mktemp "${image}.tmp.XXXXXX")"
    mformat_args+=("$temporary_image" -F)
    if [ -n "$volume_label" ]; then
        mformat_args+=(-v "$volume_label")
    fi
    mformat_args+=(::)

    echo ">>> Creating boot image ${image}..."
    if ! {
        dd if=/dev/zero of="$temporary_image" bs=1048576 count="$size_mib" status=none \
            && mformat "${mformat_args[@]}" \
            && mmd -i "$temporary_image" ::/EFI \
            && mmd -i "$temporary_image" ::/EFI/BOOT \
            && mcopy -i "$temporary_image" "$efi_boot_file" "::/EFI/BOOT/$(basename "$efi_boot_file")" \
            && mcopy -i "$temporary_image" "$kernel" ::/catten \
            && mcopy -i "$temporary_image" "$limine_config" ::/limine.conf \
            && chmod 0644 "$temporary_image"
    }; then
        rm -f -- "$temporary_image"
        echo "error: failed to create boot image ${image}" >&2
        return 1
    fi

    if ! mv -f -- "$temporary_image" "$image"; then
        rm -f -- "$temporary_image"
        return 1
    fi
}

catten_boot_validate_selftest_log() {
    local log="$1"
    local kernel="${2:-}"

    if [ ! -f "$log" ]; then
        echo "error: serial log does not exist: ${log}" >&2
        return 1
    fi
    if grep -Fq "Kernel panic:" "$log"; then
        echo "error: kernel panic observed during the test window" >&2
        if [ -n "$kernel" ] && command -v python3 >/dev/null 2>&1; then
            "${CATTEN_BOOT_ROOT_DIR}/scripts/symbolize-kernel-panic.py" \
                --kernel "$kernel" "$log" || true
        fi
        return 1
    fi
    if ! grep -Eq \
        'SELFTEST COMPLETE: passed=[0-9]+ failed=0 pending=0 passed_bitmap=0x[0-9a-f]+ failed_bitmap=0x0 pending_bitmap=0x0' \
        "$log"
    then
        echo "error: malformed or unsuccessful authoritative self-test result" >&2
        grep -E 'SELFTEST (FAILED|PENDING):' "$log" >&2 || true
        return 1
    fi
    echo ">>> All registered deferred self-tests passed."
}
