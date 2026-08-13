#!/usr/bin/env bash
set -euo pipefail
image='latex-core-texlive:2026-m7'
expected='sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38'
output="${1:?usage: export-compiler-image.sh OUTPUT.tar}"
actual="$(docker image inspect "$image" --format '{{.Id}}')"
[[ "$actual" == "$expected" ]] || { echo "expected $expected, got $actual" >&2; exit 1; }
docker save --output "$output" "$image"
printf 'Verified compiler image exported to %s\n' "$output"
