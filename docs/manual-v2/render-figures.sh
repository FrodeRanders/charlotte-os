#!/usr/bin/env bash
set -euo pipefail

manual_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd -- "$manual_dir/../.." && pwd)"
source_file="$repo_dir/docs/figures.md"
output_dir="$manual_dir/figures"
config_file="$output_dir/mermaid-config.json"

names=(
    system-layering
    kernel-userspace-boundary
    service-composition
    boot-and-testing
    capability-safe-ipc
    two-node-cluster
    external-service-capabilities
    release-admission-rollout
    operational-profile-pickup
    role-separated-deployment-trust
    durga-charlotte-generation
)

command -v mmdc >/dev/null 2>&1 || {
    echo "error: Mermaid CLI (mmdc) is required" >&2
    exit 1
}

mkdir -p "$output_dir"
render_tmp="$(mktemp -d "${TMPDIR:-/tmp}/charlotte-mermaid.XXXXXX")"
trap 'rm -rf -- "$render_tmp"' EXIT

for index in "${!names[@]}"; do
    block=$((index + 1))
    input="$render_tmp/${names[$index]}.mmd"
    awk -v wanted="$block" '
        /^```mermaid$/ { current++; capture = current == wanted; next }
        /^```$/ && capture { exit }
        capture { print }
    ' "$source_file" > "$input"
    if [[ ! -s "$input" ]]; then
        echo "error: Mermaid block $block is missing from $source_file" >&2
        exit 1
    fi
    mmdc --quiet \
        --configFile "$config_file" \
        --backgroundColor white \
        --width 2400 \
        --height 1600 \
        --input "$input" \
        --output "$output_dir/${names[$index]}.svg"

    output="$output_dir/${names[$index]}.svg"
    if rg --quiet '<foreignObject' "$output"; then
        echo "error: $output contains HTML labels that are not portable to LaTeX" >&2
        exit 1
    fi
    # Mermaid emits native labels as adjacent tspans with significant leading
    # spaces. Preserve those spaces in SVG renderers such as Inkscape and librsvg.
    perl -0pi -e 's/<svg /<svg xml:space="preserve" /' "$output"
done

echo "Rendered ${#names[@]} SVG figures in $output_dir"
