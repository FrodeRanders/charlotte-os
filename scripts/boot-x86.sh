#!/usr/bin/env bash
# Convenience wrapper around the maintained macOS/Linux x86 runner.
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
exec "$PROJECT_DIR/scripts/run-x86_64.sh" \
    --instance boot-x86 \
    --smp 1 \
    --timeout 90 \
    "$@"
