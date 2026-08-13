#!/usr/bin/env bash
set -euo pipefail
script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "${script_dir}/.." && pwd)"
cd -- "$repo_root"
command -v docker >/dev/null || { echo 'Docker is required' >&2; exit 1; }
docker info >/dev/null
image_ref="${LATEX_CORE_TEXLIVE_IMAGE:-latex-core-texlive:2026-m7}"
image_id="$(docker image inspect "$image_ref" --format '{{.Id}}')"
[[ "$image_id" =~ ^sha256:[0-9a-f]{64}$ ]] || { echo 'immutable local image ID required' >&2; exit 1; }
export LATEX_CORE_TEXLIVE_IMAGE="$image_id"
cargo test -p compiler --features docker-tests --test docker_compile -- --nocapture
