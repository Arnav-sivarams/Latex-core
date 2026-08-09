#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd -- "${script_dir}/.." && pwd)"
compose_file="${repository_root}/deploy/compose/docker-compose.yml"

docker compose -p latex-core-dev -f "${compose_file}" down
echo "PostgreSQL development database stopped; data volume preserved."
