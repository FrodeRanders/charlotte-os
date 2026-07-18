#!/bin/bash
# Quick single-LP self-test boot — delegates to run-aarch64.sh.
set -euo pipefail
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
exec "$PROJECT_DIR/scripts/run-aarch64.sh" --smp 1 --timeout 60 "$@"
