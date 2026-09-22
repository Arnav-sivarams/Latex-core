#!/usr/bin/env bash

# Shared deployment identity. Callers enable strict mode before sourcing this file.

latex_core_env_value() {
  local key="$1" file="$2"
  awk -v wanted="$key" '
    /^[[:space:]]*(#|$)/ { next }
    {
      line=$0
      sub(/^[[:space:]]*/, "", line)
      split_at=index(line, "=")
      if (!split_at) next
      name=substr(line, 1, split_at-1)
      sub(/[[:space:]]*$/, "", name)
      if (name != wanted) next
      value=substr(line, split_at+1)
      sub(/^[[:space:]]*/, "", value)
      sub(/[[:space:]]*$/, "", value)
      if ((substr(value,1,1) == "\"" && substr(value,length(value),1) == "\"") ||
          (substr(value,1,1) == "\047" && substr(value,length(value),1) == "\047")) {
        value=substr(value,2,length(value)-2)
      }
      found=value
    }
    END { if (found != "") print found }
  ' "$file"
}

latex_core_init() {
  local repository_root="$1"
  LATEX_CORE_ROOT="$(cd "$repository_root" && pwd)"
  LATEX_CORE_ENV_FILE="$LATEX_CORE_ROOT/.env"
  [[ -f "$LATEX_CORE_ENV_FILE" ]] || {
    echo "LaTeX Core is not installed: $LATEX_CORE_ENV_FILE is missing. Run ./latex-core install from this checkout." >&2
    return 1
  }
  LATEX_CORE_PROJECT="$(latex_core_env_value COMPOSE_PROJECT_NAME "$LATEX_CORE_ENV_FILE")"
  LATEX_CORE_PROJECT="${LATEX_CORE_PROJECT:-latex-core}"
  [[ "$LATEX_CORE_PROJECT" =~ ^[a-z0-9][a-z0-9_-]{1,62}$ ]] || {
    echo 'COMPOSE_PROJECT_NAME must use 2-63 lowercase letters, numbers, dashes, or underscores.' >&2
    return 1
  }
  LATEX_CORE_COMPOSE=(
    docker compose
    --project-name "$LATEX_CORE_PROJECT"
    --env-file "$LATEX_CORE_ENV_FILE"
    -f "$LATEX_CORE_ROOT/deploy/compose/docker-compose.yml"
  )
}

latex_core_container_id() {
  "${LATEX_CORE_COMPOSE[@]}" ps -aq "$1" 2>/dev/null | head -n1
}

latex_core_prepare_staging() {
  local staging parent
  staging="$(latex_core_env_value WORKER_STAGING_HOST_ROOT "$LATEX_CORE_ENV_FILE")"
  [[ -n "$staging" ]] || { echo 'WORKER_STAGING_HOST_ROOT is missing.' >&2; return 1; }
  if [[ -e "$staging" && ! -d "$staging" ]]; then
    echo "Worker staging path is not a directory: $staging. Set WORKER_STAGING_HOST_ROOT to a writable dedicated directory." >&2
    return 1
  fi
  parent="$staging"
  while [[ ! -e "$parent" ]]; do parent="$(dirname "$parent")"; done
  if [[ ! -w "$parent" || ( -d "$staging" && ! -w "$staging" ) ]]; then
    echo "Worker staging path is not writable: $staging. Set WORKER_STAGING_HOST_ROOT to a writable dedicated directory." >&2
    return 1
  fi
  mkdir -p -- "$staging" || { echo "Could not create worker staging path: $staging" >&2; return 1; }
  [[ -w "$staging" ]] || { echo "Worker staging path is not writable: $staging" >&2; return 1; }
  chmod 700 -- "$staging"
}
