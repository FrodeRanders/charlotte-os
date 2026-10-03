#!/usr/bin/env bash
# Signing configuration carries paths, never secret bytes. Source this file.

catten_require_development_trust() {
    case "${CATTEN_TRUST_MODE-development}" in
        development) export CATTEN_TRUST_MODE=development ;;
        production)
            echo "error: production images are disabled until protected trust/recipient provisioning is implemented; do not use fixture keys for real credentials" >&2
            return 1 ;;
        *) echo "error: CATTEN_TRUST_MODE must be development or production" >&2; return 1 ;;
    esac
    # Test presence without expanding a possibly sensitive old value, even
    # under bash -x. Do not migrate it through argv or a temporary shell file.
    if [ "${CLUSTER_SIGN_PRIVATE_KEY+x}" = x ]; then
        echo "error: CLUSTER_SIGN_PRIVATE_KEY is retired; unset it and set CLUSTER_SIGN_KEY_FILE to a restricted key-file path" >&2
        return 1
    fi
}

catten_signing_key_file() {
    local root_dir="$1"
    local key_file="${CLUSTER_SIGN_KEY_FILE:-${root_dir}/tools/cluster-sign/dev-key.hex}"
    catten_require_development_trust || return 1
    case "$key_file" in /*) ;; *) key_file="$PWD/$key_file" ;; esac
    if [ ! -f "$key_file" ]; then
        echo "error: signing-key file is missing (set CLUSTER_SIGN_KEY_FILE)" >&2
        return 1
    fi
    printf '%s\n' "$key_file"
}
