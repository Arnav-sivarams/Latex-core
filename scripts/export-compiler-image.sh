#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"
latex_core_select_docker "$root"
image='latex-core-texlive:2026-m7'
expected='sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38'
output="${1:?usage: export-compiler-image.sh OUTPUT.tar}"
actual="$(latex_core_docker image inspect "$image" --format '{{.Id}}')"
[[ "$actual" == "$expected" ]] || { echo "expected $expected, got $actual" >&2; exit 1; }
latex_core_docker save --output "$output" "$image"
printf 'Verified compiler image exported to %s\n' "$output"
