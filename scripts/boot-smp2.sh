#!/bin/bash
# SMP-2 boot — delegates to run-aarch64.sh.
set -euo pipefail
PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
exec "$PROJECT_DIR/scripts/run-aarch64.sh" --smp 2 --timeout 90 "$@"
