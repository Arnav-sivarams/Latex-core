#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"

usage() {
  cat <<'EOF'
Usage: ./scripts/update-deployment.sh api [worker]
       ./scripts/update-deployment.sh worker [api]

Build and replace only the selected application services in an existing
deployment. Source selection, backups, and schema migrations remain explicit
operator steps.
EOF
}

(($#)) || { usage >&2; exit 2; }
declare -a services=()
for requested in "$@"; do
  case "$requested" in
    api|worker)
      for selected in "${services[@]:-}"; do
        [[ "$selected" != "$requested" ]] || { echo "Service selected more than once: $requested" >&2; exit 2; }
      done
      services+=("$requested")
      ;;
    *) echo "Unsupported update service: $requested (choose api and/or worker)." >&2; exit 2 ;;
  esac
done

latex_core_init "$root"
phase=preflight
replacement_started=false
evidence_root=''

save_failure_evidence() {
  local stamp service
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  evidence_root="$root/.install-diagnostics/update-$stamp-$$"
  umask 077
  mkdir -p "$evidence_root"
  chmod 700 "$root/.install-diagnostics" "$evidence_root" 2>/dev/null || true
  {
    printf 'source_commit=%s\nsource_branch=%s\ncompose_project=%s\nphase=%s\n' \
      "$(git -C "$root" rev-parse HEAD 2>/dev/null || printf unknown)" \
      "$(git -C "$root" branch --show-current 2>/dev/null || printf detached)" \
      "$LATEX_CORE_PROJECT" "$phase"
    "${LATEX_CORE_COMPOSE[@]}" ps --all
  } >"$evidence_root/status.txt" 2>&1 || true
  for service in postgres caddy "${services[@]}"; do
    "${LATEX_CORE_COMPOSE[@]}" logs --no-color --tail 200 "$service" >"$evidence_root/$service.log" 2>&1 || true
  done
  chmod 600 "$evidence_root"/* 2>/dev/null || true
}

failed() {
  local status=$?
  trap - EXIT
  if ((status != 0)); then
    save_failure_evidence
    echo "Update failed during phase: $phase (exit $status)." >&2
    if [[ "$replacement_started" == false ]]; then
      echo 'No application container replacement was requested.' >&2
    fi
    echo "Private bounded diagnostics: $evidence_root" >&2
  fi
  exit "$status"
}
trap failed EXIT

python3 "$root/scripts/validate-install-config.py" "$LATEX_CORE_ENV_FILE"
"${LATEX_CORE_COMPOSE[@]}" config --quiet
configured_services="$("${LATEX_CORE_COMPOSE[@]}" config --services)"
for required_service in postgres api worker caddy; do
  grep -qx "$required_service" <<<"$configured_services" || {
    echo "Selected Compose file is missing required service: $required_service" >&2
    exit 1
  }
done

postgres_id="$(latex_core_container_id postgres)"
[[ -n "$postgres_id" ]] || { echo 'Existing deployment PostgreSQL container is missing.' >&2; exit 1; }
[[ "$(docker inspect -f '{{.State.Status}}' "$postgres_id" 2>/dev/null || true)" == running ]] || {
  echo 'Existing deployment PostgreSQL container is not running.' >&2
  exit 1
}
[[ "$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{end}}' "$postgres_id" 2>/dev/null || true)" == healthy ]] || {
  echo 'Existing deployment PostgreSQL container is not healthy.' >&2
  exit 1
}

declare -A previous_blob_volumes=()
for service in "${services[@]}"; do
  id="$(latex_core_container_id "$service")"
  [[ -n "$id" ]] || { echo "Existing deployment service is missing: $service" >&2; exit 1; }
  [[ "$(docker inspect -f '{{.State.Status}}' "$id" 2>/dev/null || true)" == running ]] || {
    echo "Existing deployment service is not running: $service" >&2
    exit 1
  }
  previous_blob_volumes["$service"]="$(docker inspect -f '{{range .Mounts}}{{if eq .Destination "/var/lib/latex-core/blobs"}}{{.Name}}{{end}}{{end}}' "$id")"
  [[ -n "${previous_blob_volumes[$service]}" ]] || { echo "$service has no persistent blob volume at the expected mount." >&2; exit 1; }
done

phase=migration-compatibility
"$root/scripts/check-deployment-migrations.sh"

phase=image-build
echo "Building application image(s): ${services[*]}"
"${LATEX_CORE_COMPOSE[@]}" build "${services[@]}"

phase=service-replacement
replacement_started=true
echo "Replacing only application service(s): ${services[*]}"
"${LATEX_CORE_COMPOSE[@]}" up -d --no-deps "${services[@]}"

container_field() {
  local service="$1" format="$2" id
  id="$(latex_core_container_id "$service")"
  [[ -n "$id" ]] && docker inspect --format "$format" "$id" 2>/dev/null || true
}

wait_healthy() {
  local service="$1" limit="$2" elapsed=0 state health restarts initial_restarts=''
  while ((elapsed < limit)); do
    state="$(container_field "$service" '{{.State.Status}}')"
    health="$(container_field "$service" '{{if .State.Health}}{{.State.Health.Status}}{{end}}')"
    restarts="$(container_field "$service" '{{.RestartCount}}')"
    [[ -n "$initial_restarts" ]] || initial_restarts="${restarts:-0}"
    [[ "$state" == running && "$health" == healthy ]] && return 0
    if [[ "$state" =~ ^(exited|dead|restarting)$ || $(( ${restarts:-0} - initial_restarts )) -ge 2 ]]; then
      echo "$service failed readiness (state=${state:-missing}, health=${health:-none}, restarts=${restarts:-0})." >&2
      return 1
    fi
    sleep 2
    elapsed=$((elapsed + 2))
  done
  echo "$service did not become healthy within ${limit}s." >&2
  return 1
}

wait_running_stable() {
  local service="$1" limit="$2" elapsed=0 stable=0 state restarts previous=''
  while ((elapsed < limit)); do
    state="$(container_field "$service" '{{.State.Status}}')"
    restarts="$(container_field "$service" '{{.RestartCount}}')"
    if [[ "$state" == running ]]; then
      if [[ "$restarts" == "$previous" ]]; then stable=$((stable + 2)); else stable=0; fi
      previous="$restarts"
      ((stable >= 8)) && return 0
    elif [[ "$state" =~ ^(exited|dead|restarting)$ ]]; then
      echo "$service failed stability (state=${state:-missing}, restarts=${restarts:-0})." >&2
      return 1
    fi
    sleep 2
    elapsed=$((elapsed + 2))
  done
  echo "$service did not remain stable within ${limit}s." >&2
  return 1
}

phase=readiness
for service in "${services[@]}"; do
  if [[ "$service" == api ]]; then wait_healthy api 120; else wait_running_stable worker 90; fi
done
if [[ " ${services[*]} " == *' worker '* ]]; then
  "${LATEX_CORE_COMPOSE[@]}" exec -T worker /usr/local/bin/latex-core-doctor
fi

phase=preservation-check
[[ "$(latex_core_container_id postgres)" == "$postgres_id" ]] || {
  echo 'PostgreSQL container identity changed during an application-only update.' >&2
  exit 1
}
for service in "${services[@]}"; do
  id="$(latex_core_container_id "$service")"
  current_blob="$(docker inspect -f '{{range .Mounts}}{{if eq .Destination "/var/lib/latex-core/blobs"}}{{.Name}}{{end}}{{end}}' "$id")"
  [[ "$current_blob" == "${previous_blob_volumes[$service]}" ]] || {
    echo "$service blob volume identity changed unexpectedly." >&2
    exit 1
  }
done

phase=complete
trap - EXIT
echo "Update passed: ${services[*]} replaced; PostgreSQL container and persistent blob volume identities preserved."
