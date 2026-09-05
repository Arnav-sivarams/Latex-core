#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
expected='sha256:8db804f76b8e80e5be9fb28ba14b0938df5989b7a8250ca6b0e9f3c200c4ee38'
failures=0
pass() { printf 'PASS  %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1" >&2; failures=$((failures + 1)); }
if command -v docker >/dev/null 2>&1; then pass 'Docker CLI exists'; else fail 'Docker CLI is missing'; fi
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then pass 'Docker daemon is reachable'; else fail 'Docker daemon is not reachable'; fi
if docker compose version >/dev/null 2>&1; then pass 'Docker Compose v2 exists'; else fail 'Docker Compose v2 is missing'; fi
if [[ -f .env ]]; then pass '.env exists'; else fail '.env is missing'; fi
actual="$(docker image inspect latex-core-texlive:2026-m7 --format '{{.Id}}' 2>/dev/null || true)"
if [[ -n "$actual" ]]; then pass 'Frozen local M7 image exists'; else fail 'Frozen local M7 image is missing'; fi
if [[ "$actual" == "$expected" ]]; then pass 'Frozen local M7 image identity matches'; else fail 'Frozen local M7 image identity differs'; fi
project="${COMPOSE_PROJECT_NAME:-$(awk -F= '$1=="COMPOSE_PROJECT_NAME"{print substr($0,index($0,"=")+1)}' .env | tail -n1)}"
project="${project:-latex-core}"
compose=(docker compose --project-name "$project" --env-file "$root/.env" -f deploy/compose/docker-compose.yml)
for service in postgres api worker caddy; do
  id="$("${compose[@]}" ps -q "$service" 2>/dev/null || true)"
  state='missing'
  [[ -n "$id" ]] && state="$(docker inspect -f '{{.State.Status}}' "$id" 2>/dev/null || true)"
  if [[ "$state" == running ]]; then pass "$service service is running"; else fail "$service service is not running"; fi
done
migrations="$("${compose[@]}" exec -T postgres psql -X -U "${POSTGRES_USER:-latex_core}" -d "${POSTGRES_DB:-latex_core}" -Atc "SELECT count(*) FROM public._sqlx_migrations WHERE version BETWEEN 1 AND 24 AND success" 2>/dev/null || true)"
if [[ "$migrations" == 24 ]]; then pass 'Migrations 1-24 are successful'; else fail 'Migrations 1-24 are not all successful'; fi
if ./latex-core doctor >/dev/null 2>&1; then pass './latex-core doctor is healthy'; else fail './latex-core doctor failed'; fi
if ((failures)); then printf '%d install verification check(s) failed.\n' "$failures" >&2; exit 1; fi
echo 'All install verification checks passed.'
