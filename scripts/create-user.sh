#!/usr/bin/env bash
set -euo pipefail
email="${1:?usage: create-user.sh EMAIL [PASSWORD]}"
args=(user create --email "${email}")
if [[ $# -gt 1 ]]; then args+=(--password "$2"); fi
docker compose -f deploy/compose/docker-compose.yml exec -T api /usr/local/bin/latex-core-admin "${args[@]}"
