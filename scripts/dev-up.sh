#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd -- "${script_dir}/.." && pwd)"
compose_file="${repository_root}/deploy/compose/docker-compose.yml"
postgres_port="${LATEX_CORE_POSTGRES_PORT:-54329}"

docker compose -p latex-core-dev -f "${compose_file}" up -d --wait postgres

echo "PostgreSQL development database is ready."
echo "postgresql://latex_core:latex_core_dev_password@127.0.0.1:${postgres_port}/latex_core_dev"
