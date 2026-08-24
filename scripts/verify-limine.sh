#!/usr/bin/env bash
# Verify or restore the exact Limine binary release used by CharlotteOS.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR_DIR="${ROOT_DIR}/limine-binary"
VERSION_FILE="${VENDOR_DIR}/VERSION"
CHECKSUM_FILE="${VENDOR_DIR}/SHA256SUMS"

usage() {
    echo "usage: scripts/verify-limine.sh [--release|--restore]" >&2
    echo "  no option   verify the checked-in files without network access" >&2
    echo "  --release   also compare them with the pinned official archive" >&2
    echo "  --restore   restore them from the verified official archive" >&2
}

case "${1:-}" in
    "") MODE="local" ;;
    --release) MODE="release" ;;
    --restore) MODE="restore" ;;
    -h|--help) usage; exit 0 ;;
    *) usage; exit 2 ;;
esac
[ "$#" -le 1 ] || { usage; exit 2; }

if [ ! -f "$VERSION_FILE" ] || [ ! -f "$CHECKSUM_FILE" ]; then
    echo "error: Limine provenance metadata is missing from ${VENDOR_DIR}" >&2
    exit 1
fi

# VERSION is a tracked file containing only namespaced constant assignments.
# shellcheck source=../limine-binary/VERSION
source "$VERSION_FILE"
: "${LIMINE_VERSION:?missing LIMINE_VERSION}"
: "${LIMINE_ARCHIVE:?missing LIMINE_ARCHIVE}"
: "${LIMINE_ARCHIVE_SHA256:?missing LIMINE_ARCHIVE_SHA256}"
: "${LIMINE_RELEASE_URL:?missing LIMINE_RELEASE_URL}"

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    else
        echo "error: sha256sum or shasum is required" >&2
        return 1
    fi
}

verify_local() {
    local expected path actual failed=0
    while read -r expected path; do
        [ -n "$expected" ] || continue
        if [ ! -f "${ROOT_DIR}/${path}" ]; then
            echo "error: missing ${path}" >&2
            failed=1
            continue
        fi
        actual="$(sha256_of "${ROOT_DIR}/${path}")"
        if [ "$actual" != "$expected" ]; then
            echo "error: checksum mismatch for ${path}" >&2
            echo "       expected ${expected}" >&2
            echo "       actual   ${actual}" >&2
            failed=1
        fi
    done < "$CHECKSUM_FILE"
    [ "$failed" = "0" ]
}

if [ "$MODE" = "local" ]; then
    verify_local
    echo ">>> Limine ${LIMINE_VERSION}: checked-in files verified"
    exit 0
fi

command -v curl >/dev/null 2>&1 || { echo "error: curl is required" >&2; exit 1; }
command -v tar >/dev/null 2>&1 || { echo "error: tar is required" >&2; exit 1; }

VERIFY_TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/charlotte-limine.XXXXXX")"
cleanup() {
    case "$VERIFY_TMP_DIR" in
        "${TMPDIR:-/tmp}"/charlotte-limine.*) rm -rf -- "$VERIFY_TMP_DIR" ;;
    esac
}
trap cleanup EXIT

ARCHIVE_PATH="${VERIFY_TMP_DIR}/${LIMINE_ARCHIVE}"
echo ">>> Downloading Limine ${LIMINE_VERSION}"
curl -L --fail --silent --show-error -o "$ARCHIVE_PATH" "$LIMINE_RELEASE_URL"
ACTUAL_ARCHIVE_SHA256="$(sha256_of "$ARCHIVE_PATH")"
if [ "$ACTUAL_ARCHIVE_SHA256" != "$LIMINE_ARCHIVE_SHA256" ]; then
    echo "error: official archive checksum mismatch" >&2
    echo "       expected ${LIMINE_ARCHIVE_SHA256}" >&2
    echo "       actual   ${ACTUAL_ARCHIVE_SHA256}" >&2
    exit 1
fi

mkdir -p "${VERIFY_TMP_DIR}/unpack"
tar -xf "$ARCHIVE_PATH" -C "${VERIFY_TMP_DIR}/unpack"
UPSTREAM_DIR="${VERIFY_TMP_DIR}/unpack/limine-binary"
[ -d "$UPSTREAM_DIR" ] || { echo "error: archive has no limine-binary directory" >&2; exit 1; }

# The checksum manifest must enumerate every payload file in the official
# archive. VERSION and SHA256SUMS are CharlotteOS metadata and intentionally do
# not occur in the upstream file list.
find "$UPSTREAM_DIR" -type f -print \
    | sed "s|^${VERIFY_TMP_DIR}/unpack/||" \
    | LC_ALL=C sort > "${VERIFY_TMP_DIR}/archive-files"
awk '{print $2}' "$CHECKSUM_FILE" \
    | LC_ALL=C sort > "${VERIFY_TMP_DIR}/manifest-files"
if ! diff -u "${VERIFY_TMP_DIR}/archive-files" "${VERIFY_TMP_DIR}/manifest-files"; then
    echo "error: SHA256SUMS does not describe the complete official archive" >&2
    exit 1
fi

if [ "$MODE" = "restore" ]; then
    while read -r _ path; do
        [ -n "$path" ] || continue
        source_path="${VERIFY_TMP_DIR}/unpack/${path}"
        [ -f "$source_path" ] || { echo "error: release is missing ${path}" >&2; exit 1; }
        mkdir -p "$(dirname "${ROOT_DIR}/${path}")"
        cp -p "$source_path" "${ROOT_DIR}/${path}"
    done < "$CHECKSUM_FILE"
fi

verify_local

while read -r _ path; do
    [ -n "$path" ] || continue
    if ! cmp -s "${ROOT_DIR}/${path}" "${VERIFY_TMP_DIR}/unpack/${path}"; then
        echo "error: ${path} differs from the pinned official release" >&2
        exit 1
    fi
done < "$CHECKSUM_FILE"

echo ">>> Limine ${LIMINE_VERSION}: official release and checked-in files verified"
