#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"
latex_core_init "$root"
phase=preflight

failed() {
  local status=$?
  if ((status != 0)); then
    echo "Startup failed during phase: $phase (exit $status)." >&2
  fi
  exit "$status"
}
trap failed EXIT

container_field() {
  local service="$1" format="$2" id
  id="$(latex_core_container_id "$service")"
  if [[ -n "$id" ]]; then docker inspect --format "$format" "$id" 2>/dev/null || true; fi
}

wait_healthy() {
  local service="$1" limit="$2" elapsed=0 state health restarts initial_restarts=''
  while ((elapsed < limit)); do
    state="$(container_field "$service" '{{.State.Status}}')"
    health="$(container_field "$service" '{{if .State.Health}}{{.State.Health.Status}}{{end}}')"
    restarts="$(container_field "$service" '{{.RestartCount}}')"
    [[ -n "$initial_restarts" ]] || initial_restarts="${restarts:-0}"
    if [[ "$state" == running && "$health" == healthy ]]; then return 0; fi
    if [[ "$state" =~ ^(exited|dead)$ || "$state" == restarting || $(( ${restarts:-0} - initial_restarts )) -ge 2 ]]; then
      echo "$service failed before readiness (state=${state:-missing}, health=${health:-none}, restarts=${restarts:-0})." >&2
      return 1
    fi
    sleep 2
    elapsed=$((elapsed + 2))
  done
  echo "$service did not become healthy within ${limit}s." >&2
  return 1
}

wait_running_stable() {
  local service="$1" limit="$2" elapsed=0 stable=0 state restarts previous='' initial_restarts=''
  while ((elapsed < limit)); do
    state="$(container_field "$service" '{{.State.Status}}')"
    restarts="$(container_field "$service" '{{.RestartCount}}')"
    [[ -n "$initial_restarts" ]] || initial_restarts="${restarts:-0}"
    if [[ "$state" == running ]]; then
      if [[ "$restarts" == "$previous" ]]; then stable=$((stable + 2)); else stable=0; fi
      previous="$restarts"
      ((stable >= 8)) && return 0
    elif [[ "$state" =~ ^(exited|dead|restarting)$ || $(( ${restarts:-0} - initial_restarts )) -ge 2 ]]; then
      echo "$service failed during startup (state=${state:-missing}, restarts=${restarts:-0})." >&2
      return 1
    fi
    sleep 2
    elapsed=$((elapsed + 2))
  done
  echo "$service did not remain running for a stable interval within ${limit}s." >&2
  return 1
}

python3 "$root/scripts/validate-install-config.py" "$LATEX_CORE_ENV_FILE"
"${LATEX_CORE_COMPOSE[@]}" config --quiet
"$root/scripts/check-install-ports.sh"
staging="$(latex_core_env_value WORKER_STAGING_HOST_ROOT "$LATEX_CORE_ENV_FILE")"
mkdir -p "$staging"
[[ -d "$staging" && -w "$staging" ]] || { echo "Worker staging path is not writable by the installer account: $staging" >&2; exit 1; }
chmod 700 "$staging"
docker run --rm --network none --user 0:0 --entrypoint /bin/sh \
  --mount "type=bind,source=$staging,target=$staging" \
  sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38 \
  -c 'test -d "$1" && test -w "$1"' sh "$staging"

phase=image-acquisition
echo 'PHASE image acquisition: PostgreSQL and Caddy'
"${LATEX_CORE_COMPOSE[@]}" pull postgres caddy

phase=image-build
echo 'PHASE image build: API and Worker'
"${LATEX_CORE_COMPOSE[@]}" build api worker

phase=database-readiness
echo 'PHASE database readiness'
"${LATEX_CORE_COMPOSE[@]}" up -d postgres
wait_healthy postgres 150
# The quoted variables intentionally expand inside the PostgreSQL container.
# shellcheck disable=SC2016
"${LATEX_CORE_COMPOSE[@]}" exec -T postgres sh -ceu \
  'PGPASSWORD="$POSTGRES_PASSWORD" psql -X -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atc "SELECT 1" >/dev/null'

phase=schema-migration
echo 'PHASE schema initialization/migration'
"${LATEX_CORE_COMPOSE[@]}" run --rm --no-deps api /usr/local/bin/latex-core-admin database migrate

phase=api-readiness
echo 'PHASE API readiness'
"${LATEX_CORE_COMPOSE[@]}" up -d api
wait_healthy api 120

phase=worker-readiness
echo 'PHASE Worker readiness'
"${LATEX_CORE_COMPOSE[@]}" up -d worker
wait_running_stable worker 90
"${LATEX_CORE_COMPOSE[@]}" exec -T worker /usr/local/bin/latex-core-doctor

phase=proxy-readiness
echo 'PHASE proxy readiness'
"${LATEX_CORE_COMPOSE[@]}" up -d caddy
wait_running_stable caddy 60
http_port="$(latex_core_env_value HTTP_PORT "$LATEX_CORE_ENV_FILE")"
curl --fail --silent --show-error --max-time 10 "http://127.0.0.1:$http_port/" >/dev/null

phase=complete
trap - EXIT
echo 'All service readiness phases passed.'
