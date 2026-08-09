#!/usr/bin/env bash
set -euo pipefail

script_dir="$(CDPATH='' cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repository_root="$(cd -- "${script_dir}/.." && pwd)"
compose_file="${repository_root}/deploy/compose/docker-compose.yml"
postgres_port="${LATEX_CORE_POSTGRES_PORT:-54329}"

"${repository_root}/scripts/dev-up.sh"

docker compose -p latex-core-dev -f "${compose_file}" exec -T postgres \
  psql -U latex_core -d postgres -v ON_ERROR_STOP=1 \
  -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = 'latex_core_test' AND pid <> pg_backend_pid();" \
  -c "DROP DATABASE IF EXISTS latex_core_test;" \
  -c "CREATE DATABASE latex_core_test;"

export TEST_DATABASE_URL="postgresql://latex_core:latex_core_dev_password@127.0.0.1:${postgres_port}/latex_core_test"
cd -- "${repository_root}"
cargo test -p persistence --features database-tests --test postgres -- --nocapture
