#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=scripts/install-common.sh
source "$root/scripts/install-common.sh"
cd "$root"
expected_image='sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38'
failures=0
pass() { printf 'PASS  %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1" >&2; failures=$((failures + 1)); }

if "$root/scripts/check-install-host.sh" >/dev/null; then pass 'supported host contract'; else fail 'supported host contract'; fi
if latex_core_init "$root"; then pass 'deployment identity resolves from repository .env'; else fail 'deployment identity is invalid'; fi
if python3 "$root/scripts/validate-install-config.py" "$root/.env" >/dev/null; then pass 'configuration matches application parsers'; else fail 'configuration is invalid'; fi
if "${LATEX_CORE_COMPOSE[@]}" config --quiet; then pass 'Compose configuration resolves'; else fail 'Compose configuration is invalid'; fi
actual="$(docker image inspect latex-core-texlive:2026-m7 --format '{{.Id}} {{.Os}}/{{.Architecture}}' 2>/dev/null || true)"
if [[ "$actual" == "$expected_image linux/amd64" ]]; then pass 'frozen local M7 image identity/platform matches'; else fail 'frozen local M7 image identity/platform differs'; fi

for service in postgres api worker caddy; do
  id="$(latex_core_container_id "$service")"
  state=missing
  [[ -n "$id" ]] && state="$(docker inspect -f '{{.State.Status}}' "$id" 2>/dev/null || true)"
  if [[ "$state" == running ]]; then pass "$service service is running"; else fail "$service service is not running"; fi
done
postgres_id="$(latex_core_container_id postgres)"
api_id="$(latex_core_container_id api)"
if [[ -n "$postgres_id" && "$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{end}}' "$postgres_id" 2>/dev/null)" == healthy ]]; then pass 'PostgreSQL healthcheck is healthy'; else fail 'PostgreSQL healthcheck is not healthy'; fi
if [[ -n "$api_id" && "$(docker inspect -f '{{if .State.Health}}{{.State.Health.Status}}{{end}}' "$api_id" 2>/dev/null)" == healthy ]]; then pass 'API database-backed readiness is healthy'; else fail 'API database-backed readiness is not healthy'; fi

expected_versions="$(find migrations -maxdepth 1 -type f -name '[0-9][0-9][0-9][0-9]_*.sql' -printf '%f\n' | sort | sed -E 's/^0*([0-9]+)_.*/\1/')"
# These quoted variables intentionally expand inside the PostgreSQL container.
# shellcheck disable=SC2016
actual_versions="$("${LATEX_CORE_COMPOSE[@]}" exec -T postgres sh -ceu 'PGPASSWORD="$POSTGRES_PASSWORD" psql -X -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atc "SELECT version FROM public._sqlx_migrations WHERE success ORDER BY version"' 2>/dev/null || true)"
# shellcheck disable=SC2016
failed_migrations="$("${LATEX_CORE_COMPOSE[@]}" exec -T postgres sh -ceu 'PGPASSWORD="$POSTGRES_PASSWORD" psql -X -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB" -Atc "SELECT count(*) FROM public._sqlx_migrations WHERE NOT success"' 2>/dev/null || true)"
if [[ "$actual_versions" == "$expected_versions" && "$failed_migrations" == 0 ]]; then pass 'database migration version set matches this checkout'; else fail 'database migration version set does not match this checkout'; fi
if "$root/latex-core" doctor >/dev/null; then pass 'Worker database/storage/staging/Docker/compiler readiness'; else fail 'Worker readiness failed'; fi
http_port="$(latex_core_env_value HTTP_PORT "$root/.env")"
if curl --fail --silent --show-error --max-time 10 "http://127.0.0.1:$http_port/" >/dev/null; then pass 'proxy serves the application'; else fail 'proxy readiness failed'; fi

if ((failures)); then printf '%d install verification check(s) failed. Run ./scripts/diagnose-install.sh.\n' "$failures" >&2; exit 1; fi
echo 'All install verification checks passed.'
