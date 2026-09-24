#!/usr/bin/env bash

# Shared deployment identity. Callers enable strict mode before sourcing this file.

latex_core_validate_invoking_identity() {
  local repository_root="$1" expected_uid="$2" expected_gid="$3" expected_user="$4"
  local passwd_entry account _ uid gid home _shell checkout_owner
  [[ "$expected_uid" =~ ^[0-9]+$ && "$expected_gid" =~ ^[0-9]+$ && "$expected_uid" -gt 0 ]] || return 1
  passwd_entry="$(getent passwd "$expected_uid" || true)"
  [[ -n "$passwd_entry" ]] || return 1
  IFS=: read -r account _ uid gid _ home _shell <<<"$passwd_entry"
  [[ "$uid" == "$expected_uid" && "$gid" == "$expected_gid" && "$account" == "$expected_user" && "$home" == /* && -d "$home" ]] || return 1
  checkout_owner="$(stat -Lc '%u' -- "$repository_root")"
  [[ "$checkout_owner" == "$uid" ]] || return 1
  printf '%s:%s\n' "$account" "$home"
}

latex_core_resolve_executable() {
  local executable
  executable="$(command -v "$1" 2>/dev/null || true)"
  [[ -n "$executable" ]] || return 1
  readlink -f -- "$executable"
}

latex_core_validate_privileged_executable() {
  local executable="$1" owner mode
  [[ "$executable" == /* && -x "$executable" ]] || return 1
  if [[ "${LATEX_CORE_INSTALL_TESTING:-}" == 1 ]]; then return 0; fi
  owner="$(stat -Lc '%u' -- "$executable")"
  mode="$(stat -Lc '%a' -- "$executable")"
  [[ "$owner" == 0 ]] || return 1
  (( (8#$mode & 8#022) == 0 ))
}

latex_core_set_docker_mode() {
  local repository_root="$1" mode="$2" docker_bin="$3"
  [[ "$mode" == direct || "$mode" == sudo ]] || return 1
  if [[ "$mode" == sudo ]] && ! latex_core_validate_privileged_executable "$docker_bin"; then
    echo "Refusing privileged Docker execution through an untrusted executable: $docker_bin" >&2
    return 1
  fi
  export LATEX_CORE_DOCKER_MODE="$mode" LATEX_CORE_DOCKER_BIN="$docker_bin"
  LATEX_CORE_DOCKER=("$repository_root/scripts/docker-exec.sh")
}

latex_core_select_docker() {
  local repository_root="$1" docker_bin direct_error privileged_error context endpoint
  repository_root="$(cd "$repository_root" && pwd -P)"
  docker_bin="${LATEX_CORE_DOCKER_BIN:-}"
  if [[ -z "$docker_bin" ]]; then
    docker_bin="$(latex_core_resolve_executable docker || true)"
  fi
  [[ -n "$docker_bin" ]] || { echo 'Docker is not installed.' >&2; return 1; }

  if [[ -n "${DOCKER_HOST:-}" && "$DOCKER_HOST" != unix:///var/run/docker.sock ]]; then
    echo "Unsupported DOCKER_HOST '$DOCKER_HOST'. LaTeX Core will not switch from a remote or alternate daemon to the local deployment daemon." >&2
    return 1
  fi

  context="$($docker_bin context show 2>/dev/null || true)"
  endpoint="$($docker_bin context inspect "$context" --format '{{.Endpoints.docker.Host}}' 2>/dev/null || true)"
  [[ "$endpoint" == unix:///var/run/docker.sock ]] || {
    echo "Unsupported Docker context '${context:-unknown}' (${endpoint:-unresolved}). LaTeX Core requires the local unix:///var/run/docker.sock daemon and will not silently switch contexts." >&2
    return 1
  }

  if [[ "${LATEX_CORE_DOCKER_MODE:-}" == direct || "${LATEX_CORE_DOCKER_MODE:-}" == sudo ]]; then
    latex_core_set_docker_mode "$repository_root" "$LATEX_CORE_DOCKER_MODE" "$docker_bin"
    "${LATEX_CORE_DOCKER[@]}" info >/dev/null
    return
  fi

  direct_error="$(mktemp)"
  if "$docker_bin" info >/dev/null 2>"$direct_error"; then
    rm -f -- "$direct_error"
    latex_core_set_docker_mode "$repository_root" direct "$docker_bin"
    return
  fi

  latex_core_set_docker_mode "$repository_root" sudo "$docker_bin" || { rm -f -- "$direct_error"; return 1; }
  privileged_error="$(mktemp)"
  if "${LATEX_CORE_DOCKER[@]}" info >/dev/null 2>"$privileged_error"; then
    rm -f -- "$direct_error" "$privileged_error"
    return
  fi
  echo 'Docker is installed, but neither direct nor sudo-assisted access reached the supported local daemon.' >&2
  if [[ -s "$direct_error" ]]; then sed -n '1,4p' "$direct_error" >&2; fi
  if [[ -s "$privileged_error" ]]; then sed -n '1,4p' "$privileged_error" >&2; fi
  rm -f -- "$direct_error" "$privileged_error"
  return 1
}

latex_core_docker() {
  "${LATEX_CORE_DOCKER[@]}" "$@"
}

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
  LATEX_CORE_ROOT="$(cd "$repository_root" && pwd -P)"
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
  latex_core_select_docker "$LATEX_CORE_ROOT"
  LATEX_CORE_COMPOSE=(
    "${LATEX_CORE_DOCKER[@]}" compose
    --project-name "$LATEX_CORE_PROJECT"
    --env-file "$LATEX_CORE_ENV_FILE"
    -f "$LATEX_CORE_ROOT/deploy/compose/docker-compose.yml"
  )
}

latex_core_container_id() {
  "${LATEX_CORE_COMPOSE[@]}" ps -aq "$1" 2>/dev/null | head -n1
}

latex_core_prepare_staging() {
  local staging parent created=false
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
  if [[ ! -d "$staging" ]]; then
    created=true
    (umask 077; mkdir -p -- "$staging") || { echo "Could not create worker staging path: $staging" >&2; return 1; }
  fi
  [[ -w "$staging" ]] || { echo "Worker staging path is not writable: $staging" >&2; return 1; }
  if [[ "$created" == true ]]; then chmod 700 -- "$staging"; fi
}
