#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/scripts/lib/signing-policy.sh"
unset CATTEN_TRUST_MODE CLUSTER_SIGN_PRIVATE_KEY CLUSTER_SIGN_KEY_FILE
catten_require_development_trust
[ "$CATTEN_TRUST_MODE" = development ]
[ "$(catten_signing_key_file "$ROOT")" = "$ROOT/tools/cluster-sign/dev-key.hex" ]
for mode in production unknown ''; do
    if (export CATTEN_TRUST_MODE="$mode"; catten_require_development_trust) 2>/dev/null; then
        echo "unexpected accepted trust mode" >&2
        exit 1
    fi
done
TRACE="$(CLUSTER_SIGN_PRIVATE_KEY=private-value-must-not-appear bash -x -c \
    'source "$1/scripts/lib/signing-policy.sh"; catten_require_development_trust' \
    test "$ROOT" 2>&1 || true)"
if [[ "$TRACE" == *private-value-must-not-appear* ]]; then
    echo "retired key value appeared in shell trace" >&2
    exit 1
fi
[[ "$TRACE" == *'CLUSTER_SIGN_PRIVATE_KEY is retired'* ]]
for script in run-aarch64.sh run-x86_64.sh build-catten-services.sh \
    build-catten-services-x86_64.sh build-catten-user.sh sign-service-elfs.sh; do
    OUTPUT="$(CATTEN_TRUST_MODE=production bash "$ROOT/scripts/$script" 2>&1 || true)"
    [[ "$OUTPUT" == *'production images are disabled'* ]]
done
echo "signing policy tests pass (defaults, fail-closed modes, traced retired variable, early runner/build rejection)"
