#!/usr/bin/env bash
# Workspace-local host tools for disposable QEMU trust tests, not guest custody.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS="$ROOT/target/secure-boot-tools"
python3 -c 'import sys; assert sys.version_info >= (3, 11), "Python 3.11 or newer required"'
"$ROOT/scripts/verify-limine.sh"
mkdir -p "$TOOLS"
python3 -m venv "$TOOLS/venv"
"$TOOLS/venv/bin/python" -m pip install --no-cache-dir --only-binary=:all: \
    --require-hashes -r "$ROOT/scripts/secure-boot-test-requirements.txt"

ARCHIVE="$TOOLS/osslsigncode-2.14.tar.gz"
if [ ! -f "$ARCHIVE" ]; then
    curl -fL https://github.com/mtrojnar/osslsigncode/archive/refs/tags/2.14.tar.gz \
        -o "$ARCHIVE"
fi
python3 - "$ARCHIVE" "$TOOLS" <<'PY'
import hashlib
from pathlib import Path
import sys
import tarfile

archive = Path(sys.argv[1])
expected = '0f033fd6069387d2e489fbd2187e62f624764eb8c2758ee94e3e793e5150b5c5'
if hashlib.sha256(archive.read_bytes()).hexdigest() != expected:
    raise SystemExit('osslsigncode source SHA-256 mismatch; refusing to build')
with tarfile.open(archive) as source:
    source.extractall(sys.argv[2], filter='data')
PY
OPENSSL_ARGS=()
if [ -n "${CATTEN_TEST_OPENSSL_ROOT:-}" ]; then
    OPENSSL_ARGS+=("-DOPENSSL_ROOT_DIR=$CATTEN_TEST_OPENSSL_ROOT")
elif command -v brew >/dev/null 2>&1; then
    OPENSSL_ARGS+=("-DOPENSSL_ROOT_DIR=$(brew --prefix openssl@3)")
fi
cmake -S "$TOOLS/osslsigncode-2.14" -B "$TOOLS/osslsigncode-build" \
    "${OPENSSL_ARGS[@]}" "-DBASH_COMPLETION_USER_DIR=$TOOLS/completions"
cmake --build "$TOOLS/osslsigncode-build" -j 4
"${CC:-cc}" -O2 -std=c99 "$ROOT/limine-binary/limine.c" -o "$TOOLS/limine"
echo "Secure Boot test tools prepared in $TOOLS; no system installation."
