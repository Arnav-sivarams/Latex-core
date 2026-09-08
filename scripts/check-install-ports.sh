#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"
latex_core_init "$root"

check_port() {
  local label="$1" port="$2" published foreign=false
  published="$(docker ps --filter "publish=$port" --format '{{.ID}} {{.Label "com.docker.compose.project"}} {{.Names}}')"
  if [[ -n "$published" ]]; then
    while IFS=' ' read -r _id project _name; do
      [[ "$project" == "$LATEX_CORE_PROJECT" ]] || foreign=true
    done <<<"$published"
    if [[ "$foreign" == true ]]; then
      echo "$label port $port is already published by another Docker container; no service was stopped." >&2
      return 1
    fi
  fi
  if ss -H -ltn "sport = :$port" | grep -q . && [[ -z "$published" ]]; then
    echo "$label port $port is already occupied by a host process; no service was stopped." >&2
    return 1
  fi
}

http_port="$(latex_core_env_value HTTP_PORT "$LATEX_CORE_ENV_FILE")"
postgres_port="$(latex_core_env_value LATEX_CORE_POSTGRES_PORT "$LATEX_CORE_ENV_FILE")"
check_port HTTP "$http_port"
check_port PostgreSQL "$postgres_port"
echo 'Configured host ports are available or already belong to this Compose project.'
