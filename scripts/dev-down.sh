#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd -- "${script_dir}/.." && pwd)"
compose_file="${repository_root}/deploy/compose/docker-compose.yml"
# shellcheck source=scripts/install-common.sh
source "$repository_root/scripts/install-common.sh"
latex_core_select_docker "$repository_root"

env POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-latex_core_dev_password}" \
  TEX_ENVIRONMENT_ID="${TEX_ENVIRONMENT_ID:-development-env}" \
  COMPILER_IMAGE="${COMPILER_IMAGE:-repository@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}" \
  "${LATEX_CORE_DOCKER[@]}" compose -p latex-core-dev -f "${compose_file}" down
echo "PostgreSQL development database stopped; data volume preserved."
