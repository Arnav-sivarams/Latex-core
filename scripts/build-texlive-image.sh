#!/usr/bin/env bash
set -euo pipefail
script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
cd -- "$repo_root"
command -v docker >/dev/null || { echo 'Docker is required' >&2; exit 1; }
docker info >/dev/null
checksum='docker/texlive/texlive2026-20260301.iso.sha512'
grep -Eq '^[0-9a-f]{128}  texlive2026-20260301\.iso$' "$checksum" || { echo 'invalid checksum file' >&2; exit 1; }
[[ "$(wc -l < "$checksum")" -eq 1 ]] || { echo 'checksum file must contain one line' >&2; exit 1; }
cargo build --release -p tex-index --bin tex-index
docker build --tag latex-core-texlive:2026-m7 --file docker/texlive/Dockerfile .
image_id="$(docker image inspect latex-core-texlive:2026-m7 --format '{{.Id}}')"
[[ "$image_id" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo 'Docker did not return an immutable image ID' >&2; exit 1; }
test_output="$("$repo_root/scripts/test-texlive-image.sh" "$image_id")"
printf '%s\n' "$test_output"
tex_environment_id="$(sed -n 's/^TEX_ENVIRONMENT_ID=//p' <<<"$test_output")"
[[ "$tex_environment_id" =~ ^texlive-2026-sha256-[0-9a-f]{64}$ ]] || { echo 'invalid TeX environment ID' >&2; exit 1; }
printf 'IMAGE_ID=%s\nTEX_ENVIRONMENT_ID=%s\n' "$image_id" "$tex_environment_id"
