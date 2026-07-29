#!/usr/bin/env bash
set -euo pipefail

if [[ $# -gt 1 ]]; then
    echo "usage: $0 [path-to-tla2tools.jar]" >&2
    exit 2
fi

tools_jar="${1:-${TLA2TOOLS_JAR:-}}"
if [[ -z "${tools_jar}" || ! -f "${tools_jar}" ]]; then
    echo "error: pass tla2tools.jar as the first argument or set TLA2TOOLS_JAR" >&2
    exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
model_dir="$(mktemp -d "${TMPDIR:-/tmp}/charlotte-tla.XXXXXX")"
trap 'rm -rf "${model_dir}"' EXIT

run_model() {
    local module="$1"
    local config="$2"
    shift 2
    local log="${model_dir}/${module}.log"

    echo ">>> TLC ${module} (${config})"
    if ! java -XX:+UseParallelGC -cp "${tools_jar}" tlc2.TLC \
            "${module}" \
            -config "${config}" \
            -workers auto \
            -metadir "${model_dir}/${module}-states" \
            -coverage 1 >"${log}" 2>&1; then
        cat "${log}" >&2
        exit 1
    fi

    if grep -Eq '^Warning: (The variable|The EXCEPT|Successor state)' "${log}"; then
        echo "error: TLC reported a model-structure warning for ${module}" >&2
        exit 1
    fi

    local action
    for action in "$@"; do
        if ! grep -Eq "^<${action} .*: [1-9][0-9]*:[1-9][0-9]*$" "${log}"; then
            echo "error: required action ${action} had no TLC coverage in ${module}" >&2
            exit 1
        fi
        grep -E "^<${action} " "${log}" | tail -n 1
    done
    grep -E 'states generated,|The depth of the complete state graph' "${log}"
}

cd "${script_dir}"
run_model CharlotteIPC CharlotteIPC_small.cfg \
    MemoryCreate ScalarCallMove ScalarCallBorrowRead ScalarCallBorrowWrite \
    ScalarCallCopy ReplyReturnMemory ReplyRevokeBorrow EndpointClose \
    DomainTeardown
run_model CharlotteCQ CharlotteCQ_mini.cfg \
    Complete Fail CancelOp DrainOne DrainAll CqWait CqWake TimerFire
